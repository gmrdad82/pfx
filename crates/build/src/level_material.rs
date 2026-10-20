use crate::layers::{
    BARE, Baked, FLAT, Frame, NOBODY, SMOOTH, THREADS, byte, framed, image, layered, world,
};
use crate::normal_map::{Master, dilate, raster};
use crate::{Error, Part, fail, lod, maps};
use glam::DMat4;
use pfx_glb::{Lod, Look, Looks, Map, Mime};
use pfx_materials::{
    At, LAYER_CAP, Material, NoiseLayer, UvTransform, encode_channel, linear_channel, resolve,
};
use pfx_mesh::Mesh;
use pfx_recipe::Recipe;
use pfx_trace::sobol::{generate_4d, hash_uint2};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SAMPLES: usize = 16;
const RETRIES: usize = 10;
const CLAMP: u32 = 33071;
const MIRROR: u32 = 33648;

type V3 = [f32; 3];
type Uv = [f32; 2];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: V3) -> V3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

fn mixed<const N: usize>(list: [[f32; N]; 3], w: [f32; 3]) -> [f32; N] {
    std::array::from_fn(|c| list[0][c] * w[0] + list[1][c] * w[1] + list[2][c] * w[2])
}

fn final_mix(mut a: u32, mut b: u32, mut c: u32) -> u32 {
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(14));
    a ^= c;
    a = a.wrapping_sub(c.rotate_left(11));
    b ^= a;
    b = b.wrapping_sub(a.rotate_left(25));
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(16));
    a ^= c;
    a = a.wrapping_sub(c.rotate_left(4));
    b ^= a;
    b = b.wrapping_sub(a.rotate_left(14));
    c ^= b;
    c.wrapping_sub(b.rotate_left(24))
}

fn hash_uint3(kx: u32, ky: u32, kz: u32) -> u32 {
    let start = 0xdeadbeef_u32.wrapping_add((3 << 2) + 13);
    final_mix(
        start.wrapping_add(kx),
        start.wrapping_add(ky),
        start.wrapping_add(kz),
    )
}

fn uint_to_float_incl(n: u32) -> f32 {
    n as f32 * (1.0 / u32::MAX as f32)
}

fn hash_float2_to_float2(k: Uv) -> Uv {
    let [x, y] = k.map(f32::to_bits);
    [
        uint_to_float_incl(hash_uint2(x, y)),
        uint_to_float_incl(hash_uint3(x, y, 1.0f32.to_bits())),
    ]
}

struct Differentials {
    dudx: f32,
    dudy: f32,
    dvdx: f32,
    dvdy: f32,
}

fn differentials(uv: [Uv; 3], side: f32) -> Differentials {
    let p = uv.map(|c| [c[0] * side, c[1] * side]);
    let area =
        (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
    if area.abs() < 1e-12 {
        return Differentials {
            dudx: 0.0,
            dudy: 0.0,
            dvdx: 0.0,
            dvdy: 0.0,
        };
    }
    Differentials {
        dudx: -(p[2][1] - p[1][1]) / area,
        dudy: (p[2][0] - p[1][0]) / area,
        dvdx: -(p[0][1] - p[2][1]) / area,
        dvdy: (p[0][0] - p[2][0]) / area,
    }
}

fn jitter(w: [f32; 3], mut filter: Uv, d: &Differentials) -> [f32; 3] {
    for _ in 0..RETRIES {
        let u = w[0] + (filter[0] - 0.5) * d.dudx + (filter[1] - 0.5) * d.dudy;
        let v = w[1] + (filter[0] - 0.5) * d.dvdx + (filter[1] - 0.5) * d.dvdy;
        if u > 0.0 && v > 0.0 && u + v < 1.0 {
            return [u, v, 1.0 - u - v];
        }
        filter = hash_float2_to_float2(filter);
    }
    w
}

fn wrapped(i: i64, n: usize, mode: u32) -> usize {
    let n = n as i64;
    let out = match mode {
        CLAMP => i.clamp(0, n - 1),
        MIRROR => {
            let m = i.rem_euclid(2 * n);
            if m < n { m } else { 2 * n - 1 - m }
        }
        _ => i.rem_euclid(n),
    };
    out as usize
}

struct Texture {
    width: usize,
    height: usize,
    texels: Vec<[u8; 4]>,
    wrap: [u32; 2],
    set: usize,
    transform: Option<UvTransform>,
}

impl Texture {
    fn read(map: &Map) -> Option<Texture> {
        let image = match map.image.mime {
            Mime::Png => pfx_load::Image::png(&map.image.bytes),
            Mime::Jpeg => pfx_load::Image::jpeg(&map.image.bytes),
        }
        .ok()?;
        Some(Texture {
            width: image.width as usize,
            height: image.height as usize,
            texels: pfx_load::surface::rgba8(&image),
            wrap: map.sampler.map_or([0; 2], |s| s.wrap),
            set: map
                .transform
                .and_then(|t| t.tex_coord)
                .unwrap_or(map.tex_coord)
                .min(1) as usize,
            transform: map.transform.map(|t| UvTransform {
                set: 0,
                offset: t.offset,
                rotation: t.rotation,
                scale: t.scale,
            }),
        })
    }

    fn at(&self, uvs: [Uv; 2]) -> [f32; 4] {
        let uv = self
            .transform
            .map_or(uvs[self.set], |t| t.apply(uvs[self.set]));
        let x = uv[0] * self.width as f32 - 0.5;
        let y = uv[1] * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let fetch = |dx: i64, dy: i64| {
            let i = wrapped(x0 as i64 + dx, self.width, self.wrap[0]);
            let j = wrapped(y0 as i64 + dy, self.height, self.wrap[1]);
            self.texels[j * self.width + i].map(|c| f32::from(c) / 255.0)
        };
        let [a, b, c, d] = [fetch(0, 0), fetch(1, 0), fetch(0, 1), fetch(1, 1)];
        std::array::from_fn(|k| {
            (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty
        })
    }
}

struct Shader {
    material: Material,
    base: Option<Texture>,
    surface: Option<Texture>,
}

#[derive(Clone, Copy, Default)]
struct Shaded {
    base: V3,
    roughness: f32,
    metalness: f32,
    normal: V3,
}

impl Shader {
    fn new(name: &str, materials: &BTreeMap<String, Value>, looks: &Looks) -> Shader {
        let look = looks.get(name);
        let material = materials
            .get(name)
            .and_then(|value| serde_json::from_value::<Material>(value.clone()).ok())
            .unwrap_or_else(|| {
                let mut material = Material::default();
                if let Some(look) = look {
                    if let Some([r, g, b, a]) = look.base_color {
                        material.base = [r, g, b];
                        material.alpha = a;
                    }
                    material.roughness = look.roughness.unwrap_or(material.roughness);
                    material.metalness = look.metallic.unwrap_or(material.metalness);
                }
                material
            });
        Shader {
            material,
            base: look.and_then(|l| l.base.as_ref()).and_then(Texture::read),
            surface: look
                .and_then(|l| l.metal_roughness.as_ref())
                .and_then(Texture::read),
        }
    }

    fn shade(&self, at: &At, uvs: [Uv; 2]) -> Shaded {
        let resolved = resolve(&self.material, 0.0, at);
        let mut base = resolved.material.base;
        let mut roughness = resolved.material.roughness;
        let mut metalness = resolved.material.metalness;
        if let Some(texture) = &self.base {
            let c = texture.at(uvs);
            for k in 0..3 {
                base[k] *= linear_channel(c[k]);
            }
        }
        if let Some(texture) = &self.surface {
            let c = texture.at(uvs);
            roughness *= c[1];
            metalness *= c[2];
        }
        Shaded {
            base,
            roughness,
            metalness,
            normal: resolved.normal,
        }
    }
}

fn owners(mesh: &Mesh, names: &mut Vec<String>) -> Vec<u32> {
    let mut index = |name: &str| match names.iter().position(|n| n == name) {
        Some(at) => at as u32,
        None => {
            names.push(name.to_string());
            names.len() as u32 - 1
        }
    };
    let triangles = mesh.triangle_count();
    if mesh.slots.is_empty() {
        let only = index("");
        return vec![only; triangles];
    }
    let mut out = vec![NOBODY; triangles];
    for slot in &mesh.slots {
        let at = index(&slot.material);
        let first = (slot.start / 3) as usize;
        let last = ((slot.start + slot.count) / 3) as usize;
        for owner in out.iter_mut().take(last.min(triangles)).skip(first) {
            *owner = at;
        }
    }
    let fallback = index("");
    for owner in out.iter_mut().filter(|o| **o == NOBODY) {
        *owner = fallback;
    }
    out
}

fn blend(out: &mut Value, others: &[(Option<&Value>, u64)]) {
    match out {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                let inner: Vec<_> = others
                    .iter()
                    .map(|(v, w)| (v.and_then(|v| v.get(key)), *w))
                    .collect();
                blend(value, &inner);
            }
        }
        Value::Array(list) => {
            for (at, value) in list.iter_mut().enumerate() {
                let inner: Vec<_> = others
                    .iter()
                    .map(|(v, w)| (v.and_then(|v| v.get(at)), *w))
                    .collect();
                blend(value, &inner);
            }
        }
        Value::Number(n) if n.is_f64() => {
            let (sum, weight) = others
                .iter()
                .filter_map(|(v, w)| {
                    let v = v.filter(|v| v.is_f64())?.as_f64()?;
                    Some((v * *w as f64, *w as f64))
                })
                .fold((0.0, 0.0), |(s, t), (v, w)| (s + v, t + w));
            if weight > 0.0
                && let Some(mean) = serde_json::Number::from_f64(sum / weight)
            {
                *out = Value::Number(mean);
            }
        }
        _ => {}
    }
}

fn plain(material: &Material) -> Material {
    let mut plain = *material;
    plain.layers = [NoiseLayer::default(); LAYER_CAP];
    plain
}

pub(crate) fn record(materials: &[(Material, u64)]) -> Result<(Material, usize), String> {
    let dominant = materials
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.1.cmp(&b.1.1).then(b.0.cmp(&a.0)))
        .map_or(0, |(at, _)| at);
    let values = materials
        .iter()
        .map(|(m, w)| Ok((serde_json::to_value(plain(m))?, *w)))
        .collect::<Result<Vec<_>, serde_json::Error>>()
        .map_err(|e| e.to_string())?;
    let Some((first, _)) = values.get(dominant) else {
        return Ok((Material::default(), 0));
    };
    let mut out = first.clone();
    let others: Vec<_> = values
        .iter()
        .filter(|(_, w)| *w > 0)
        .map(|(v, w)| (Some(v), *w))
        .collect();
    blend(&mut out, &others);
    let material = serde_json::from_value::<Material>(out).map_err(|e| e.to_string())?;
    Ok((plain(&material), dominant))
}

pub(crate) struct Bake<'a> {
    pub(crate) master: &'a Mesh,
    pub(crate) level: &'a Mesh,
    pub(crate) tangents: Option<&'a [[f32; 4]]>,
    pub(crate) distance: f32,
    pub(crate) reach: f32,
    pub(crate) along_master: bool,
    pub(crate) size: u32,
}

pub(crate) struct Maps {
    pub(crate) base: Vec<u8>,
    pub(crate) surface: Vec<u8>,
    pub(crate) normal: Option<Vec<u8>>,
    pub(crate) material: Material,
    pub(crate) dominant: String,
}

impl Bake<'_> {
    pub(crate) fn run(
        &self,
        matrix: DMat4,
        materials: &BTreeMap<String, Value>,
        looks: &Looks,
    ) -> Result<Maps, String> {
        let low = self.level;
        let uvs = low.uvs.as_deref().ok_or("the level has no TEXCOORD_0")?;
        let mut names = Vec::new();
        let master_owner = owners(self.master, &mut names);
        let low_owner = owners(low, &mut names);
        let shaders: Vec<Shader> = names
            .iter()
            .map(|name| Shader::new(name, materials, looks))
            .collect();
        let detailed = shaders.iter().any(|s| layered(&s.material));
        let side = self.size as usize;
        let raster = raster(low, uvs, side, "material maps")?;
        let mut placed = world(low, matrix);
        if let Some(tangents) = self.tangents {
            let frame = Frame::new(matrix);
            placed.tangents = tangents.iter().map(|&t| frame.tangent(t)).collect();
        }
        let seen = world(self.master, matrix);
        let hit_mesh = Mesh {
            positions: seen.positions,
            normals: seen.normals,
            indices: self.master.indices.clone(),
            ..Mesh::default()
        };
        let caster = Master::new(&hit_mesh);
        let vertices = self.master.positions.len();
        let master_uvs = self.master.uvs.as_deref().filter(|u| u.len() == vertices);
        let master_uvs1 = self.master.uvs1.as_deref().filter(|u| u.len() == vertices);
        let low_uvs1 = low
            .uvs1
            .as_deref()
            .filter(|u| u.len() == low.positions.len());
        let mut base = vec![0u8; side * side * 3];
        let mut surface = vec![0u8; side * side * 3];
        let mut normal = vec![0u8; side * side * 3];
        let mut filled = vec![false; side * side];
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, THREADS);
        let rows = side.div_ceil(threads);
        let mut counts = vec![0u64; names.len()];
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for (chunk, (((bs, ss), ns), fl)) in base
                .chunks_mut(rows * side * 3)
                .zip(surface.chunks_mut(rows * side * 3))
                .zip(normal.chunks_mut(rows * side * 3))
                .zip(filled.chunks_mut(rows * side))
                .enumerate()
            {
                let (raster, placed, caster) = (&raster, &placed, &caster);
                let (shaders, master_owner, low_owner) = (&shaders, &master_owner, &low_owner);
                let corners = &self.master.indices;
                let slots = names.len();
                handles.push(scope.spawn(move || {
                    let mut counts = vec![0u64; slots];
                    let mut points = [[0.0f32; 4]; SAMPLES];
                    let first = chunk * rows * side;
                    for (i, done) in fl.iter_mut().enumerate() {
                        let at = first + i;
                        let owner = raster.owner[at];
                        if owner == NOBODY {
                            continue;
                        }
                        let ids = [0, 1, 2].map(|k| low.indices[owner as usize * 3 + k] as usize);
                        let d = differentials(ids.map(|id| uvs[id]), side as f32);
                        generate_4d(
                            &mut points,
                            hash_uint2((at % side) as u32, (at / side) as u32),
                        );
                        let mut sum = Shaded::default();
                        for (s, point) in points.iter().enumerate() {
                            let filter = if s == 0 {
                                [0.5, 0.5]
                            } else {
                                [point[0], point[1]]
                            };
                            let w = jitter(raster.weights[at], filter, &d);
                            let p = mixed(ids.map(|id| placed.positions[id]), w);
                            let n = normalize(mixed(ids.map(|id| placed.normals[id]), w));
                            let tv = mixed(
                                ids.map(|id| {
                                    let t = placed.tangents[id];
                                    [t[0], t[1], t[2]]
                                }),
                                w,
                            );
                            let sign: f32 = (0..3).map(|k| placed.tangents[ids[k]][3] * w[k]).sum();
                            let b = scale(cross(n, tv), if sign < 0.0 { -1.0 } else { 1.0 });
                            let own_uvs = || {
                                let one =
                                    low_uvs1.map_or([0.0; 2], |u| mixed(ids.map(|id| u[id]), w));
                                [mixed(ids.map(|id| uvs[id]), w), one]
                            };
                            let (slot, position, pair, towards) =
                                match caster.cast_within(p, n, self.distance, self.reach) {
                                    Some(hit) => {
                                        let tri = &corners[hit.index * 3..hit.index * 3 + 3];
                                        let on = |list: Option<&[Uv]>| {
                                            list.map_or([0.0; 2], |u| {
                                                mixed(
                                                    [0, 1, 2].map(|k| u[tri[k] as usize]),
                                                    hit.weights,
                                                )
                                            })
                                        };
                                        let towards = if self.along_master {
                                            caster.normal(hit.index, hit.point, n)
                                        } else {
                                            n
                                        };
                                        (
                                            master_owner[hit.index] as usize,
                                            hit.point,
                                            [on(master_uvs), on(master_uvs1)],
                                            towards,
                                        )
                                    }
                                    None => (low_owner[owner as usize] as usize, p, own_uvs(), n),
                                };
                            counts[slot] += 1;
                            let found = shaders[slot].shade(
                                &At {
                                    position,
                                    uv: pair[0],
                                    normal: towards,
                                    edge: 0.0,
                                    time: 0.0,
                                },
                                pair,
                            );
                            sum.base = add(sum.base, found.base);
                            sum.roughness += found.roughness;
                            sum.metalness += found.metalness;
                            sum.normal = add(sum.normal, framed(found.normal, n, tv, b));
                        }
                        let share = 1.0 / SAMPLES as f32;
                        bs[i * 3..i * 3 + 3].copy_from_slice(
                            &scale(sum.base, share).map(|c| byte(encode_channel(c))),
                        );
                        ss[i * 3..i * 3 + 3].copy_from_slice(&[
                            255,
                            byte(sum.roughness * share),
                            byte(sum.metalness * share),
                        ]);
                        ns[i * 3..i * 3 + 3]
                            .copy_from_slice(&normalize(sum.normal).map(|c| byte(c * 0.5 + 0.5)));
                        *done = true;
                    }
                    counts
                }));
            }
            for handle in handles {
                if let Ok(found) = handle.join() {
                    for (total, n) in counts.iter_mut().zip(found) {
                        *total += n;
                    }
                }
            }
        });
        let mut again = filled.clone();
        dilate(&mut base, &mut again, side, &BARE);
        let mut again = filled.clone();
        dilate(&mut surface, &mut again, side, &SMOOTH);
        dilate(&mut normal, &mut filled, side, &FLAT);
        let weighted: Vec<(Material, u64)> = shaders
            .iter()
            .zip(&counts)
            .map(|(s, &n)| (s.material, n))
            .collect();
        let (material, dominant) = record(&weighted)?;
        Ok(Maps {
            base,
            surface,
            normal: (self.along_master || detailed).then_some(normal),
            material,
            dominant: names.get(dominant).cloned().unwrap_or_default(),
        })
    }
}

pub(crate) fn look(maps: &Maps, size: u32, looks: &Looks) -> Result<Look, String> {
    let plain = maps.material;
    let mut look = looks.get(&maps.dominant).cloned().unwrap_or_default();
    look.base = None;
    look.normal = None;
    look.metal_roughness = None;
    look.occlusion = None;
    look.detail_normal = None;
    maps::factors(&mut look, &plain);
    look.base_color = Some([1.0, 1.0, 1.0, plain.alpha]);
    look.roughness = Some(1.0);
    look.metallic = Some(1.0);
    look.material = Some(serde_json::to_value(plain).map_err(|e| e.to_string())?);
    look.base = Some(image(&maps.base, size)?);
    look.metal_roughness = Some(image(&maps.surface, size)?);
    if let Some(normal) = &maps.normal {
        look.normal = Some(image(normal, size)?);
    }
    Ok(look)
}

pub(crate) fn bake(
    recipe: &Recipe,
    parts: &[Part],
    lods: &[Lod],
    (materials, looks): (&BTreeMap<String, Value>, &Looks),
) -> Result<BTreeMap<usize, Baked>, Error> {
    let mut out = BTreeMap::new();
    for (at, level) in lods.iter().enumerate() {
        let Some(part) = parts.get(level.of) else {
            continue;
        };
        let object = &recipe.objects[part.object];
        let facts = &level.extras["lod"];
        let k = facts["level"].as_u64().unwrap_or(1).saturating_sub(1) as usize;
        let Some(asked) = object.lod.get(k).filter(|asked| asked.bake_material) else {
            continue;
        };
        let key = format!("lod[{k}].bake_material");
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
        let job = Bake {
            master: &part.node.mesh,
            level: &level.node.mesh,
            tangents: level.tangents.as_deref(),
            distance,
            reach: asked.bake_reach.map_or(distance, |r| r as f32),
            along_master: asked.bake_normals,
            size: asked.material_size,
        };
        let made = job
            .run(matrix, materials, looks)
            .and_then(|maps| look(&maps, asked.material_size, looks))
            .map_err(|text| fail(object, &key, format!("{}: {text}", level.node.name)))?;
        out.insert(
            at,
            Baked {
                material: level.node.name.clone(),
                look: made,
            },
        );
    }
    Ok(out)
}
