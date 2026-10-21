use std::path::Path;
use std::sync::Arc;

use glam::{DMat4, DQuat, DVec3};
use pfx_core::sim::{Rng, mix64};
use pfx_recipe::{Recipe, Scatter};

use crate::{Error, Part, Scattered, local_matrix, poisson, rotation, world};

pub const MAX_COPIES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub at: DVec3,
    pub normal: DVec3,
}

pub struct Ground {
    triangles: Vec<[DVec3; 3]>,
    low: [f64; 2],
    size: [f64; 2],
    cell: f64,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

impl Ground {
    pub fn new(triangles: Vec<[DVec3; 3]>) -> Option<Ground> {
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for p in triangles.iter().flatten() {
            low = [low[0].min(p.x), low[1].min(p.z)];
            high = [high[0].max(p.x), high[1].max(p.z)];
        }
        let size = [high[0] - low[0], high[1] - low[1]];
        if !(size[0] > 0.0 && size[1] > 0.0 && size[0].is_finite() && size[1].is_finite()) {
            return None;
        }
        let cell = (size[0] * size[1] / triangles.len().max(1) as f64)
            .sqrt()
            .max(size[0].max(size[1]) / 512.0);
        let columns = ((size[0] / cell).ceil() as usize).max(1);
        let rows = ((size[1] / cell).ceil() as usize).max(1);
        let mut ground = Ground {
            triangles,
            low,
            size,
            cell,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
        };
        for (at, t) in ground.triangles.iter().enumerate() {
            let (c0, r0) = ground.slot(
                t[0].x.min(t[1].x).min(t[2].x),
                t[0].z.min(t[1].z).min(t[2].z),
            );
            let (c1, r1) = ground.slot(
                t[0].x.max(t[1].x).max(t[2].x),
                t[0].z.max(t[1].z).max(t[2].z),
            );
            for r in r0..=r1 {
                for c in c0..=c1 {
                    ground.cells[r * columns + c].push(at as u32);
                }
            }
        }
        Some(ground)
    }

    pub fn low(&self) -> [f64; 2] {
        self.low
    }

    pub fn size(&self) -> [f64; 2] {
        self.size
    }

    fn slot(&self, x: f64, z: f64) -> (usize, usize) {
        let c = ((x - self.low[0]) / self.cell).max(0.0) as usize;
        let r = ((z - self.low[1]) / self.cell).max(0.0) as usize;
        (c.min(self.columns - 1), r.min(self.rows - 1))
    }

    pub fn below(&self, x: f64, z: f64) -> Option<Hit> {
        self.under(x, z, f64::INFINITY)
    }

    pub fn under(&self, x: f64, z: f64, ceiling: f64) -> Option<Hit> {
        let (c, r) = self.slot(x, z);
        let mut best: Option<Hit> = None;
        for &at in &self.cells[r * self.columns + c] {
            let [a, b, d] = self.triangles[at as usize];
            let area = (b.x - a.x) * (d.z - a.z) - (d.x - a.x) * (b.z - a.z);
            if area.abs() < 1e-12 {
                continue;
            }
            let u = ((b.x - x) * (d.z - z) - (d.x - x) * (b.z - z)) / area;
            let v = ((d.x - x) * (a.z - z) - (a.x - x) * (d.z - z)) / area;
            let w = 1.0 - u - v;
            if u < 0.0 || v < 0.0 || w < 0.0 {
                continue;
            }
            let y = u * a.y + v * b.y + w * d.y;
            if y > ceiling || best.is_some_and(|hit| hit.at.y >= y) {
                continue;
            }
            let mut normal = (b - a).cross(d - a).normalize_or(DVec3::Y);
            if normal.y < 0.0 {
                normal = -normal;
            }
            best = Some(Hit {
                at: DVec3::new(x, y, z),
                normal,
            });
        }
        best
    }
}

pub struct Map {
    width: usize,
    height: usize,
    grey: Vec<f64>,
}

impl Map {
    pub fn decode(bytes: &[u8]) -> Result<Map, String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut buffer = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or("the image is too large")?
        ];
        let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
        let channels = frame.color_type.samples();
        let grey = buffer[..frame.buffer_size()]
            .chunks_exact(channels)
            .map(|px| {
                let level = match channels {
                    1 | 2 => f64::from(px[0]),
                    _ => {
                        0.2126 * f64::from(px[0])
                            + 0.7152 * f64::from(px[1])
                            + 0.0722 * f64::from(px[2])
                    }
                };
                level / 255.0
            })
            .collect();
        Ok(Map {
            width: frame.width as usize,
            height: frame.height as usize,
            grey,
        })
    }

    pub fn at(&self, u: f64, v: f64) -> f64 {
        let x = ((u * self.width as f64) as usize).min(self.width - 1);
        let y = ((v * self.height as f64) as usize).min(self.height - 1);
        self.grey[y * self.width + x]
    }
}

pub fn seed_of(recipe: &Recipe, scatter: &Scatter) -> u64 {
    scatter.seed.unwrap_or_else(|| {
        scatter.name.bytes().fold(mix64(recipe.seed), |hash, byte| {
            mix64(hash ^ u64::from(byte))
        })
    })
}

fn lerp(rng: &mut Rng, low: f64, high: f64) -> f64 {
    low + (high - low) * rng.next_f64()
}

pub fn place(
    scatter: &Scatter,
    seed: u64,
    ground: &Ground,
    map: Option<&Map>,
) -> Result<Vec<DMat4>, String> {
    let [w, d] = ground.size();
    let [x0, z0] = ground.low();
    let mut rng = Rng::new(seed);
    let points = match scatter.spacing {
        Some(spacing) => {
            if (w / spacing) * (d / spacing) > (MAX_COPIES * 4) as f64 {
                return Err(too_many((w / spacing * d / spacing) as usize));
            }
            poisson::poisson([w, d], spacing, seed)
        }
        None => {
            let wanted = (scatter.density.unwrap_or(0.0) * w * d).round();
            if wanted > MAX_COPIES as f64 {
                return Err(too_many(wanted as usize));
            }
            (0..wanted as usize)
                .map(|_| [rng.next_f64() * w, rng.next_f64() * d])
                .collect()
        }
    };
    let thin = match (scatter.spacing, scatter.density) {
        (Some(_), Some(density)) if !points.is_empty() => {
            (density * w * d / points.len() as f64).min(1.0)
        }
        _ => 1.0,
    };
    let mut draws = rng.fork(1);
    let mut placed = Vec::new();
    for [px, pz] in points {
        let keep = draws.next_f64();
        let turn =
            std::array::from_fn(|k| lerp(&mut draws, scatter.rotate_min[k], scatter.rotate_max[k]));
        let scale = lerp(&mut draws, scatter.scale_min, scatter.scale_max);
        let chance = thin * map.map_or(1.0, |map| map.at(px / w, pz / d));
        if keep >= chance {
            continue;
        }
        let Some(hit) = ground.below(x0 + px, z0 + pz) else {
            continue;
        };
        let align = if scatter.align {
            DQuat::from_rotation_arc(DVec3::Y, hit.normal)
        } else {
            DQuat::IDENTITY
        };
        placed.push(
            DMat4::from_translation(hit.at)
                * DMat4::from_quat(align)
                * rotation(turn)
                * DMat4::from_scale(DVec3::splat(scale)),
        );
        if placed.len() > MAX_COPIES {
            return Err(too_many(placed.len()));
        }
    }
    Ok(placed)
}

fn too_many(count: usize) -> String {
    format!(
        "makes {count} copies or more, more than {MAX_COPIES}; expected a larger `spacing` or a lower `density`"
    )
}

pub fn triangles(parts: &[Part], object: usize) -> Vec<[DVec3; 3]> {
    let mut out = Vec::new();
    for part in parts.iter().filter(|part| part.object == object) {
        let node = &part.node;
        let matrix = if node.keep_transform {
            node.transform
        } else {
            DMat4::IDENTITY
        };
        let point = |i: u32| {
            let p = node.mesh.positions[i as usize];
            matrix.transform_point3(DVec3::new(
                f64::from(p[0]),
                f64::from(p[1]),
                f64::from(p[2]),
            ))
        };
        for tri in node.mesh.indices.chunks_exact(3) {
            out.push([point(tri[0]), point(tri[1]), point(tri[2])]);
        }
    }
    out
}

pub(crate) fn scatter(recipe: &Recipe, base: &Path, parts: &mut Vec<Part>) -> Result<usize, Error> {
    let first = parts.len();
    for (number, scatter) in recipe.scatters.iter().enumerate() {
        let at = |key: &str| format!("scatter[{}].{key}", scatter.name);
        let index = |name: &str, key: &str| {
            recipe
                .objects
                .iter()
                .rposition(|o| o.name == name)
                .ok_or_else(|| Error::new(at(key), format!("no object is named `{name}`")))
        };
        let template = index(&scatter.object, "object")?;
        let target = index(&scatter.on, "on")?;
        let ground = Ground::new(triangles(&parts[..first], target)).ok_or_else(|| {
            Error::new(
                at("on"),
                format!("`{}` has no surface seen from above", scatter.on),
            )
        })?;
        let map = match &scatter.map {
            Some(path) => {
                let bytes = pfx_recipe::files::read(&base.join(path))
                    .map_err(|e| Error::new(at("map"), format!("cannot read {path}: {e}")))?;
                Some(
                    Map::decode(&bytes)
                        .map_err(|e| Error::new(at("map"), format!("{path}: {e}")))?,
                )
            }
            None => None,
        };
        let placed = place(scatter, seed_of(recipe, scatter), &ground, map.as_ref())
            .map_err(|text| Error::new(at("spacing"), text))?;
        let object = &recipe.objects[template];
        let own = local_matrix(&pfx_recipe::Object {
            at: [0.0; 3],
            ..object.clone()
        });
        let strip = own * world(recipe, template)?.inverse();
        let hidden = recipe.collection_hides(&scatter.name);
        let shown: Vec<&Part> = parts[..first]
            .iter()
            .filter(|part| part.object == template)
            .collect();
        let pieces = shown.len();
        let mut copies = Vec::with_capacity(placed.len() * pieces);
        for (n, matrix) in placed.iter().enumerate() {
            for (piece, part) in shown.iter().enumerate() {
                let mut node = part.node.clone();
                let placement = if node.keep_transform {
                    node.transform
                } else {
                    DMat4::IDENTITY
                };
                node.transform = *matrix * strip * placement;
                node.keep_transform = true;
                node.targets = None;
                node.name = if pieces == 1 {
                    format!("{}.{n}", scatter.name)
                } else {
                    format!("{}.{n}.{}", scatter.name, part.node.name)
                };
                copies.push(Part {
                    node,
                    object: template,
                    shadow: part.shadow,
                    two_sided: part.two_sided,
                    hidden,
                    looks: Arc::clone(&part.looks),
                    scattered: Some(Scattered {
                        scatter: number as u32,
                        piece: piece as u32,
                        copy: n as u32,
                    }),
                });
            }
        }
        parts.extend(copies);
    }
    Ok(parts.len() - first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slope() -> Ground {
        let a = DVec3::new(-2.0, 0.0, -2.0);
        let b = DVec3::new(2.0, 0.0, -2.0);
        let c = DVec3::new(2.0, 2.0, 2.0);
        let d = DVec3::new(-2.0, 2.0, 2.0);
        Ground::new(vec![[a, b, c], [a, c, d]]).unwrap()
    }

    fn scatter() -> Scatter {
        Scatter {
            name: "stones".into(),
            object: "stone".into(),
            on: "ground".into(),
            seed: Some(11),
            spacing: Some(0.5),
            density: None,
            map: None,
            rotate_min: [0.0; 3],
            rotate_max: [0.0, 360.0, 0.0],
            scale_min: 0.5,
            scale_max: 1.5,
            align: false,
        }
    }

    #[test]
    fn a_copy_lands_on_the_highest_surface_below_it() {
        let ground = slope();
        let hit = ground.below(0.0, 1.0).unwrap();
        assert!((hit.at.y - 1.5).abs() < 1e-12);
        assert!(hit.normal.y > 0.0 && hit.normal.z < 0.0);
        assert!(ground.below(3.0, 0.0).is_none());
    }

    #[test]
    fn copies_keep_their_spacing_stay_on_the_surface_and_repeat_with_the_seed() {
        let ground = slope();
        let placed = place(&scatter(), 11, &ground, None).unwrap();
        assert!(placed.len() > 40, "{}", placed.len());
        assert_eq!(placed, place(&scatter(), 11, &ground, None).unwrap());
        assert_ne!(placed, place(&scatter(), 12, &ground, None).unwrap());
        for (i, m) in placed.iter().enumerate() {
            let p = m.w_axis.truncate();
            assert!((p.y - (p.z + 2.0) / 2.0).abs() < 1e-9);
            let scale = m.x_axis.truncate().length();
            assert!((0.5..=1.5).contains(&scale));
            for other in &placed[i + 1..] {
                let q = other.w_axis.truncate();
                assert!(DVec3::new(p.x - q.x, 0.0, p.z - q.z).length() >= 0.5);
            }
        }
    }

    #[test]
    fn density_sets_the_count_and_align_follows_the_normal() {
        let ground = slope();
        let random = Scatter {
            spacing: None,
            density: Some(2.0),
            align: true,
            rotate_max: [0.0; 3],
            scale_min: 1.0,
            scale_max: 1.0,
            ..scatter()
        };
        let placed = place(&random, 3, &ground, None).unwrap();
        assert_eq!(placed.len(), 32);
        let up = placed[0].transform_vector3(DVec3::Y);
        let normal = DVec3::new(0.0, 2.0, -1.0).normalize();
        assert!((up - normal).length() < 1e-9);
        let thinned = Scatter {
            density: Some(1.0),
            ..scatter()
        };
        let full = place(&scatter(), 11, &ground, None).unwrap().len();
        let thin = place(&thinned, 11, &ground, None).unwrap().len();
        assert!(thin < full && (10..=24).contains(&thin), "{thin} of {full}");
    }

    #[test]
    fn a_black_map_places_nothing_and_a_white_one_everything() {
        let ground = slope();
        let flat = |level: f64| Map {
            width: 2,
            height: 2,
            grey: vec![level; 4],
        };
        let all = place(&scatter(), 11, &ground, None).unwrap();
        assert_eq!(
            place(&scatter(), 11, &ground, Some(&flat(1.0))).unwrap(),
            all
        );
        assert!(
            place(&scatter(), 11, &ground, Some(&flat(0.0)))
                .unwrap()
                .is_empty()
        );
        let half = Map {
            width: 2,
            height: 1,
            grey: vec![1.0, 0.0],
        };
        let left = place(&scatter(), 11, &ground, Some(&half)).unwrap();
        assert!(!left.is_empty() && left.iter().all(|m| m.w_axis.x < 0.0));
    }

    #[test]
    fn too_dense_a_scatter_is_refused() {
        let dense = Scatter {
            spacing: None,
            density: Some(1.0e5),
            ..scatter()
        };
        let error = place(&dense, 1, &slope(), None).unwrap_err();
        assert!(error.contains("more than 100000"), "{error}");
    }

    const RECIPE: &str = r#"recipe = 1
name = "field"
seed = 4

[[object]]
name = "ground"
kind = "plane"
size = [6.0, 4.0]

[[object]]
name = "stone"
kind = "box"
size = [0.2, 0.1, 0.2]
rotate = [0.0, 0.0, 90.0]
hidden = true

[[scatter]]
name = "stones"
object = "stone"
on = "ground"
spacing = 0.6
rotate_max = [0.0, 360.0, 0.0]

[[collection]]
name = "rocks"
objects = ["stones"]

[[camera]]
name = "main"
position = [0.0, 5.0, 5.0]
target = [0.0, 0.0, 0.0]
fov_y_deg = 40.0
"#;

    #[test]
    fn a_recipe_scatters_visible_copies_of_a_hidden_object() {
        let recipe = pfx_recipe::parse(RECIPE).unwrap();
        assert!(pfx_recipe::validate(&recipe).is_empty());
        let built = crate::build(&recipe, Path::new("")).unwrap();
        let copies: Vec<&Part> = built
            .parts
            .iter()
            .filter(|p| p.node.name.starts_with("stones."))
            .collect();
        assert!(copies.len() > 20, "{}", copies.len());
        assert!(copies.iter().all(|p| !p.hidden && p.node.keep_transform));
        let stone = built.parts.iter().find(|p| p.node.name == "stone").unwrap();
        assert!(stone.hidden);
        assert!(copies.iter().all(|p| p.node.mesh == stone.node.mesh));
        let node = &copies[0].node;
        let heights: Vec<f64> = node
            .mesh
            .positions
            .iter()
            .map(|p| {
                node.transform
                    .transform_point3(DVec3::new(
                        f64::from(p[0]),
                        f64::from(p[1]),
                        f64::from(p[2]),
                    ))
                    .y
            })
            .collect();
        let low = heights.iter().copied().fold(f64::INFINITY, f64::min);
        let high = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (high - low - 0.2).abs() < 1e-6,
            "the stone's own turn stays: {}",
            high - low
        );
        assert!((low + 0.1).abs() < 1e-6, "{low}");
        let again = crate::build(&recipe, Path::new("")).unwrap();
        assert_eq!(again.glb(), built.glb());
        let hidden = pfx_recipe::parse(&RECIPE.replace(
            "objects = [\"stones\"]",
            "objects = [\"stones\"]\nhidden = true",
        ))
        .unwrap();
        let built = crate::build(&hidden, Path::new("")).unwrap();
        assert!(
            built
                .parts
                .iter()
                .filter(|p| p.node.name.starts_with("stones."))
                .all(|p| p.hidden)
        );
    }
}
