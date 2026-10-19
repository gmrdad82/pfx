use crate::normal_map::{dilate, raster, rgb_png};
use crate::{Error, Part, fail, maps};
use glam::{DMat3, DMat4, DVec3};
use pfx_bake::detail::{Input, sample};
use pfx_glb::{Look, Looks, Map, Node};
use pfx_materials::{Material, NoiseLayer, encode_channel};
use pfx_mesh::{Mesh, Slot};
use pfx_recipe::Recipe;
use std::collections::BTreeMap;

const NOBODY: u32 = u32::MAX;
const THREADS: usize = 16;
const FLAT: [u8; 3] = [128, 128, 255];
const BARE: [u8; 3] = [0, 0, 0];
const SMOOTH: [u8; 3] = [255, 255, 255];

#[derive(Clone, Debug)]
pub(crate) struct Baked {
    pub(crate) material: String,
    pub(crate) look: Look,
}

pub(crate) struct Texels {
    pub(crate) base: Vec<u8>,
    pub(crate) surface: Vec<u8>,
    pub(crate) normal: Vec<u8>,
}

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}

fn normalize(a: V3) -> V3 {
    let l = length(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

fn unit(normal: V3, positions: &[V3], triangle: &[u32]) -> V3 {
    let size = length(normal);
    if (size - 1.0).abs() <= 5e-3 {
        return normal;
    }
    if size.is_finite() && size > 1e-12 {
        return scale(normal, 1.0 / size);
    }
    let [a, b, c] = [0, 1, 2].map(|k| positions[triangle[k] as usize]);
    normalize(cross(sub(b, a), sub(c, a)))
}

fn piece(mesh: &Mesh, slots: &[Slot]) -> Option<Mesh> {
    let vertices = mesh.positions.len();
    let uvs = mesh.uvs.as_ref().filter(|uvs| uvs.len() == vertices)?;
    if mesh.normals.len() != vertices {
        return None;
    }
    let mut remap = vec![u32::MAX; vertices];
    let mut out = Mesh {
        uvs: Some(Vec::new()),
        ..Mesh::default()
    };
    let ranges = slots.iter().map(|slot| {
        let start = (slot.start as usize).min(mesh.indices.len());
        start..(start + slot.count as usize).min(mesh.indices.len())
    });
    for triangle in ranges.flat_map(|range| mesh.indices[range].chunks_exact(3)) {
        if triangle.iter().any(|&i| i as usize >= vertices) {
            continue;
        }
        for &old in triangle {
            let at = &mut remap[old as usize];
            if *at == u32::MAX {
                *at = out.positions.len() as u32;
                out.positions.push(mesh.positions[old as usize]);
                out.normals
                    .push(unit(mesh.normals[old as usize], &mesh.positions, triangle));
                if let Some(list) = out.uvs.as_mut() {
                    list.push(uvs[old as usize]);
                }
            }
            out.indices.push(*at);
        }
    }
    (!out.indices.is_empty()).then_some(out)
}

pub(crate) fn layered(material: &Material) -> bool {
    material.layers.iter().any(|layer| layer.active())
}

fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(crate) struct Frame {
    matrix: DMat4,
    linear: DMat3,
    normals: DMat3,
}

impl Frame {
    pub(crate) fn new(matrix: DMat4) -> Frame {
        let linear = DMat3::from_mat4(matrix);
        Frame {
            matrix,
            linear,
            normals: linear.inverse().transpose(),
        }
    }

    fn apply(m: DMat3, v: V3) -> V3 {
        (m * DVec3::from(v.map(f64::from)))
            .to_array()
            .map(|c| c as f32)
    }

    pub(crate) fn point(&self, p: V3) -> V3 {
        let v = self.matrix.transform_point3(DVec3::from(p.map(f64::from)));
        v.to_array().map(|c| c as f32)
    }

    pub(crate) fn normal(&self, n: V3) -> V3 {
        normalize(Frame::apply(self.normals, n))
    }

    pub(crate) fn tangent(&self, t: [f32; 4]) -> [f32; 4] {
        let [x, y, z] = normalize(Frame::apply(self.linear, [t[0], t[1], t[2]]));
        [x, y, z, t[3]]
    }
}

pub(crate) struct World {
    pub(crate) positions: Vec<V3>,
    pub(crate) normals: Vec<V3>,
    pub(crate) tangents: Vec<[f32; 4]>,
}

pub(crate) fn world(mesh: &Mesh, matrix: DMat4) -> World {
    let frame = Frame::new(matrix);
    let tangents = pfx_load::surface::tangents(
        &mesh.positions,
        &mesh.normals,
        mesh.uvs.as_deref().unwrap_or_default(),
        &mesh.indices,
    );
    World {
        positions: mesh.positions.iter().map(|&p| frame.point(p)).collect(),
        normals: mesh.normals.iter().map(|&n| frame.normal(n)).collect(),
        tangents: tangents.into_iter().map(|t| frame.tangent(t)).collect(),
    }
}

pub(crate) fn texels(
    mesh: &Mesh,
    matrix: DMat4,
    material: &Material,
    size: u32,
) -> Result<Texels, String> {
    let uvs = mesh.uvs.as_deref().ok_or("the mesh has no TEXCOORD_0")?;
    let side = size as usize;
    let raster = raster(mesh, uvs, side, "layer maps")?;
    let placed = world(mesh, matrix);
    let input = Input {
        positions: &placed.positions,
        normals: &placed.normals,
        tangents: &placed.tangents,
        uvs,
        uvs1: None,
        indices: &mesh.indices,
        material,
    };
    let mut base = vec![0u8; side * side * 3];
    let mut surface = vec![0u8; side * side * 3];
    let mut normal = vec![0u8; side * side * 3];
    let mut filled = vec![false; side * side];
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, THREADS);
    let rows = side.div_ceil(threads);
    std::thread::scope(|scope| {
        for (chunk, (((bs, ss), ns), fl)) in base
            .chunks_mut(rows * side * 3)
            .zip(surface.chunks_mut(rows * side * 3))
            .zip(normal.chunks_mut(rows * side * 3))
            .zip(filled.chunks_mut(rows * side))
            .enumerate()
        {
            let raster = &raster;
            let input = &input;
            scope.spawn(move || {
                let first = chunk * rows * side;
                for (i, done) in fl.iter_mut().enumerate() {
                    let at = first + i;
                    let owner = raster.owner[at];
                    if owner == NOBODY {
                        continue;
                    }
                    let t = owner as usize * 3;
                    let ids = [0, 1, 2].map(|k| input.indices[t + k] as usize);
                    let centre = [
                        ((at % side) as f32 + 0.5) / size as f32,
                        ((at / side) as f32 + 0.5) / size as f32,
                    ];
                    let found = sample(input, ids, raster.weights[at], centre);
                    bs[i * 3..i * 3 + 3]
                        .copy_from_slice(&found.base.map(|c| byte(encode_channel(c))));
                    ss[i * 3..i * 3 + 3].copy_from_slice(&[255, byte(found.roughness), 255]);
                    ns[i * 3..i * 3 + 3]
                        .copy_from_slice(&found.normal.map(|c| byte(c * 0.5 + 0.5)));
                    *done = true;
                }
            });
        }
    });
    let mut again = filled.clone();
    dilate(&mut base, &mut again, side, &BARE);
    let mut again = filled.clone();
    dilate(&mut surface, &mut again, side, &SMOOTH);
    dilate(&mut normal, &mut filled, side, &FLAT);
    Ok(Texels {
        base,
        surface,
        normal,
    })
}

fn image(pixels: &[u8], size: u32) -> Result<Map, String> {
    Ok(Map::new(maps::image(rgb_png(pixels, size)?)?))
}

pub(crate) fn look(
    material: &Material,
    texels: &Texels,
    size: u32,
    imported: Option<&Look>,
) -> Result<Look, String> {
    let mut plain = *material;
    plain.layers = [NoiseLayer::default(); pfx_materials::LAYER_CAP];
    let mut look = imported.cloned().unwrap_or_default();
    maps::factors(&mut look, &plain);
    look.base_color = Some([1.0; 4]);
    look.roughness = Some(1.0);
    look.metallic = Some(plain.metalness);
    look.base = Some(image(&texels.base, size)?);
    look.metal_roughness = Some(image(&texels.surface, size)?);
    look.normal = Some(image(&texels.normal, size)?);
    Ok(look)
}

pub(crate) fn name(material: &str, node: &str) -> String {
    format!("{material}+{node}")
}

pub(crate) fn bake(
    recipe: &Recipe,
    parts: &[Part],
    materials: &BTreeMap<String, serde_json::Value>,
    looks: &Looks,
    notes: &mut Vec<String>,
) -> Result<BTreeMap<usize, Vec<Baked>>, Error> {
    let mut out = BTreeMap::new();
    let mut baked_any: BTreeMap<usize, bool> = BTreeMap::new();
    for (at, part) in parts.iter().enumerate() {
        let object = &recipe.objects[part.object];
        let Some(wanted) = object.bake_layers else {
            continue;
        };
        let node = &part.node;
        let vertices = node.mesh.positions.len();
        if node
            .mesh
            .uvs
            .as_ref()
            .is_none_or(|uvs| uvs.len() != vertices)
        {
            return Err(fail(
                object,
                "bake_layers",
                format!(
                    "the node `{}` has no TEXCOORD_0 to bake the layer maps on, expected uv other than \"none\"",
                    node.name
                ),
            ));
        }
        let matrix = if node.keep_transform {
            node.transform
        } else {
            DMat4::IDENTITY
        };
        let mut list = Vec::new();
        let mut shared: Vec<(&str, Vec<Slot>)> = Vec::new();
        for slot in &node.mesh.slots {
            match shared.iter_mut().find(|(name, _)| *name == slot.material) {
                Some((_, slots)) => slots.push(slot.clone()),
                None => shared.push((&slot.material, vec![slot.clone()])),
            }
        }
        for (name, slots) in shared {
            let Some(value) = materials.get(name) else {
                continue;
            };
            let material = match serde_json::from_value::<Material>(value.clone()) {
                Ok(material) => material,
                Err(e) => {
                    notes.push(format!(
                        "object[{}].bake_layers: the material `{}` does not read as an engine material ({e}); its layers stay unbaked",
                        object.name, name
                    ));
                    continue;
                }
            };
            if !layered(&material) {
                continue;
            }
            let Some(mesh) = piece(&node.mesh, &slots) else {
                notes.push(format!(
                    "object[{}].bake_layers: the node `{}` has no normals or no triangles for the material `{name}`, so its layers stay unbaked",
                    object.name, node.name
                ));
                continue;
            };
            let texels = texels(&mesh, matrix, &material, wanted.size)
                .map_err(|text| fail(object, "bake_layers", format!("{}: {text}", node.name)))?;
            let look = look(&material, &texels, wanted.size, looks.get(name))
                .map_err(|text| fail(object, "bake_layers", text))?;
            list.push(Baked {
                material: name.to_string(),
                look,
            });
        }
        let any = baked_any.entry(part.object).or_default();
        *any |= !list.is_empty();
        if !list.is_empty() {
            out.insert(at, list);
        }
    }
    for (object, any) in baked_any {
        if !any {
            notes.push(format!(
                "object[{}].bake_layers: no material of the object has a procedural layer, so nothing was baked",
                recipe.objects[object].name
            ));
        }
    }
    Ok(out)
}

fn renamed(mesh: &mut Mesh, from: &str, to: &str) {
    for slot in mesh.slots.iter_mut().filter(|slot| slot.material == from) {
        slot.material = to.to_string();
    }
}

fn retitle(node: &mut Node, from: &str, to: &str) {
    renamed(&mut node.mesh, from, to);
    if let Some(targets) = node.targets.as_mut() {
        renamed(&mut targets.base, from, to);
    }
}

pub(crate) fn dress(node: &mut Node, baked: &[Baked], looks: &mut Looks) {
    for item in baked {
        let to = name(&item.material, &node.name);
        looks.insert(to.clone(), item.look.clone());
        retitle(node, &item.material, &to);
    }
}

pub(crate) fn rename(level: &mut Node, base: &Node, baked: &[Baked]) {
    for item in baked {
        retitle(level, &item.material, &name(&item.material, &base.name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DQuat;
    use pfx_materials::{At, Crinkle, Grime, NoiseKind, WallMottle, resolve};

    fn add(a: V3, b: V3) -> V3 {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }

    fn blend(list: [V3; 3], w: [f32; 3]) -> V3 {
        add(
            add(scale(list[0], w[0]), scale(list[1], w[1])),
            scale(list[2], w[2]),
        )
    }

    fn bumped(cells: u32) -> Mesh {
        let mut m = Mesh {
            uvs: Some(Vec::new()),
            ..Mesh::default()
        };
        let side = cells + 1;
        let height = |u: f32, v: f32| 0.08 * (u * 3.0).sin() * (v * 2.0).cos();
        for y in 0..side {
            for x in 0..side {
                let (u, v) = (x as f32 / cells as f32, y as f32 / cells as f32);
                let e = 1e-3;
                let du = (height(u + e, v) - height(u - e, v)) / (2.0 * e);
                let dv = (height(u, v + e) - height(u, v - e)) / (2.0 * e);
                m.positions.push([u, 1.0 - v, height(u, v)]);
                m.normals.push(normalize([-du, dv, 1.0]));
                m.uvs.as_mut().unwrap().push([u, v]);
            }
        }
        for y in 0..cells {
            for x in 0..cells {
                let a = y * side + x;
                m.indices
                    .extend([a, a + side, a + 1, a + 1, a + side, a + side + 1]);
            }
        }
        m
    }

    fn layered_material() -> Material {
        let mut material = Material {
            base: [0.6, 0.5, 0.4],
            roughness: 0.55,
            ..Material::default()
        };
        material.layers[0] = WallMottle::SAMPLE.layer(1.0, 3);
        material.layers[1] = Grime::SAMPLE.layer(0.8, 0);
        material.layers[2] = NoiseLayer {
            kind: NoiseKind::Fbm,
            frequency: 6.0,
            amplitude: 0.5,
            seed: 0,
            params: [0.0; pfx_materials::PARAMS],
        };
        material.layers[3] = Crinkle {
            frequencies: [8.0, 21.0],
            gains: [0.04, 0.02],
        }
        .layer(1.0, 5);
        material
    }

    fn inside(mesh: &Mesh, uv: [f32; 2]) -> Option<(usize, [f32; 3])> {
        let uvs = mesh.uvs.as_deref().unwrap();
        for (t, tri) in mesh.indices.chunks_exact(3).enumerate() {
            let [a, b, c] = [0, 1, 2].map(|k| uvs[tri[k] as usize].map(f64::from));
            let edge = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
                (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
            };
            let at = uv.map(f64::from);
            let area = edge(a, b, c);
            let w = [
                edge(b, c, at) / area,
                edge(c, a, at) / area,
                edge(a, b, at) / area,
            ];
            if w.iter().all(|&w| w >= -1e-9) {
                return Some((t, w.map(|w| w as f32)));
            }
        }
        None
    }

    fn near(a: u8, b: u8) -> bool {
        a.abs_diff(b) <= 1
    }

    #[test]
    fn baked_layer_maps_match_the_layer_evaluation_at_every_texel_centre() {
        let mesh = bumped(6);
        let matrix = DMat4::from_scale_rotation_translation(
            DVec3::splat(1.5),
            DQuat::from_rotation_y(0.7),
            DVec3::new(1.0, 0.5, -3.0),
        );
        let material = layered_material();
        let size = 48;
        let baked = texels(&mesh, matrix, &material, size).unwrap();
        let placed = world(&mesh, matrix);
        let side = size as usize;
        let mut bent = 0;
        for y in 0..side {
            for x in 0..side {
                let uv = [
                    (x as f32 + 0.5) / size as f32,
                    (y as f32 + 0.5) / size as f32,
                ];
                let (triangle, w) = inside(&mesh, uv).unwrap();
                let ids = [0, 1, 2].map(|k| mesh.indices[triangle * 3 + k] as usize);
                let n = normalize(blend(ids.map(|id| placed.normals[id]), w));
                let at = At {
                    position: blend(ids.map(|id| placed.positions[id]), w),
                    uv,
                    normal: n,
                    edge: 0.0,
                    time: 0.0,
                };
                let resolved = resolve(&material, 0.0, &at);
                let texel = (y * side + x) * 3;
                for c in 0..3 {
                    let want = byte(encode_channel(resolved.material.base[c]));
                    assert!(near(baked.base[texel + c], want), "base at {x},{y}");
                }
                assert!(
                    near(baked.surface[texel + 1], byte(resolved.material.roughness)),
                    "roughness at {x},{y}"
                );
                let tv = blend(
                    ids.map(|id| {
                        let t = placed.tangents[id];
                        [t[0], t[1], t[2]]
                    }),
                    w,
                );
                let t = normalize(sub(tv, scale(n, dot(n, tv))));
                let sign: f32 = (0..3).map(|k| placed.tangents[ids[k]][3] * w[k]).sum();
                let b = scale(cross(n, t), sign.signum());
                let code =
                    [0, 1, 2].map(|c| f32::from(baked.normal[texel + c]) / 255.0 * 2.0 - 1.0);
                let shown = normalize(add(
                    add(scale(t, code[0]), scale(b, code[1])),
                    scale(n, code[2]),
                ));
                assert!(
                    dot(shown, resolved.normal) > 0.999,
                    "normal at {x},{y}: {shown:?} for {:?}",
                    resolved.normal
                );
                if dot(resolved.normal, n) < 0.9999 {
                    bent += 1;
                }
            }
        }
        assert!(bent > side * side / 2, "{bent} texels bent");
        let again = texels(&mesh, matrix, &material, size).unwrap();
        assert_eq!(again.base, baked.base);
        assert_eq!(again.normal, baked.normal);
    }
}
