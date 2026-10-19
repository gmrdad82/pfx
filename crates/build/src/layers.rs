use crate::normal_map::{dilate, raster, rgb_png};
use crate::{Error, Part, fail, lod, maps, normal_map};
use glam::{DMat3, DMat4, DVec3};
use pfx_bake::detail::{Input, sample};
use pfx_glb::{Lod, Look, Looks, Map, Node};
use pfx_materials::{At, Material, NoiseLayer, encode_channel, resolve};
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
    cut(mesh, slots).map(|(piece, _)| piece)
}

fn cut(mesh: &Mesh, slots: &[Slot]) -> Option<(Mesh, Vec<u32>)> {
    let vertices = mesh.positions.len();
    let uvs = mesh.uvs.as_ref().filter(|uvs| uvs.len() == vertices)?;
    if mesh.normals.len() != vertices {
        return None;
    }
    let mut remap = vec![u32::MAX; vertices];
    let mut sources = Vec::new();
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
                sources.push(old);
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
    (!out.indices.is_empty()).then_some((out, sources))
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

pub(crate) struct Transfer<'a> {
    pub(crate) master: &'a Mesh,
    pub(crate) level: &'a Mesh,
    pub(crate) tangents: Option<&'a [[f32; 4]]>,
    pub(crate) distance: f32,
    pub(crate) along_master: bool,
}

fn framed(target: V3, n: V3, t: V3, b: V3) -> V3 {
    let det = dot(t, cross(b, n));
    let local = if det.abs() > 1e-12 {
        [
            dot(cross(b, n), target) / det,
            dot(cross(n, t), target) / det,
            dot(cross(t, b), target) / det,
        ]
    } else {
        [
            dot(normalize(t), target),
            dot(normalize(b), target),
            dot(n, target),
        ]
    };
    normalize(local)
}

pub(crate) fn transferred(
    transfer: &Transfer,
    matrix: DMat4,
    material: &Material,
    size: u32,
) -> Result<Texels, String> {
    let low = transfer.level;
    let uvs = low.uvs.as_deref().ok_or("the level has no TEXCOORD_0")?;
    let master_uvs = transfer
        .master
        .uvs
        .as_deref()
        .ok_or("the object has no TEXCOORD_0")?;
    let side = size as usize;
    let raster = raster(low, uvs, side, "layer maps")?;
    let mut placed = world(low, matrix);
    if let Some(tangents) = transfer.tangents {
        let frame = Frame::new(matrix);
        placed.tangents = tangents.iter().map(|&t| frame.tangent(t)).collect();
    }
    let seen = world(transfer.master, matrix);
    let hit_mesh = Mesh {
        positions: seen.positions,
        normals: seen.normals,
        indices: transfer.master.indices.clone(),
        ..Mesh::default()
    };
    let caster = normal_map::Master::new(&hit_mesh);
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
            let (raster, placed, caster) = (&raster, &placed, &caster);
            let corners = &transfer.master.indices;
            scope.spawn(move || {
                let first = chunk * rows * side;
                for (i, done) in fl.iter_mut().enumerate() {
                    let at = first + i;
                    let owner = raster.owner[at];
                    if owner == NOBODY {
                        continue;
                    }
                    let w = raster.weights[at];
                    let ids = [0, 1, 2].map(|k| low.indices[owner as usize * 3 + k] as usize);
                    let mix = |list: [V3; 3]| {
                        [0, 1, 2].map(|c| list[0][c] * w[0] + list[1][c] * w[1] + list[2][c] * w[2])
                    };
                    let p = mix(ids.map(|id| placed.positions[id]));
                    let n = normalize(mix(ids.map(|id| placed.normals[id])));
                    let tv = mix(ids.map(|id| {
                        let t = placed.tangents[id];
                        [t[0], t[1], t[2]]
                    }));
                    let sign: f32 = (0..3).map(|k| placed.tangents[ids[k]][3] * w[k]).sum();
                    let b = scale(cross(n, tv), if sign < 0.0 { -1.0 } else { 1.0 });
                    let own = || {
                        let u = ids.map(|id| uvs[id]);
                        [0, 1].map(|c| u[0][c] * w[0] + u[1][c] * w[1] + u[2][c] * w[2])
                    };
                    let (position, uv, surface_normal) = match caster.cast(p, n, transfer.distance)
                    {
                        Some(hit) => {
                            let tri = &corners[hit.index * 3..hit.index * 3 + 3];
                            let u = [0, 1, 2].map(|k| master_uvs[tri[k] as usize]);
                            let hw = hit.weights;
                            let uv =
                                [0, 1].map(|c| u[0][c] * hw[0] + u[1][c] * hw[1] + u[2][c] * hw[2]);
                            let towards = if transfer.along_master {
                                caster.normal(hit.index, hit.point, n)
                            } else {
                                n
                            };
                            (hit.point, uv, towards)
                        }
                        None => (p, own(), n),
                    };
                    let resolved = resolve(
                        material,
                        0.0,
                        &At {
                            position,
                            uv,
                            normal: surface_normal,
                            edge: 0.0,
                            time: 0.0,
                        },
                    );
                    bs[i * 3..i * 3 + 3]
                        .copy_from_slice(&resolved.material.base.map(|c| byte(encode_channel(c))));
                    ss[i * 3..i * 3 + 3].copy_from_slice(&[
                        255,
                        byte(resolved.material.roughness),
                        255,
                    ]);
                    let local = framed(resolved.normal, n, tv, b);
                    ns[i * 3..i * 3 + 3].copy_from_slice(&local.map(|c| byte(c * 0.5 + 0.5)));
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
    look.base_color = Some([1.0, 1.0, 1.0, plain.alpha]);
    look.roughness = Some(1.0);
    look.metallic = Some(plain.metalness);
    look.material = Some(serde_json::to_value(plain).map_err(|e| e.to_string())?);
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

pub(crate) fn bake_levels(
    recipe: &Recipe,
    parts: &[Part],
    lods: &[Lod],
    baked: &BTreeMap<usize, Vec<Baked>>,
    (materials, looks): (&BTreeMap<String, serde_json::Value>, &Looks),
) -> Result<BTreeMap<usize, Vec<Baked>>, Error> {
    let mut out = BTreeMap::new();
    for (at, level) in lods.iter().enumerate() {
        let (Some(list), Some(part)) = (baked.get(&level.of), parts.get(level.of)) else {
            continue;
        };
        if !lod::authored(level) {
            continue;
        }
        let object = &recipe.objects[part.object];
        let Some(wanted) = object.bake_layers else {
            continue;
        };
        let facts = &level.extras["lod"];
        let k = facts["level"].as_u64().unwrap_or(1).saturating_sub(1) as usize;
        let Some(asked) = object.lod.get(k) else {
            continue;
        };
        let key = format!("lod[{k}].path");
        let base = part
            .node
            .targets
            .as_ref()
            .map_or(&part.node.mesh, |t| &t.base);
        let error = facts["error"].as_f64().unwrap_or(0.0) as f32;
        let distance = asked
            .bake_distance
            .unwrap_or_else(|| lod::default_distance(base, error)) as f32;
        let matrix = if part.node.keep_transform {
            part.node.transform
        } else {
            DMat4::IDENTITY
        };
        let mut done = Vec::new();
        for item in list {
            let Some(material) = materials
                .get(&item.material)
                .and_then(|value| serde_json::from_value::<Material>(value.clone()).ok())
            else {
                continue;
            };
            let slots = |mesh: &Mesh| -> Vec<Slot> {
                mesh.slots
                    .iter()
                    .filter(|slot| slot.material == item.material)
                    .cloned()
                    .collect()
            };
            let (Some(master), Some((low, sources))) = (
                piece(&part.node.mesh, &slots(&part.node.mesh)),
                cut(&level.node.mesh, &slots(&level.node.mesh)),
            ) else {
                continue;
            };
            let tangents: Option<Vec<[f32; 4]>> = level
                .tangents
                .as_ref()
                .map(|list| sources.iter().map(|&s| list[s as usize]).collect());
            let transfer = Transfer {
                master: &master,
                level: &low,
                tangents: tangents.as_deref(),
                distance,
                along_master: asked.bake_normals,
            };
            let texels = transferred(&transfer, matrix, &material, wanted.size)
                .map_err(|text| fail(object, &key, format!("{}: {text}", level.node.name)))?;
            let look = look(&material, &texels, wanted.size, looks.get(&item.material))
                .map_err(|text| fail(object, &key, text))?;
            done.push(Baked {
                material: item.material.clone(),
                look,
            });
        }
        if !done.is_empty() {
            out.insert(at, done);
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
