use pfx_geom::mark::{Form, Mark, Profile};
use pfx_geom::mesh::Mesh;
use pfx_geom::svg::{Gradient, Paint};
use pfx_materials::Material;
use serde_json::Value;

use crate::look;

pub type M4 = [[f64; 4]; 4];

pub const IDENTITY: M4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub const AXES: M4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub const RAMP: u32 = 256;
pub const TILE_HALF: f64 = 1.2;
pub const CATCHER_DROP: f64 = 0.001;

pub fn mul(a: &M4, b: &M4) -> M4 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..4).map(|k| a[r][k] * b[k][c]).sum()))
}

pub fn apply(m: &M4, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3])
}

pub fn translation(t: [f64; 3]) -> M4 {
    let mut m = IDENTITY;
    for (r, v) in t.into_iter().enumerate() {
        m[r][3] = v;
    }
    m
}

pub fn scaling(k: f64) -> M4 {
    let mut m = IDENTITY;
    for (r, row) in m.iter_mut().enumerate().take(3) {
        row[r] = k;
    }
    m
}

pub fn about_z(deg: f64) -> M4 {
    let (s, c) = deg.to_radians().sin_cos();
    let mut m = IDENTITY;
    m[0][0] = c;
    m[0][1] = -s;
    m[1][0] = s;
    m[1][1] = c;
    m
}

pub fn about_x(deg: f64) -> M4 {
    let (s, c) = deg.to_radians().sin_cos();
    let mut m = IDENTITY;
    m[1][1] = c;
    m[1][2] = -s;
    m[2][1] = s;
    m[2][2] = c;
    m
}

pub fn column_major(m: &M4) -> [[f32; 4]; 4] {
    std::array::from_fn(|c| std::array::from_fn(|r| m[r][c] as f32))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box3 {
    pub lo: [f64; 3],
    pub hi: [f64; 3],
}

impl Box3 {
    pub fn empty() -> Self {
        Self {
            lo: [f64::INFINITY; 3],
            hi: [f64::NEG_INFINITY; 3],
        }
    }

    pub fn of(points: &[[f64; 3]]) -> Self {
        let mut b = Self::empty();
        for p in points {
            b.add(*p);
        }
        b
    }

    pub fn add(&mut self, p: [f64; 3]) {
        self.lo = std::array::from_fn(|k| self.lo[k].min(p[k]));
        self.hi = std::array::from_fn(|k| self.hi[k].max(p[k]));
    }

    pub fn corners(&self) -> Vec<[f64; 3]> {
        let mut out = Vec::with_capacity(8);
        for x in [self.lo[0], self.hi[0]] {
            for y in [self.lo[1], self.hi[1]] {
                for z in [self.lo[2], self.hi[2]] {
                    out.push([x, y, z]);
                }
            }
        }
        out
    }
}

fn mesh_corners(mesh: &Mesh, m: &M4) -> Vec<[f64; 3]> {
    let b = &mesh.bounds;
    Box3 {
        lo: b.min.map(f64::from),
        hi: b.max.map(f64::from),
    }
    .corners()
    .into_iter()
    .map(|p| apply(m, p))
    .collect()
}

pub struct Solid {
    pub mesh: usize,
    pub material: usize,
    pub ramp: Option<usize>,
    pub frame: M4,
    pub mark: bool,
}

pub struct Placed {
    pub mesh: usize,
    pub model: M4,
    pub material: usize,
    pub ramp: Option<usize>,
}

#[derive(Default)]
pub struct World {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub ramps: Vec<Vec<[u8; 4]>>,
    pub placed: Vec<Placed>,
    pub corners: Vec<[f64; 3]>,
    pub outlined: Vec<u32>,
    pub catcher: Option<u32>,
    pub ignored: Vec<String>,
    pub triangles: usize,
}

impl World {
    fn mesh(&mut self, mesh: Mesh) -> usize {
        self.triangles += mesh.triangle_count();
        self.meshes.push(mesh);
        self.meshes.len() - 1
    }

    fn material(&mut self, material: Material) -> usize {
        self.materials.push(material);
        self.materials.len() - 1
    }
}

fn ramp(gradient: &Gradient, span: [f32; 2], view: [f32; 4]) -> Vec<[u8; 4]> {
    let width = f64::from(view[2] / view[2].max(view[3]));
    let (s0, s1) = (f64::from(span[0]), f64::from(span[1]));
    let stops: Vec<(f64, [f32; 3])> = gradient
        .stops
        .iter()
        .map(|(at, rgb)| (f64::from(*at), look::bytes_linear(*rgb)))
        .collect();
    (0..RAMP)
        .map(|i| {
            let u = (f64::from(i) + 0.5) / f64::from(RAMP);
            let x = u * width - width * 0.5;
            let fac = ((x - s0) / (s1 - s0).abs().max(1e-6).copysign(s1 - s0)).clamp(0.0, 1.0);
            let colour = ramp_at(&stops, fac);
            let mut out = [255u8; 4];
            for c in 0..3 {
                out[c] =
                    (pfx_post::view::srgb_encode(colour[c].clamp(0.0, 1.0)) * 255.0).round() as u8;
            }
            out
        })
        .collect()
}

fn ramp_at(stops: &[(f64, [f32; 3])], fac: f64) -> [f32; 3] {
    let Some(first) = stops.first() else {
        return [0.5; 3];
    };
    if fac <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if fac <= b.0 {
            let t = if b.0 > a.0 {
                ((fac - a.0) / (b.0 - a.0)) as f32
            } else {
                1.0
            };
            return std::array::from_fn(|c| a.1[c] + (b.1[c] - a.1[c]) * t);
        }
    }
    stops[stops.len() - 1].1
}

pub struct Built {
    pub solids: Vec<Solid>,
}

impl Built {
    pub fn corners(&self, world: &World) -> Vec<[f64; 3]> {
        self.solids
            .iter()
            .flat_map(|s| mesh_corners(&world.meshes[s.mesh], &s.frame))
            .collect()
    }
}

fn frame(world: &World, solids: &[Solid], height: f64) -> M4 {
    let corners: Vec<[f64; 3]> = solids
        .iter()
        .flat_map(|s| mesh_corners(&world.meshes[s.mesh], &s.frame))
        .collect();
    let b = Box3::of(&corners);
    let size = (b.hi[0] - b.lo[0]).max(b.hi[1] - b.lo[1]).max(1e-6);
    let k = height / size;
    let centre = [
        (b.lo[0] + b.hi[0]) * 0.5,
        (b.lo[1] + b.hi[1]) * 0.5,
        b.lo[2],
    ];
    mul(&scaling(k), &translation(centre.map(|v| -v)))
}

fn mark_solids(
    world: &mut World,
    svg: &str,
    preset: &Value,
) -> Result<(Vec<Solid>, Option<Paint>), String> {
    let form = look::form(preset)?;
    let mark = Mark::build(svg, &form)?;
    let master = preset
        .get("colors")
        .and_then(Value::as_str)
        .unwrap_or("master")
        == "master";
    let mut solids = Vec::new();
    for solid in mark.solids {
        let own = solid.material.clone();
        let (tint, gradient) = match (&own, master, &solid.paint) {
            (None, true, Paint::Color(rgb)) => (Some(look::bytes_linear(*rgb)), None),
            (None, true, Paint::Gradient(g)) => (None, solid.span.map(|span| (g.clone(), span))),
            _ => (None, None),
        };
        let chosen = look::look_of(preset, own.as_deref())?;
        let material = look::material(&chosen, tint, &mut world.ignored)?;
        let material = world.material(material);
        let ramp = gradient.map(|(g, span)| {
            world.ramps.push(ramp(&g, span, mark.view));
            world.ramps.len() - 1
        });
        let mesh = world.mesh(solid.mesh);
        solids.push(Solid {
            mesh,
            material,
            ramp,
            frame: IDENTITY,
            mark: true,
        });
    }
    if solids.is_empty() {
        return Err("the master draws no solid".into());
    }
    Ok((solids, mark.ground))
}

fn icon_solids(world: &mut World, spec: &Value, preset: &Value) -> Result<Vec<Solid>, String> {
    let icon = look::icon(spec)?;
    let mut solids = Vec::new();
    for piece in icon.build()? {
        let chosen = look::look_of(preset, piece.material.as_deref())?;
        let material = look::material(&chosen, None, &mut world.ignored)?;
        let material = world.material(material);
        let mesh = world.mesh(piece.mesh);
        solids.push(Solid {
            mesh,
            material,
            ramp: None,
            frame: IDENTITY,
            mark: true,
        });
    }
    if solids.is_empty() {
        let name = spec.get("name").and_then(Value::as_str).unwrap_or("?");
        return Err(format!("icon {name} has no parts"));
    }
    Ok(solids)
}

pub fn slab(radius: f64, thickness: f64) -> Result<Mesh, String> {
    let unit = 1000.0;
    let side = 2.0 * TILE_HALF * unit;
    let bevel = 0.35 * thickness;
    let corner = (2.0 * TILE_HALF * radius - bevel).max(0.0);
    let b = bevel * unit;
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} {tall}"><rect x="{b}" y="{b}" width="{w}" height="{w}" rx="{rx}" fill="#808080"/></svg>"##,
        tall = side + 1.0,
        w = side - 2.0 * b,
        rx = corner * unit,
    );
    let longest = (side + 1.0) / unit;
    let form = Form {
        depth: (0.15 * thickness / longest) as f32,
        bevel: (bevel / longest) as f32,
        segments: 8,
        profile: Profile::Round,
        layer: 0.0,
        ..Form::default()
    };
    let mark = Mark::build(&svg, &form)?;
    let mut mesh = mark
        .solids
        .into_iter()
        .next()
        .ok_or("the tile's slab drew nothing")?
        .mesh;
    let b0 = mesh.bounds;
    let centre = [
        (b0.min[0] + b0.max[0]) * 0.5,
        (b0.min[1] + b0.max[1]) * 0.5,
        b0.max[2],
    ];
    let k = longest as f32;
    for p in &mut mesh.positions {
        *p = std::array::from_fn(|i| (p[i] - centre[i]) * k);
    }
    Ok(Mesh::new(
        mesh.positions,
        mesh.normals,
        Vec::new(),
        mesh.uvs,
        mesh.indices,
    ))
}

pub fn build_asset(world: &mut World, asset: &Value) -> Result<Built, String> {
    let preset = asset.get("preset").ok_or("an asset has no preset")?;
    look::check_preset(preset)?;
    look::style_notes(preset, &mut world.ignored)?;
    let kind = asset.get("kind").and_then(Value::as_str).unwrap_or("logo");
    let ground = look::ground(preset)?;
    let (mut solids, paint) = if kind == "icons" {
        let spec = asset.get("icon").ok_or("an icon asset has no spec")?;
        (icon_solids(world, spec, preset)?, None)
    } else {
        let path = asset
            .get("svg")
            .and_then(Value::as_str)
            .ok_or("a mark asset has no master")?;
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        mark_solids(world, &text, preset)?
    };
    let height = if kind == "tile" {
        2.0 * TILE_HALF * f64::from(ground.mark)
    } else {
        2.0
    };
    let fit = frame(world, &solids, height);
    let lifted = if kind == "tile" {
        mul(&translation([0.0, 0.0, f64::from(ground.lift)]), &fit)
    } else {
        fit
    };
    for s in &mut solids {
        s.frame = mul(&lifted, &s.frame);
    }
    if kind == "tile" {
        let colour = match (ground.color, paint) {
            (Some(c), _) => c,
            (None, Some(Paint::Color(rgb))) => look::bytes_linear(rgb),
            _ => look::hex_linear("#24282C")?,
        };
        let material = world.material(ground.material(colour));
        let mesh = world.mesh(slab(f64::from(ground.radius), f64::from(ground.thickness))?);
        solids.push(Solid {
            mesh,
            material,
            ramp: None,
            frame: IDENTITY,
            mark: false,
        });
    }
    Ok(Built { solids })
}

pub struct Copy {
    pub at: [f64; 3],
    pub turn: f64,
    pub tilt: f64,
    pub scale: f64,
}

fn copies(asset: &Value) -> Result<Vec<Copy>, String> {
    let list = asset
        .get("copies")
        .and_then(Value::as_array)
        .ok_or("an asset lists its copies")?;
    list.iter()
        .map(|c| {
            let at: Vec<f64> = c
                .get("at")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            let at = match at[..] {
                [x, y, z] => [x, y, z],
                [x, y] => [x, y, 0.0],
                _ => [0.0; 3],
            };
            let get = |k: &str, d: f64| c.get(k).and_then(Value::as_f64).unwrap_or(d);
            Ok(Copy {
                at,
                turn: get("turn", 0.0),
                tilt: get("tilt", 0.0),
                scale: get("scale", 1.0),
            })
        })
        .collect()
}

pub fn place(world: &mut World, job: &Value) -> Result<(), String> {
    let assets = job
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("a job lists its assets")?;
    let single = job.get("single").and_then(Value::as_bool).unwrap_or(false);
    let mut built = Vec::new();
    for asset in assets {
        built.push(build_asset(world, asset)?);
    }
    let first = built.first().ok_or("a job needs an asset")?;
    if single {
        world.corners = first.corners(world);
        let solids: Vec<(usize, M4, usize, Option<usize>, bool)> = first
            .solids
            .iter()
            .map(|s| (s.mesh, s.frame, s.material, s.ramp, s.mark))
            .collect();
        for (mesh, frame, material, ramp, mark) in solids {
            world.placed.push(Placed {
                mesh,
                model: mul(&AXES, &frame),
                material,
                ramp,
            });
            if mark {
                world.outlined.push(material as u32);
            }
        }
        return Ok(());
    }
    let locals: Vec<Box3> = built.iter().map(|b| Box3::of(&b.corners(world))).collect();
    let group = |a: &Value| a.get("group").and_then(Value::as_u64).unwrap_or(0);
    let mut units: std::collections::BTreeMap<u64, f64> = std::collections::BTreeMap::new();
    for (asset, local) in assets.iter().zip(&locals) {
        let unit = units.entry(group(asset)).or_insert(0.0);
        *unit = unit
            .max(local.hi[0] - local.lo[0])
            .max(local.hi[1] - local.lo[1]);
    }
    let mut corners = Vec::new();
    let mut placed = Vec::new();
    for ((asset, b), local) in assets.iter().zip(&built).zip(&locals) {
        let unit = units[&group(asset)];
        let offset: Vec<f64> = asset
            .get("offset")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        let offset = match offset[..] {
            [x, y, z] => [x, y, z],
            _ => [0.0; 3],
        };
        for copy in copies(asset)? {
            let at = std::array::from_fn(|k| offset[k] + copy.at[k] * unit);
            let mut m = mul(
                &mul(&translation(at), &about_z(copy.turn)),
                &mul(&about_x(copy.tilt), &scaling(copy.scale)),
            );
            let pts: Vec<[f64; 3]> = local.corners().into_iter().map(|p| apply(&m, p)).collect();
            let drop = pts.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min);
            if drop < 0.0 {
                m = mul(&translation([0.0, 0.0, -drop]), &m);
            }
            corners.extend(local.corners().into_iter().map(|p| apply(&m, p)));
            for s in &b.solids {
                placed.push(Placed {
                    mesh: s.mesh,
                    model: mul(&AXES, &mul(&m, &s.frame)),
                    material: s.material,
                    ramp: s.ramp,
                });
            }
        }
    }
    world.placed = placed;
    world.corners = corners;
    world.outlined = (0..world.materials.len() as u32).collect();
    Ok(())
}

pub fn catcher(world: &mut World) {
    let b = Box3::of(&world.corners);
    let span = (b.hi[0] - b.lo[0]).max(b.hi[1] - b.lo[1]).max(2.0);
    let half = (span * 3.0) as f32;
    let positions = vec![
        [-half, -half, 0.0],
        [half, -half, 0.0],
        [half, half, 0.0],
        [-half, half, 0.0],
    ];
    let uvs = vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let mesh = Mesh::new(
        positions,
        vec![[0.0, 0.0, 1.0]; 4],
        Vec::new(),
        uvs,
        vec![0, 1, 2, 0, 2, 3],
    );
    let material = world.material(Material {
        base: [0.8; 3],
        roughness: 0.5,
        ..Material::default()
    });
    let mesh = world.mesh(mesh);
    let centre = [
        (b.lo[0] + b.hi[0]) * 0.5,
        (b.lo[1] + b.hi[1]) * 0.5,
        -CATCHER_DROP,
    ];
    world.placed.push(Placed {
        mesh,
        model: mul(&AXES, &translation(centre)),
        material,
        ramp: None,
    });
    world.outlined.retain(|m| *m != material as u32);
    world.catcher = Some(material as u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn marks() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/marks")
    }

    fn preset() -> Value {
        let text = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/recipes/presets/sample.toml"),
        )
        .unwrap();
        serde_json::to_value(text.parse::<toml::Table>().unwrap()).unwrap()
    }

    fn logo(kind: &str) -> Value {
        json!({
            "kind": kind,
            "preset": preset(),
            "svg": marks().join("logos/sample.svg"),
            "group": 0,
            "offset": [0.0, 0.0, 0.0],
            "copies": [{"at": [0.0, 0.0, 0.0], "turn": 0.0, "tilt": 0.0, "scale": 1.0}],
        })
    }

    #[test]
    fn a_mark_is_framed_two_units_across_resting_on_the_floor() {
        let mut world = World::default();
        place(
            &mut world,
            &json!({"single": true, "assets": [logo("logo")]}),
        )
        .unwrap();
        let b = Box3::of(&world.corners);
        assert!((b.hi[0] - b.lo[0] - 2.0).abs() < 1e-3, "{b:?}");
        assert!((b.hi[1] - b.lo[1] - 2.0).abs() < 1e-3, "{b:?}");
        assert!(b.lo[2].abs() < 1e-6 && b.hi[2] > 0.1, "{b:?}");
        assert!((b.lo[0] + b.hi[0]).abs() < 1e-6);
        assert_eq!(world.placed.len(), 2);
        assert_eq!(world.outlined, [0, 1]);
        let grey = look::bytes_linear([0x80; 3]);
        let dark = look::bytes_linear([0x40; 3]);
        assert_eq!(world.materials[0].base, grey);
        assert_eq!(world.materials[1].base, dark);
        assert_eq!(world.materials[0].roughness, 0.5);
        let up = apply(&world.placed[0].model, [0.0, 0.0, 1.0]);
        let origin = apply(&world.placed[0].model, [0.0; 3]);
        assert!((up[1] - origin[1]) > 0.0, "z up becomes the engine's y up");
    }

    #[test]
    fn a_tile_rests_the_mark_on_a_rounded_slab_under_the_floor_line() {
        let mut world = World::default();
        place(
            &mut world,
            &json!({"single": true, "assets": [logo("tile")]}),
        )
        .unwrap();
        let b = Box3::of(&world.corners);
        assert!((b.hi[0] - b.lo[0] - 2.4).abs() < 2e-3, "{b:?}");
        assert!((b.lo[2] + 0.14).abs() < 2e-3, "{b:?}");
        let slab = &world.meshes[2];
        let sb = slab.bounds;
        assert!((sb.max[0] - sb.min[0] - 2.4).abs() < 2e-3);
        assert!(sb.max[2].abs() < 1e-5 && (sb.min[2] + 0.14).abs() < 1e-3);
        let corner = slab
            .positions
            .iter()
            .map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt())
            .fold(0.0f32, f32::max);
        let rounded = (1.2 - 0.528) * std::f32::consts::SQRT_2 + 0.528;
        assert!(
            (corner - rounded).abs() < 0.03,
            "{corner} against {rounded}"
        );
        let ground = &world.materials[2];
        assert_eq!(ground.base, look::bytes_linear([0x40; 3]));
        assert_eq!((ground.roughness, ground.clearcoat), (0.6, 0.0));
        assert_eq!(world.outlined, [0, 1]);
        let m = Box3::of(
            &world.meshes[0]
                .positions
                .iter()
                .map(|p| apply(&world.placed[0].model, p.map(f64::from)))
                .collect::<Vec<_>>(),
        );
        assert!((m.hi[0] - m.lo[0] - 2.4 * 0.62).abs() < 2e-3);
    }

    #[test]
    fn a_scene_places_copies_on_the_floor_and_outlines_all_but_the_catcher() {
        let icon = |name: &str, r: f64, x: f64| {
            json!({
                "kind": "icons",
                "preset": preset(),
                "icon": {"name": name, "parts": [{"kind": "ball", "r": r}]},
                "group": 0,
                "offset": [0.0, 0.0, 0.0],
                "copies": [{"at": [x, 0.0, 0.0], "turn": 0.0, "tilt": 0.0, "scale": 0.5}],
            })
        };
        let mut world = World::default();
        place(
            &mut world,
            &json!({"assets": [icon("a", 0.4, -0.6), icon("b", 0.2, 0.6)]}),
        )
        .unwrap();
        catcher(&mut world);
        assert_eq!(world.placed.len(), 3);
        let b = Box3::of(&world.corners);
        assert!(b.lo[2].abs() < 1e-6);
        assert!(
            (b.lo[0] + 1.7).abs() < 1e-3 && (b.hi[0] - 1.7).abs() < 1e-3,
            "{b:?}"
        );
        assert_eq!(world.catcher, Some(2));
        assert_eq!(world.outlined, [0, 1]);
        let floor = apply(&world.placed[2].model, [0.0; 3]);
        assert!((floor[1] + CATCHER_DROP).abs() < 1e-9);
    }

    #[test]
    fn same_paint_elements_keep_their_own_bevels() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 40"><circle cx="20" cy="20" r="15" fill="#808080"/><rect x="50" y="10" width="4" height="4" fill="#808080"/><rect x="54" y="10" width="4" height="4" fill="#404040"/><rect x="60" y="10" width="4" height="4" fill="#808080"/></svg>"##;
        let form = look::form(&json!({"form": {"depth": 0.1, "bevel": 0.03}})).unwrap();
        assert!(form.merge);
        let mark = Mark::build(svg, &form).unwrap();
        assert_eq!(mark.solids.len(), 2);
        assert_eq!(mark.solids[0].elements, [0, 1, 3]);
        let disc = mark.solids[0].mesh.bounds;
        assert!(
            (disc.max[2] - 0.13).abs() < 1e-4,
            "the disc keeps its own bevel: {disc:?}"
        );
    }

    #[test]
    fn gradients_ramp_in_linear_light_across_their_span() {
        let gradient = Gradient {
            id: "g".into(),
            stops: vec![(0.0, [0, 0, 0]), (1.0, [255, 255, 255])],
            x1: 0.0,
            x2: 1.0,
            y1: 0.0,
            y2: 0.0,
        };
        let texels = ramp(&gradient, [-0.25, 0.25], [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(texels.len(), RAMP as usize);
        assert_eq!(texels[0], [0, 0, 0, 255]);
        assert_eq!(texels[255], [255, 255, 255, 255]);
        let mid = texels[128][0];
        assert!((186..=190).contains(&mid), "{mid}");
    }

    #[test]
    fn matrices_compose_and_transpose() {
        let m = mul(&translation([1.0, 2.0, 3.0]), &about_z(90.0));
        let p = apply(&m, [1.0, 0.0, 0.0]);
        assert!((p[0] - 1.0).abs() < 1e-12 && (p[1] - 3.0).abs() < 1e-12);
        assert_eq!(apply(&AXES, [1.0, 2.0, 3.0]), [1.0, 3.0, -2.0]);
        let c = column_major(&m);
        assert_eq!(c[3], [1.0, 2.0, 3.0, 1.0]);
    }
}
