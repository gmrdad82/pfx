use std::collections::VecDeque;
use std::fs;
use std::path::Path;

use pfx_materials::{At, Material, resolve};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub mod atlas;
pub mod ktx2;

pub const LIMIT: u32 = 4096;
pub const FLOOR: u32 = 64;

const NEAR: f32 = 0.4;
const FAR: f32 = 6.0;

pub struct Input<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
    pub tangents: &'a [[f32; 4]],
    pub uvs: &'a [[f32; 2]],
    pub uvs1: Option<&'a [[f32; 2]]>,
    pub indices: &'a [u32],
    pub material: &'a Material,
}

impl Input<'_> {
    pub fn raster(&self) -> &[[f32; 2]] {
        self.uvs1.unwrap_or(self.uvs)
    }

    pub fn uv_set(&self) -> u32 {
        u32::from(self.uvs1.is_some())
    }
}

pub struct View {
    pub matrix: [[f32; 4]; 4],
    pub size: [u32; 2],
}

impl View {
    pub fn look(camera: &crate::recipe::DetailView) -> Self {
        let eye = camera.position;
        let forward = unit(sub(camera.target, eye));
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let view = [
            [right[0], up[0], -forward[0], 0.0],
            [right[1], up[1], -forward[1], 0.0],
            [right[2], up[2], -forward[2], 0.0],
            [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
        ];
        let tan_y = (camera.fov_y_deg.to_radians() * 0.5).tan();
        let tan_x = tan_y * camera.width as f32 / camera.height as f32;
        let projection = [
            [1.0 / tan_x, 0.0, 0.0, 0.0],
            [0.0, 1.0 / tan_y, 0.0, 0.0],
            [camera.shift[0], camera.shift[1], FAR / (NEAR - FAR), -1.0],
            [0.0, 0.0, FAR * NEAR / (NEAR - FAR), 0.0],
        ];
        let mut matrix = [[0.0; 4]; 4];
        for (column, out) in matrix.iter_mut().enumerate() {
            for (row, value) in out.iter_mut().enumerate() {
                *value = (0..4).map(|k| projection[k][row] * view[column][k]).sum();
            }
        }
        Self {
            matrix,
            size: [camera.width, camera.height],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub path: String,
    pub node: String,
    pub size: u32,
    pub crop: Crop,
    pub page: u32,
    pub bytes: u64,
    pub input_hash: String,
    pub uv_set: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub parts: Vec<Part>,
    pub total_bytes: u64,
    pub vram_bytes: u64,
    pub decoded_vram_bytes: u64,
    pub full_bytes: u64,
    pub page: [u32; 2],
    pub pages: u32,
    pub psnr: [Option<f64>; 3],
}

pub fn map_bytes(size: u32) -> u64 {
    3 * 4 * u64::from(size) * u64::from(size)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub size: u32,
    pub crop: Crop,
    pub uv_set: u32,
    pub base: Vec<u8>,
    pub roughness: Vec<u8>,
    pub normal: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Maps {
    pub size: u32,
    pub base: Vec<u8>,
    pub roughness: Vec<u8>,
    pub normal: Vec<u8>,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    if length > 1e-12 {
        v.map(|x| x / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mix3(values: [[f32; 3]; 3], weight: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| {
        (0..3)
            .map(|corner| values[corner][axis] * weight[corner])
            .sum()
    })
}

fn mix2(values: [[f32; 2]; 3], weight: [f32; 3]) -> [f32; 2] {
    std::array::from_fn(|axis| {
        (0..3)
            .map(|corner| values[corner][axis] * weight[corner])
            .sum()
    })
}

fn project(view: &View, position: [f32; 3]) -> Option<[f32; 2]> {
    let clip: [f32; 4] = std::array::from_fn(|axis| {
        view.matrix[3][axis]
            + (0..3)
                .map(|component| view.matrix[component][axis] * position[component])
                .sum::<f32>()
    });
    if clip[3] <= 0.0 || !clip[3].is_finite() {
        return None;
    }
    Some([
        (clip[0] / clip[3] * 0.5 + 0.5) * view.size[0] as f32,
        (0.5 - clip[1] / clip[3] * 0.5) * view.size[1] as f32,
    ])
}

pub fn resolution(input: &Input<'_>, view: &View, largest: u32) -> u32 {
    let largest = largest.clamp(FLOOR, LIMIT);
    let mut need = FLOOR as f32;
    for triangle in input.indices.chunks_exact(3) {
        let ids: [usize; 3] = std::array::from_fn(|corner| triangle[corner] as usize);
        let Some(screen) = ids
            .map(|id| project(view, input.positions[id]))
            .into_iter()
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        for edge in [(0, 1), (1, 2), (2, 0)] {
            let a = edge.0;
            let b = edge.1;
            let uv = input.raster()[ids[a]];
            let next = input.raster()[ids[b]];
            let du = ((uv[0] - next[0]).powi(2) + (uv[1] - next[1]).powi(2)).sqrt();
            let ds = ((screen[a][0] - screen[b][0]).powi(2)
                + (screen[a][1] - screen[b][1]).powi(2))
            .sqrt();
            if du > 1e-5 {
                need = need.max(ds / du);
            }
        }
    }
    (need.ceil() as u32)
        .clamp(FLOOR, largest)
        .next_power_of_two()
        .min(largest)
}

fn edge(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (p[0] - a[0]) * (b[1] - a[1]) - (p[1] - a[1]) * (b[0] - a[0])
}

fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn set_texel(map: &mut [u8], index: usize, value: [u8; 4]) {
    map[index * 4..index * 4 + 4].copy_from_slice(&value);
}

fn texel(map: &[u8], index: usize) -> [u8; 4] {
    map[index * 4..index * 4 + 4].try_into().unwrap()
}

pub fn repeats(input: &Input<'_>, size: u32) -> bool {
    let uvs = input.raster();
    if size == 0
        || uvs.len() != input.positions.len()
        || uvs.is_empty()
        || uvs
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || *v < -0.1 || *v > 1.1)
    {
        return true;
    }
    let mut owner = vec![u32::MAX; (size * size) as usize];
    for (number, triangle) in input.indices.chunks_exact(3).enumerate() {
        let Some(uv) = triangle
            .iter()
            .map(|&id| uvs.get(id as usize).copied())
            .collect::<Option<Vec<_>>>()
        else {
            return true;
        };
        let area = edge(uv[0], uv[1], uv[2]);
        if area.abs() < 1e-12 {
            continue;
        }
        let min: [u32; 2] = std::array::from_fn(|axis| {
            (uv.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min) * size as f32 - 0.5)
                .ceil()
                .max(0.0) as u32
        });
        let max: [u32; 2] = std::array::from_fn(|axis| {
            (uv.iter().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max) * size as f32 - 0.5)
                .floor()
                .min(size as f32 - 1.0) as u32
        });
        for y in min[1]..=max[1] {
            for x in min[0]..=max[0] {
                let at = [
                    (x as f32 + 0.5) / size as f32,
                    (y as f32 + 0.5) / size as f32,
                ];
                let weight = [
                    edge(uv[1], uv[2], at) / area,
                    edge(uv[2], uv[0], at) / area,
                    edge(uv[0], uv[1], at) / area,
                ];
                if weight.iter().all(|&v| v > 1e-5) {
                    let index = (y * size + x) as usize;
                    if owner[index] != u32::MAX && owner[index] != number as u32 {
                        return true;
                    }
                    owner[index] = number as u32;
                }
            }
        }
    }
    false
}

fn check_input(input: &Input<'_>, size: u32) -> Result<(), String> {
    if size == 0 || size > LIMIT || !size.is_power_of_two() {
        return Err(format!(
            "detail size must be a power of two through {LIMIT}"
        ));
    }
    if input.positions.len() != input.normals.len()
        || input.positions.len() != input.tangents.len()
        || input.positions.len() != input.uvs.len()
        || input.positions.len() != input.raster().len()
        || input
            .indices
            .iter()
            .any(|&i| i as usize >= input.positions.len())
    {
        return Err("detail mesh attributes are incomplete".into());
    }
    Ok(())
}

pub fn crop(input: &Input<'_>, size: u32, padding: u32) -> Result<Crop, String> {
    check_input(input, size)?;
    let scale = size as f32;
    let mut low = [i64::MAX; 2];
    let mut high = [i64::MIN; 2];
    for triangle in input.indices.chunks_exact(3) {
        let uv = [0, 1, 2].map(|corner| input.raster()[triangle[corner] as usize]);
        if edge(uv[0], uv[1], uv[2]).abs() < 1e-12 {
            continue;
        }
        let min = std::array::from_fn::<_, 2, _>(|axis| {
            (uv.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min) * scale - 0.5)
                .ceil()
                .max(0.0) as u32
        });
        let max = std::array::from_fn::<_, 2, _>(|axis| {
            (uv.iter().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max) * scale - 0.5)
                .floor()
                .min(scale - 1.0) as u32
        });
        if min[0] > max[0] || min[1] > max[1] {
            continue;
        }
        for axis in 0..2 {
            low[axis] = low[axis].min(i64::from(min[axis]));
            high[axis] = high[axis].max(i64::from(max[axis]));
        }
    }
    if low.iter().zip(&high).any(|(low, high)| low > high) {
        return Err("detail mesh has no UV coverage".into());
    }
    let align = i64::from(atlas::ALIGN);
    let start = low.map(|value| (value - i64::from(padding)).max(0) / align * align);
    let end = high.map(|value| {
        ((value + i64::from(padding) + 1 + align - 1) / align * align).min(i64::from(size))
    });
    Ok(Crop {
        x: start[0] as u32,
        y: start[1] as u32,
        width: (end[0] - start[0]) as u32,
        height: (end[1] - start[1]) as u32,
    })
}

pub fn bake(input: &Input<'_>, size: u32, padding: u32) -> Result<Maps, String> {
    let region = bake_crop(
        input,
        size,
        padding,
        Crop {
            x: 0,
            y: 0,
            width: size,
            height: size,
        },
    )?;
    Ok(Maps {
        size,
        base: region.base,
        roughness: region.roughness,
        normal: region.normal,
    })
}

pub fn bake_crop(input: &Input<'_>, size: u32, padding: u32, crop: Crop) -> Result<Region, String> {
    check_input(input, size)?;
    if crop.width == 0
        || crop.height == 0
        || crop.x + crop.width > size
        || crop.y + crop.height > size
    {
        return Err("detail crop lies outside the map".into());
    }
    let count = (crop.width * crop.height) as usize;
    let mut region = Region {
        size,
        crop,
        uv_set: input.uv_set(),
        base: vec![0; count * 4],
        roughness: vec![0; count * 4],
        normal: vec![0; count * 4],
    };
    let mut covered = vec![false; count];
    let scale = size as f32;
    for triangle in input.indices.chunks_exact(3) {
        let ids: [usize; 3] = std::array::from_fn(|corner| triangle[corner] as usize);
        let uv = ids.map(|id| input.raster()[id]);
        let area = edge(uv[0], uv[1], uv[2]);
        if area.abs() < 1e-12 {
            continue;
        }
        let min = std::array::from_fn::<_, 2, _>(|axis| {
            (uv.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min) * scale - 0.5)
                .ceil()
                .max(0.0) as u32
        });
        let max = std::array::from_fn::<_, 2, _>(|axis| {
            (uv.iter().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max) * scale - 0.5)
                .floor()
                .min(scale - 1.0) as u32
        });
        let min = [min[0].max(crop.x), min[1].max(crop.y)];
        let max = [
            max[0].min(crop.x + crop.width - 1),
            max[1].min(crop.y + crop.height - 1),
        ];
        for y in min[1]..=max[1] {
            for x in min[0]..=max[0] {
                let at_uv = [(x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale];
                let weight = [
                    edge(uv[1], uv[2], at_uv) / area,
                    edge(uv[2], uv[0], at_uv) / area,
                    edge(uv[0], uv[1], at_uv) / area,
                ];
                if weight.iter().any(|&value| value < -1e-5) {
                    continue;
                }
                let index = ((y - crop.y) * crop.width + x - crop.x) as usize;
                if covered[index] {
                    continue;
                }
                let position = mix3(ids.map(|id| input.positions[id]), weight);
                let normal = unit(mix3(ids.map(|id| input.normals[id]), weight));
                let tangent = ids.map(|id| input.tangents[id]);
                let tangent_xyz = unit(mix3(tangent.map(|v| [v[0], v[1], v[2]]), weight));
                let tangent_xyz = unit(sub(
                    tangent_xyz,
                    normal.map(|v| v * dot(normal, tangent_xyz)),
                ));
                let handedness: f32 = (0..3).map(|i| tangent[i][3] * weight[i]).sum();
                let bitangent = cross(normal, tangent_xyz).map(|v| v * handedness.signum());
                let surface_uv = match input.uvs1 {
                    Some(_) => mix2(ids.map(|id| input.uvs[id]), weight),
                    None => at_uv,
                };
                let resolved = resolve(
                    input.material,
                    0.0,
                    &At {
                        position,
                        uv: surface_uv,
                        normal,
                        edge: 0.0,
                        time: 0.0,
                    },
                );
                let mapped = [
                    dot(resolved.normal, tangent_xyz),
                    dot(resolved.normal, bitangent),
                    dot(resolved.normal, normal),
                ];
                set_texel(
                    &mut region.base,
                    index,
                    resolved.material.base.map(byte).map_with_alpha(),
                );
                let rough = byte(resolved.material.roughness);
                set_texel(&mut region.roughness, index, [rough, rough, rough, 255]);
                set_texel(
                    &mut region.normal,
                    index,
                    [
                        byte(mapped[0] * 0.5 + 0.5),
                        byte(mapped[1] * 0.5 + 0.5),
                        byte(mapped[2] * 0.5 + 0.5),
                        255,
                    ],
                );
                covered[index] = true;
            }
        }
    }
    if !covered.iter().any(|&value| value) {
        return Err("detail mesh has no UV coverage".into());
    }
    pad(&mut region, &mut covered, padding);
    Ok(region)
}

trait Alpha {
    fn map_with_alpha(self) -> [u8; 4];
}

impl Alpha for [u8; 3] {
    fn map_with_alpha(self) -> [u8; 4] {
        [self[0], self[1], self[2], 255]
    }
}

fn pad(region: &mut Region, covered: &mut [bool], padding: u32) {
    let width = region.crop.width as usize;
    let height = region.crop.height as usize;
    let mut queue = VecDeque::new();
    for (index, &filled) in covered.iter().enumerate() {
        if filled {
            queue.push_back((index, index, 0u32));
        }
    }
    while let Some((index, source, distance)) = queue.pop_front() {
        if distance >= padding {
            continue;
        }
        let x = index % width;
        let y = index / width;
        for next in [
            (x > 0).then_some(index.wrapping_sub(1)),
            (x + 1 < width).then_some(index + 1),
            (y > 0).then_some(index.wrapping_sub(width)),
            (y + 1 < height).then_some(index + width),
        ]
        .into_iter()
        .flatten()
        {
            if covered[next] {
                continue;
            }
            covered[next] = true;
            let base = texel(&region.base, source);
            let roughness = texel(&region.roughness, source);
            let normal = texel(&region.normal, source);
            set_texel(&mut region.base, next, base);
            set_texel(&mut region.roughness, next, roughness);
            set_texel(&mut region.normal, next, normal);
            queue.push_back((next, source, distance + 1));
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn input_hash(input: &Input<'_>, size: u32, padding: u32) -> Result<String, String> {
    let mut h = Sha256::new();
    h.update(b"pito-engine-detail-v1");
    h.update(env!("CARGO_PKG_VERSION"));
    h.update(include_str!("detail.rs"));
    h.update(size.to_le_bytes());
    h.update(padding.to_le_bytes());
    for position in input.positions {
        for value in position {
            h.update(value.to_bits().to_le_bytes());
        }
    }
    for normal in input.normals {
        for value in normal {
            h.update(value.to_bits().to_le_bytes());
        }
    }
    for tangent in input.tangents {
        for value in tangent {
            h.update(value.to_bits().to_le_bytes());
        }
    }
    for uv in input.uvs {
        for value in uv {
            h.update(value.to_bits().to_le_bytes());
        }
    }
    if let Some(uvs1) = input.uvs1 {
        h.update(b"uvs1");
        for uv in uvs1 {
            for value in uv {
                h.update(value.to_bits().to_le_bytes());
            }
        }
    }
    for index in input.indices {
        h.update(index.to_le_bytes());
    }
    h.update(serde_json::to_vec(input.material).map_err(|e| e.to_string())?);
    Ok(h.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn write(path: &Path, hash: &str, maps: &Maps) -> Result<(), String> {
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    let files = [
        ("base.rgba", &maps.base),
        ("roughness.rgba", &maps.roughness),
        ("normal.rgba", &maps.normal),
    ];
    let entries = files
        .iter()
        .map(|(name, data)| {
            fs::write(path.join(name), data).map_err(|e| e.to_string())?;
            Ok(json!({"path": name, "bytes": data.len(), "sha256": digest(data)}))
        })
        .collect::<Result<Vec<Value>, String>>()?;
    let manifest = serde_json::to_vec_pretty(&json!({
        "schema_version": 1,
        "input_hash": hash,
        "size": maps.size,
        "format": "rgba8-linear",
        "files": entries,
    }))
    .map_err(|e| e.to_string())?;
    fs::write(path.join("manifest.json"), &manifest).map_err(|e| e.to_string())?;
    fs::write(path.join("manifest.sha256"), digest(&manifest)).map_err(|e| e.to_string())?;
    read(path)?;
    Ok(())
}

pub fn read(path: &Path) -> Result<(String, Maps), String> {
    let manifest = fs::read(path.join("manifest.json")).map_err(|e| e.to_string())?;
    let expected = fs::read_to_string(path.join("manifest.sha256")).map_err(|e| e.to_string())?;
    if expected != digest(&manifest) {
        return Err("detail manifest checksum differs".into());
    }
    let value: Value = serde_json::from_slice(&manifest).map_err(|e| e.to_string())?;
    if value["schema_version"] != 1 || value["format"] != "rgba8-linear" {
        return Err("unsupported detail artifact".into());
    }
    let hash = value["input_hash"]
        .as_str()
        .ok_or("detail input hash is missing")?;
    let size = value["size"].as_u64().ok_or("detail size is missing")?;
    let size = u32::try_from(size).map_err(|_| "detail size is invalid")?;
    if size == 0 || size > LIMIT || !size.is_power_of_two() {
        return Err("detail size is invalid".into());
    }
    let entries = value["files"]
        .as_array()
        .ok_or("detail files are missing")?;
    if entries.len() != 3 {
        return Err("detail needs three maps".into());
    }
    let mut maps = Vec::new();
    for (entry, name) in entries
        .iter()
        .zip(["base.rgba", "roughness.rgba", "normal.rgba"])
    {
        if entry["path"] != name {
            return Err("detail map name differs".into());
        }
        let bytes = fs::read(path.join(name)).map_err(|e| e.to_string())?;
        if bytes.len() != (size * size * 4) as usize
            || entry["bytes"].as_u64() != Some(bytes.len() as u64)
            || entry["sha256"].as_str() != Some(digest(&bytes).as_str())
        {
            return Err(format!("detail {name} checksum differs"));
        }
        maps.push(bytes);
    }
    Ok((
        hash.to_owned(),
        Maps {
            size,
            base: maps.remove(0),
            roughness: maps.remove(0),
            normal: maps.remove(0),
        },
    ))
}

struct Candidate<'a> {
    index: u32,
    node: String,
    geometry: pfx_geom::mesh::Mesh,
    uvs1: Option<Vec<[f32; 2]>>,
    material: &'a Material,
    size: u32,
    hash: String,
}

impl Candidate<'_> {
    fn input(&self) -> Input<'_> {
        Input {
            positions: &self.geometry.positions,
            normals: &self.geometry.normals,
            tangents: &self.geometry.tangents,
            uvs: &self.geometry.uvs,
            uvs1: self.uvs1.as_deref(),
            indices: &self.geometry.indices,
            material: self.material,
        }
    }
}

pub fn report(atlas: &atlas::Atlas, summary: &atlas::Summary) -> Report {
    let parts: Vec<Part> = atlas
        .parts
        .iter()
        .map(|part| Part {
            path: format!("part-{:04}", part.index),
            node: part.node.clone(),
            size: part.density,
            crop: part.crop,
            page: part.page,
            bytes: atlas.part_bytes(part),
            input_hash: part.input_hash.clone(),
            uv_set: part.uv_set,
        })
        .collect();
    Report {
        full_bytes: parts.iter().map(|part| map_bytes(part.size)).sum(),
        parts,
        total_bytes: summary.total_bytes,
        vram_bytes: summary.vram_bytes,
        decoded_vram_bytes: atlas.decoded_vram_bytes(),
        page: [atlas.width, atlas.height],
        pages: atlas.pages(),
        psnr: summary.psnr,
    }
}

fn replaceable(path: &Path) -> Result<(), String> {
    let mut files = vec!["manifest.json".to_string(), "manifest.sha256".to_string()];
    let mut folders = Vec::new();
    if let Ok((_, atlas)) = atlas::read(path) {
        files.extend(atlas::file_names(atlas.pages() as usize));
    } else {
        atlas::load(path)?;
        let manifest: Value = serde_json::from_slice(
            &fs::read(path.join("manifest.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        for part in manifest["parts"].as_array().into_iter().flatten() {
            folders.push(part["path"].as_str().unwrap_or_default().to_owned());
        }
    }
    let part_files = [
        "manifest.json",
        "manifest.sha256",
        "base.rgba",
        "roughness.rgba",
        "normal.rgba",
    ];
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let expected = if kind.is_file() {
            files.contains(&name)
        } else if kind.is_dir() && folders.contains(&name) {
            fs::read_dir(entry.path())
                .map_err(|e| e.to_string())?
                .all(|inner| {
                    inner.is_ok_and(|inner| {
                        part_files.contains(&inner.file_name().to_string_lossy().as_ref())
                            && inner.file_type().is_ok_and(|kind| kind.is_file())
                    })
                })
        } else {
            false
        };
        if !expected {
            return Err(format!("{} contains non-artifact files", path.display()));
        }
    }
    Ok(())
}

fn mesh_node_name<'a>(mesh: &'a pfx_load::Mesh, node: &'a pfx_load::Node) -> Option<&'a str> {
    let group = node.group?;
    Some(if node.name.is_empty() {
        mesh.groups[group as usize].name.as_str()
    } else {
        node.name.as_str()
    })
}

pub fn bake_scene(
    folder: &Path,
    scene_file: &str,
    materials: &[Material],
    recipe: &crate::recipe::DetailRecipe,
    out: &Path,
    force: bool,
) -> Result<Report, String> {
    use pfx_geom::mesh::Mesh as Geometry;
    use pfx_load::Mesh;

    recipe.check()?;
    let view = recipe.view.as_ref().map(View::look);
    let root = folder.canonicalize().map_err(|e| e.to_string())?;
    let bytes = crate::recipe::read_inside(&root, scene_file)?;
    let animated: std::collections::BTreeSet<usize> = gltf::Gltf::from_slice(&bytes)
        .map_err(|e| e.to_string())?
        .animations()
        .flat_map(|animation| {
            animation
                .channels()
                .map(|channel| channel.target().node().index())
        })
        .collect();
    let mesh = Mesh::parse_with(&bytes, |uri| {
        let base = Path::new(scene_file).parent().unwrap_or(Path::new(""));
        let name = base.join(uri).to_string_lossy().into_owned();
        crate::recipe::read_inside(&root, &name)
    })?;
    if let Some(missing) = recipe.sizes.keys().find(|name| {
        !mesh
            .nodes
            .iter()
            .any(|node| mesh_node_name(&mesh, node) == Some(name.as_str()))
    }) {
        return Err(format!(
            "detail size names {missing:?}, which is not a mesh node"
        ));
    }
    if let Some(missing) = recipe.parts.iter().find(|name| {
        !mesh
            .nodes
            .iter()
            .any(|node| mesh_node_name(&mesh, node) == Some(name.as_str()))
    }) {
        return Err(format!(
            "detail parts names {missing:?}, which is not a mesh node"
        ));
    }
    let mut candidates = Vec::new();
    let mut stack: Vec<u32> = mesh.roots.iter().rev().copied().collect();
    let mut part_index = 0u32;
    while let Some(index) = stack.pop() {
        let node = mesh
            .nodes
            .get(index as usize)
            .ok_or("detail node is missing")?;
        stack.extend(node.children.iter().rev().copied());
        let Some(group_index) = node.group else {
            continue;
        };
        let group = &mesh.groups[group_index as usize];
        let name = if node.name.is_empty() {
            &group.name
        } else {
            &node.name
        };
        let world = mesh.world(index).ok_or("invalid detail node transform")?;
        for primitive in &group.primitives {
            let this_part = part_index;
            part_index += 1;
            let mut ancestor = Some(index);
            let mut animated_part = false;
            while let Some(node_index) = ancestor {
                if animated.contains(&(node_index as usize)) {
                    animated_part = true;
                    break;
                }
                ancestor = mesh.nodes[node_index as usize].parent;
            }
            if animated_part || recipe.dynamic_nodes.iter().any(|dynamic| dynamic == name) {
                println!("detail bake: {name} keeps procedural detail because it moves");
                continue;
            }
            if !recipe.parts.is_empty() && recipe.parts.iter().all(|part| part != name) {
                continue;
            }
            let material = materials
                .get(primitive.material.unwrap_or(materials.len() as u32 - 1) as usize)
                .ok_or("detail material is missing")?;
            if !material.layers.iter().any(|layer| layer.amplitude != 0.0) {
                continue;
            }
            let positions: Vec<[f32; 3]> = primitive
                .positions
                .iter()
                .map(|point| {
                    std::array::from_fn(|axis| {
                        world[3][axis] + (0..3).map(|k| world[k][axis] * point[k]).sum::<f32>()
                    })
                })
                .collect();
            let a = |row: usize, column: usize| world[column][row];
            let cofactor = |row: usize, column: usize| {
                let (r0, r1) = ((row + 1) % 3, (row + 2) % 3);
                let (c0, c1) = ((column + 1) % 3, (column + 2) % 3);
                a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0)
            };
            let determinant: f32 = (0..3)
                .map(|column| a(0, column) * cofactor(0, column))
                .sum();
            let sign = if determinant < 0.0 { -1.0 } else { 1.0 };
            let normals: Vec<[f32; 3]> = primitive
                .normals
                .iter()
                .map(|normal| {
                    let transformed: [f32; 3] = std::array::from_fn(|row| {
                        (0..3)
                            .map(|column| cofactor(row, column) * normal[column] * sign)
                            .sum()
                    });
                    let length = transformed.iter().map(|v| v * v).sum::<f32>().sqrt();
                    transformed.map(|v| v / length.max(1e-20))
                })
                .collect();
            let normals = if normals.is_empty() {
                let mut flat = vec![[0.0; 3]; positions.len()];
                for triangle in primitive.indices.chunks_exact(3) {
                    let [a, b, c] =
                        [triangle[0], triangle[1], triangle[2]].map(|id| positions[id as usize]);
                    let face = cross(sub(b, a), sub(c, a));
                    for &id in triangle {
                        for axis in 0..3 {
                            flat[id as usize][axis] += face[axis];
                        }
                    }
                }
                flat.into_iter().map(unit).collect()
            } else {
                normals
            };
            let geometry = Geometry::new(
                positions,
                normals,
                Vec::new(),
                primitive.uvs.clone(),
                primitive.indices.clone(),
            );
            let mut candidate = Candidate {
                index: this_part,
                node: name.to_owned(),
                geometry,
                uvs1: primitive.uvs1.clone(),
                material,
                size: 0,
                hash: String::new(),
            };
            let input = candidate.input();
            let size = match (recipe.sizes.get(name), &view) {
                (Some(&size), _) => size,
                (None, Some(view)) => resolution(&input, view, recipe.size),
                (None, None) => recipe.size,
            };
            if recipe.padding > size / 4 {
                return Err(format!(
                    "detail padding {} is more than a quarter of {name}'s {size}² map",
                    recipe.padding
                ));
            }
            if repeats(&input, size) {
                println!(
                    "detail bake: {name} keeps procedural detail because its UVs repeat or are missing"
                );
                continue;
            }
            if input.uvs1.is_some() {
                println!("detail bake: {name} bakes through TEXCOORD_1");
            }
            let hash = input_hash(&input, size, recipe.padding)?;
            candidate.size = size;
            candidate.hash = hash;
            candidates.push(candidate);
        }
    }
    if candidates.is_empty() {
        println!("detail bake: no static part with unique UVs has procedural detail to bake");
        return Ok(Report {
            parts: Vec::new(),
            total_bytes: 0,
            vram_bytes: 0,
            decoded_vram_bytes: 0,
            full_bytes: 0,
            page: [0, 0],
            pages: 0,
            psnr: [None; 3],
        });
    }
    let keys: Vec<(u32, &str, &str)> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.index,
                candidate.node.as_str(),
                candidate.hash.as_str(),
            )
        })
        .collect();
    let hash = atlas::hash(&keys, recipe.page, recipe.padding);
    if out.exists() {
        if let Ok((summary, saved)) = atlas::read(out)
            && summary.input_hash == hash
        {
            println!(
                "detail bake: {} unchanged, {} parts on {} pages",
                out.display(),
                saved.parts.len(),
                saved.pages()
            );
            return Ok(report(&saved, &summary));
        }
        if !force {
            return Err(format!(
                "{} already exists; use --force to replace it",
                out.display()
            ));
        }
        replaceable(out)?;
    }
    let mut pieces = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let input = candidate.input();
        let crop = crop(&input, candidate.size, recipe.padding)?;
        let region = bake_crop(&input, candidate.size, recipe.padding, crop)?;
        println!(
            "detail bake: {} part-{:04} {}² crop {}×{} at ({}, {}) through TEXCOORD_{}",
            candidate.node,
            candidate.index,
            candidate.size,
            crop.width,
            crop.height,
            crop.x,
            crop.y,
            input.uv_set()
        );
        pieces.push(atlas::Piece {
            index: candidate.index,
            node: candidate.node.clone(),
            input_hash: candidate.hash.clone(),
            region,
        });
    }
    let plain = atlas::assemble(pieces, recipe.page)?;
    let packed = plain.compress()?;
    let psnr = atlas::Kind::ALL.map(|kind| atlas::psnr(&plain, &packed, kind));
    let name = out
        .file_name()
        .ok_or("detail output needs a folder name")?
        .to_string_lossy()
        .into_owned();
    let parent = out.parent().unwrap_or(Path::new(""));
    let next = parent.join(format!(".{name}.next"));
    let old = parent.join(format!(".{name}.old"));
    for path in [&next, &old] {
        if path.exists() {
            return Err(format!("{} already exists", path.display()));
        }
    }
    let summary = atlas::write(&next, &packed, &hash, recipe.padding, psnr)?;
    atlas::read(&next)?;
    if out.exists() {
        fs::rename(out, &old).map_err(|e| e.to_string())?;
        if let Err(error) = fs::rename(&next, out) {
            let _ = fs::rename(&old, out);
            return Err(error.to_string());
        }
        fs::remove_dir_all(&old).map_err(|e| e.to_string())?;
    } else {
        fs::rename(&next, out).map_err(|e| e.to_string())?;
    }
    println!(
        "detail bake: {} parts on {} pages of {}×{}",
        packed.parts.len(),
        packed.pages(),
        packed.width,
        packed.height
    );
    Ok(report(&packed, &summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_materials::{NoiseKind, NoiseLayer};

    fn mesh<'a>(material: &'a Material, uvs: &'a [[f32; 2]; 3]) -> Input<'a> {
        static POSITIONS: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        static NORMALS: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
        static TANGENTS: [[f32; 4]; 3] = [[1.0, 0.0, 0.0, 1.0]; 3];
        static INDICES: [u32; 3] = [0, 1, 2];
        Input {
            positions: &POSITIONS,
            normals: &NORMALS,
            tangents: &TANGENTS,
            uvs,
            uvs1: None,
            indices: &INDICES,
            material,
        }
    }

    #[test]
    fn baked_texel_matches_procedural_evaluation() {
        let mut material = Material::default();
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 8.0, 0.7, 19);
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let input = mesh(&material, &uvs);
        let maps = bake(&input, 8, 0).unwrap();
        let uv = [1.5 / 8.0, 1.5 / 8.0];
        let resolved = resolve(
            &material,
            0.0,
            &At {
                position: [uv[0], uv[1], 0.0],
                uv,
                normal: [0.0, 0.0, 1.0],
                edge: 0.0,
                time: 0.0,
            },
        );
        assert_eq!(texel(&maps.base, 9)[..3], resolved.material.base.map(byte));
        assert_eq!(
            texel(&maps.roughness, 9)[0],
            byte(resolved.material.roughness)
        );
    }

    #[test]
    fn padding_extends_covered_texels_without_crossing_the_limit() {
        let material = Material::default();
        let uvs = [[0.125, 0.125], [0.375, 0.125], [0.125, 0.375]];
        let maps = bake(&mesh(&material, &uvs), 8, 1).unwrap();
        assert_eq!(texel(&maps.base, 8), texel(&maps.base, 9));
        assert_eq!(texel(&maps.normal, 8), texel(&maps.normal, 9));
        assert_eq!(texel(&maps.base, 63), [0; 4]);
    }

    #[test]
    fn overlapping_uv_islands_remain_procedural() {
        let material = Material::default();
        let positions = [[0.0, 0.0, 0.0]; 6];
        let normals = [[0.0, 0.0, 1.0]; 6];
        let tangents = [[1.0, 0.0, 0.0, 1.0]; 6];
        let uvs = [
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
        ];
        let input = Input {
            positions: &positions,
            normals: &normals,
            tangents: &tangents,
            uvs: &uvs,
            uvs1: None,
            indices: &[0, 1, 2, 3, 4, 5],
            material: &material,
        };
        assert!(repeats(&input, 16));
    }

    #[test]
    fn scene_bake_writes_checked_maps_and_skips_repeated_uvs() {
        use pfx_materials::{NoiseKind, NoiseLayer};

        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/data");
        let root = Path::new("tmp").join(format!("detail-scene-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        fs::copy(source.join("triangle.gltf"), root.join("triangle.gltf")).unwrap();
        fs::copy(source.join("triangle.bin"), root.join("triangle.bin")).unwrap();
        let mut material = Material::default();
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 5.0, 0.1, 7);
        let recipe = crate::recipe::DetailRecipe {
            size: 64,
            padding: 2,
            dynamic_nodes: Vec::new(),
            parts: Vec::new(),
            sizes: Default::default(),
            view: None,
            page: atlas::PAGE,
        };
        let out = root.join("out");
        let report = bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, false).unwrap();
        assert_eq!(report.parts.len(), 1);
        assert_eq!(report.full_bytes, 3 * 4 * 64 * 64);
        assert_eq!(report.page, [64, 64]);
        assert_eq!(report.pages, 1);
        let levels: u64 = (0..atlas::LEVELS)
            .map(|level| (64u64 >> level).pow(2))
            .sum();
        assert_eq!(report.vram_bytes, levels * 5 / 2);
        let first = atlas::read(&out).unwrap();
        assert_eq!(
            first.1.parts[0].crop,
            Crop {
                x: 0,
                y: 0,
                width: 64,
                height: 64
            }
        );
        assert!(first.1.compressed());
        assert_eq!(first.1.parts[0].uv_set, 0);
        assert_eq!(report.parts[0].uv_set, 0);
        assert_eq!(first.1.schema(), atlas::SCHEMA);
        let again = bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, false).unwrap();
        assert_eq!(again, report);
        assert_eq!(first, atlas::read(&out).unwrap());
        let mut bin = fs::read(root.join("triangle.bin")).unwrap();
        bin[120..124].copy_from_slice(&2.0f32.to_le_bytes());
        fs::write(root.join("triangle.bin"), bin).unwrap();
        let none = bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, false).unwrap();
        assert!(none.parts.is_empty());
        assert_eq!(first, atlas::read(&out).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    const SECOND: [[f32; 2]; 3] = [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75]];

    #[test]
    fn a_part_with_a_second_uv_set_bakes_through_it() {
        let mut material = Material::default();
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 8.0, 0.7, 19);
        let tiled = [[-3.0, 0.0], [5.0, 0.0], [-3.0, 8.0]];
        let first = mesh(&material, &tiled);
        let second = Input {
            uvs1: Some(&SECOND),
            ..mesh(&material, &tiled)
        };
        assert!(repeats(&first, 64));
        assert!(!repeats(&second, 64));
        assert_eq!((first.uv_set(), second.uv_set()), (0, 1));
        assert_eq!(second.raster(), &SECOND[..]);
        let alone = mesh(&material, &SECOND);
        assert_eq!(
            crop(&second, 256, 2).unwrap(),
            crop(&alone, 256, 2).unwrap()
        );
        assert_eq!(
            crop(&second, 256, 2).unwrap(),
            Crop {
                x: 48,
                y: 48,
                width: 160,
                height: 160
            }
        );
        assert_ne!(
            input_hash(&second, 64, 2).unwrap(),
            input_hash(&first, 64, 2).unwrap()
        );
        let whole = Crop {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let region = bake_crop(&second, 64, 0, whole).unwrap();
        assert_eq!(region.uv_set, 1);
        assert_eq!(bake_crop(&alone, 64, 0, whole).unwrap().uv_set, 0);
        let (x, y) = (25u32, 20u32);
        let at = [(x as f32 + 0.5) / 64.0, (y as f32 + 0.5) / 64.0];
        let area = edge(SECOND[0], SECOND[1], SECOND[2]);
        let weight = [
            edge(SECOND[1], SECOND[2], at) / area,
            edge(SECOND[2], SECOND[0], at) / area,
            edge(SECOND[0], SECOND[1], at) / area,
        ];
        assert!(weight.iter().all(|&w| w > 0.0));
        let resolved = resolve(
            &material,
            0.0,
            &At {
                position: mix3([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], weight),
                uv: mix2(tiled, weight),
                normal: [0.0, 0.0, 1.0],
                edge: 0.0,
                time: 0.0,
            },
        );
        let index = (y * 64 + x) as usize;
        assert_eq!(
            texel(&region.base, index)[..3],
            resolved.material.base.map(byte)
        );
        assert_eq!(
            texel(&region.roughness, index)[0],
            byte(resolved.material.roughness)
        );
        assert_eq!(texel(&region.base, 63 * 64 + 63), [0; 4]);
    }

    fn with_second_uvs(bin: &[u8], json: &str) -> (Vec<u8>, String) {
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let offset = bin.len();
        for value in SECOND.iter().flatten() {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let json = json
            .replace("\"byteLength\":150}]", &format!("\"byteLength\":{}}}]", bin.len()))
            .replace("\"TEXCOORD_0\":3}", "\"TEXCOORD_0\":3,\"TEXCOORD_1\":5}")
            .replace(
                "\"type\":\"SCALAR\"}]",
                "\"type\":\"SCALAR\"},{\"bufferView\":5,\"componentType\":5126,\"count\":3,\"type\":\"VEC2\"}]",
            )
            .replace(
                "\"byteLength\":6}]",
                &format!("\"byteLength\":6}},{{\"buffer\":0,\"byteOffset\":{offset},\"byteLength\":24}}]"),
            );
        (bin, json)
    }

    #[test]
    fn a_scene_part_with_texcoord_1_bakes_through_it_and_the_manifest_says_so() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/data");
        let root = Path::new("tmp").join(format!("detail-uv1-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        let (bin, json) = with_second_uvs(
            &fs::read(source.join("triangle.bin")).unwrap(),
            &fs::read_to_string(source.join("triangle.gltf")).unwrap(),
        );
        assert!(json.contains("TEXCOORD_1") && json.contains(&format!("{}", bin.len())));
        fs::write(root.join("triangle.bin"), &bin).unwrap();
        fs::write(root.join("triangle.gltf"), &json).unwrap();
        let mut material = Material::default();
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 5.0, 0.1, 7);
        let recipe = crate::recipe::DetailRecipe {
            size: 256,
            padding: 2,
            dynamic_nodes: Vec::new(),
            parts: Vec::new(),
            sizes: Default::default(),
            view: None,
            page: atlas::PAGE,
        };
        let out = root.join("out");
        let report = bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, false).unwrap();
        assert_eq!(report.parts.len(), 1);
        assert_eq!(report.parts[0].uv_set, 1);
        assert_eq!(
            report.parts[0].crop,
            Crop {
                x: 48,
                y: 48,
                width: 160,
                height: 160
            }
        );
        let (_, atlas) = atlas::read(&out).unwrap();
        assert_eq!(atlas.parts[0].uv_set, 1);
        assert_eq!(atlas.schema(), atlas::SCHEMA_UV1);
        let manifest: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["schema_version"], atlas::SCHEMA_UV1);
        assert_eq!(manifest["parts"][0]["uv_set"], 1);
        fs::remove_dir_all(root).unwrap();
    }

    fn camera(distance: f32) -> crate::recipe::DetailView {
        facing([1.5, 2.5, 3.0], distance)
    }

    fn facing(target: [f32; 3], distance: f32) -> crate::recipe::DetailView {
        crate::recipe::DetailView {
            position: [target[0], target[1], target[2] + distance],
            target,
            fov_y_deg: 60.0,
            shift: [0.0, 0.0],
            width: 1000,
            height: 1000,
        }
    }

    #[test]
    fn the_view_projects_the_target_to_the_middle_and_the_shift_moves_it() {
        let mut look = facing([0.5, 0.5, 0.0], 2.0);
        let view = View::look(&look);
        let [x, y] = project(&view, look.target).unwrap();
        assert!(
            (x - 500.0).abs() < 1e-2 && (y - 500.0).abs() < 1e-2,
            "{x} {y}"
        );
        look.shift = [0.2, -0.5];
        let [x, y] = project(&View::look(&look), look.target).unwrap();
        assert!(
            (x - 400.0).abs() < 1e-2 && (y - 250.0).abs() < 1e-2,
            "{x} {y}"
        );
        assert!(project(&view, [0.5, 0.5, 4.0]).is_none());
    }

    #[test]
    fn the_footprint_picks_a_power_of_two_between_the_floor_and_the_largest() {
        let material = Material::default();
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let input = mesh(&material, &uvs);
        let at = [0.5, 0.5, 0.0];
        let near = View::look(&facing(at, 0.5));
        let closest = View::look(&facing(at, 0.2));
        let far = View::look(&facing(at, 200.0));
        let middle = View::look(&facing(at, 5.0));
        assert_eq!(resolution(&input, &near, 2048), 2048);
        assert_eq!(resolution(&input, &near, 256), 256);
        assert_eq!(resolution(&input, &far, 2048), FLOOR);
        let fit = resolution(&input, &middle, 4096);
        assert!(
            fit.is_power_of_two() && (FLOOR..4096).contains(&fit),
            "{fit}"
        );
        assert!(fit > resolution(&input, &far, 4096));
        assert_eq!(resolution(&input, &near, 4096), 2048);
        assert_eq!(resolution(&input, &closest, 4096), 4096);
        assert_eq!(resolution(&input, &closest, 99999), 4096);
    }

    #[test]
    fn each_part_takes_its_footprint_unless_a_node_is_named() {
        use pfx_materials::{NoiseKind, NoiseLayer};

        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/data");
        let root = Path::new("tmp").join(format!("detail-sizes-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        fs::copy(source.join("triangle.gltf"), root.join("triangle.gltf")).unwrap();
        fs::copy(source.join("triangle.bin"), root.join("triangle.bin")).unwrap();
        let mut material = Material::default();
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 5.0, 0.1, 7);
        let mut recipe = crate::recipe::DetailRecipe {
            size: 256,
            padding: 2,
            dynamic_nodes: Vec::new(),
            parts: Vec::new(),
            sizes: Default::default(),
            view: Some(camera(200.0)),
            page: atlas::PAGE,
        };
        let out = root.join("out");
        let far = bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, false).unwrap();
        assert_eq!(far.parts.len(), 1);
        assert_eq!(far.parts[0].size, FLOOR);
        assert_eq!(far.parts[0].node, "triangle");
        assert_eq!(far.full_bytes, map_bytes(FLOOR));
        let manifest = fs::read(out.join("manifest.json")).unwrap();
        assert_eq!(
            fs::read_to_string(out.join("manifest.sha256")).unwrap(),
            digest(&manifest)
        );
        let value: Value = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["total_bytes"], far.total_bytes);
        assert_eq!(value["parts"][0]["size"], FLOOR);
        assert_eq!(value["parts"][0]["node"], "triangle");
        assert_eq!(value["maps"]["base"], "bc7-srgb");

        recipe.sizes.insert("triangle".into(), 128);
        assert!(
            bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, false)
                .unwrap_err()
                .contains("already exists")
        );
        let forced = bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, true).unwrap();
        assert_eq!(forced.parts[0].size, 128);
        assert_eq!(forced.full_bytes, map_bytes(128));
        assert_eq!(atlas::read(&out).unwrap().1.parts[0].density, 128);

        recipe.view = None;
        recipe.sizes.clear();
        let flat = bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, true).unwrap();
        assert_eq!(flat.parts[0].size, 256);

        recipe.sizes.insert("missing".into(), 128);
        assert!(
            bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, true)
                .unwrap_err()
                .contains("not a mesh node")
        );
        recipe.sizes.clear();
        recipe.padding = 40;
        recipe.view = Some(camera(200.0));
        recipe.size = 256;
        assert!(
            bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, true)
                .unwrap_err()
                .contains("padding")
        );
        recipe.padding = 2;
        recipe.view = None;
        recipe.sizes.insert("triangle".into(), 100);
        assert!(bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, true).is_err());
        recipe.sizes.insert("triangle".into(), 8192);
        assert!(bake_scene(&root, "triangle.gltf", &[material], &recipe, &out, true).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_recipe_reads_a_view_and_sizes_and_refuses_bad_ones() {
        let parsed: crate::recipe::DetailRecipe = toml::from_str(
            "size = 2048\n[sizes]\n\"mv_knob\" = 256\n[view]\nposition = [0.0, 0.1, 1.6]\ntarget = [0.0, 0.1, 0.0]\nfov_y_deg = 13.4\nshift = [0.0, -0.64]\n",
        )
        .unwrap();
        parsed.check().unwrap();
        assert_eq!(parsed.sizes["mv_knob"], 256);
        let view = parsed.view.unwrap();
        assert_eq!((view.width, view.height), (3840, 2160));
        let bad = |text: &str| {
            toml::from_str::<crate::recipe::DetailRecipe>(text)
                .unwrap()
                .check()
                .is_err()
        };
        assert!(bad("size = 32"));
        assert!(bad("size = 8192"));
        assert!(bad("size = 1000"));
        assert!(bad("size = 1024\n[sizes]\nknob = 48"));
        assert!(bad("size = 1024\n[sizes]\nknob = 8192"));
        assert!(bad(
            "size = 1024\n[view]\nposition = [0.0, 0.0, 1.0]\ntarget = [0.0, 0.0, 1.0]\nfov_y_deg = 30.0"
        ));
        assert!(bad(
            "size = 1024\n[view]\nposition = [0.0, 0.0, 1.0]\ntarget = [0.0, 0.0, 0.0]\nfov_y_deg = 0.0"
        ));
        assert!(toml::from_str::<crate::recipe::DetailRecipe>("size = 64\nsizess = 1").is_err());
        let listed: crate::recipe::DetailRecipe =
            toml::from_str("size = 64\nparts = [\"knob\", \"lip\"]\n").unwrap();
        assert_eq!(listed.parts, ["knob", "lip"]);
        listed.check().unwrap();
        assert!(parsed.parts.is_empty());
        assert!(
            toml::from_str::<crate::recipe::DetailRecipe>("size = 64\nparts = [\"\"]\n")
                .unwrap()
                .check()
                .unwrap_err()
                .contains("parts")
        );
    }

    fn folder_bytes(path: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut files = std::collections::BTreeMap::new();
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                let name = entry.file_name().into_string().unwrap();
                files.insert(name, fs::read(entry.path()).unwrap());
            }
        }
        files
    }

    #[test]
    fn parts_bakes_exactly_the_named_nodes_and_matches_skipping_the_rest() {
        use pfx_materials::{NoiseKind, NoiseLayer};

        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/data");
        let root = Path::new("tmp").join(format!("detail-parts-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        fs::copy(source.join("triangle.bin"), root.join("triangle.bin")).unwrap();
        let json = fs::read_to_string(source.join("triangle.gltf"))
            .unwrap()
            .replace(r#""children":[1]"#, r#""children":[1,2]"#)
            .replace(
                r#"{"name":"triangle","mesh":0,"translation":[1,2,3]}]"#,
                r#"{"name":"triangle","mesh":0,"translation":[1,2,3]},{"name":"wedge","mesh":0,"translation":[4,0,0]}]"#,
            );
        assert!(json.contains("wedge") && json.contains("[1,2]"));
        fs::write(root.join("triangle.gltf"), json).unwrap();
        let mut material = Material::default();
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 5.0, 0.1, 7);
        let mut recipe = crate::recipe::DetailRecipe {
            size: 64,
            padding: 2,
            dynamic_nodes: Vec::new(),
            parts: Vec::new(),
            sizes: Default::default(),
            view: None,
            page: 64,
        };
        let bake = |recipe: &crate::recipe::DetailRecipe, name: &str| {
            let out = root.join(name);
            let report =
                bake_scene(&root, "triangle.gltf", &[material], recipe, &out, false).unwrap();
            (report, folder_bytes(&out))
        };
        recipe.parts = vec!["triangle".into()];
        let (named, named_bytes) = bake(&recipe, "named");
        recipe.parts.clear();
        recipe.dynamic_nodes = vec!["wedge".into()];
        let (_, skipped_bytes) = bake(&recipe, "skipped");
        assert_eq!(named.parts.len(), 1);
        assert_eq!(named.parts[0].node, "triangle");
        assert_eq!(named_bytes, skipped_bytes);
        assert_eq!(
            atlas::read(&root.join("named")).unwrap().1.parts[0].index,
            0
        );

        recipe.dynamic_nodes.clear();
        recipe.parts = vec!["wedge".into()];
        let (wedge, wedge_bytes) = bake(&recipe, "wedge");
        recipe.parts.clear();
        recipe.dynamic_nodes = vec!["triangle".into()];
        let (_, triangle_skipped) = bake(&recipe, "triangle-skipped");
        assert_eq!(wedge.parts.len(), 1);
        assert_eq!(wedge.parts[0].node, "wedge");
        assert_eq!(wedge_bytes, triangle_skipped);
        assert_eq!(
            atlas::read(&root.join("wedge")).unwrap().1.parts[0].index,
            1
        );

        recipe.dynamic_nodes.clear();
        recipe.parts = vec!["wedge".into(), "triangle".into()];
        let (both, both_bytes) = bake(&recipe, "both");
        recipe.parts = vec!["triangle".into(), "triangle".into(), "wedge".into()];
        let (_, repeated) = bake(&recipe, "repeated");
        recipe.parts.clear();
        let (all, all_bytes) = bake(&recipe, "all");
        assert_eq!(both.parts.len(), 2);
        assert_eq!(all.parts.len(), 2);
        assert_eq!(both_bytes, all_bytes);
        assert_eq!(repeated, both_bytes);
        let nodes: Vec<&str> = all.parts.iter().map(|part| part.node.as_str()).collect();
        assert_eq!(nodes, ["triangle", "wedge"]);

        recipe.parts = vec!["missing".into()];
        assert!(
            bake_scene(
                &root,
                "triangle.gltf",
                &[material],
                &recipe,
                &root.join("missing"),
                false
            )
            .unwrap_err()
            .contains("not a mesh node")
        );
        recipe.parts = vec!["root".into()];
        assert!(
            bake_scene(
                &root,
                "triangle.gltf",
                &[material],
                &recipe,
                &root.join("root-part"),
                false
            )
            .unwrap_err()
            .contains("not a mesh node")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_crop_is_the_uv_box_with_padding_on_the_alignment_and_bakes_like_the_tile() {
        let mut material = Material::default();
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 9.0, 0.6, 3);
        let uvs = [[0.30, 0.55], [0.45, 0.55], [0.30, 0.70]];
        let input = mesh(&material, &uvs);
        let boxed = crop(&input, 256, 8).unwrap();
        assert_eq!(
            boxed,
            Crop {
                x: 64,
                y: 128,
                width: 64,
                height: 64
            }
        );
        for value in [boxed.x, boxed.y, boxed.width, boxed.height] {
            assert_eq!(value % atlas::ALIGN, 0);
        }
        let full = bake(&input, 256, 8).unwrap();
        let region = bake_crop(&input, 256, 8, boxed).unwrap();
        for y in 0..256u32 {
            for x in 0..256u32 {
                let tile = texel(&full.base, (y * 256 + x) as usize);
                let inside = (boxed.x..boxed.x + boxed.width).contains(&x)
                    && (boxed.y..boxed.y + boxed.height).contains(&y);
                if !inside {
                    assert_eq!(tile[3], 0, "texel {x},{y} outside the crop is covered");
                    continue;
                }
                let at = ((y - boxed.y) * boxed.width + x - boxed.x) as usize;
                assert_eq!(texel(&region.base, at), tile);
                assert_eq!(
                    texel(&region.normal, at),
                    texel(&full.normal, (y * 256 + x) as usize)
                );
                assert_eq!(
                    texel(&region.roughness, at),
                    texel(&full.roughness, (y * 256 + x) as usize)
                );
            }
        }
        let covered = atlas::covered_crop(256, &full.base).unwrap();
        assert!(covered.x >= boxed.x && covered.x + covered.width <= boxed.x + boxed.width);
        let edge_uvs = [[0.9, 0.9], [1.0, 0.9], [0.9, 1.0]];
        let edge = crop(&mesh(&material, &edge_uvs), 64, 8).unwrap();
        assert_eq!(
            edge,
            Crop {
                x: 48,
                y: 48,
                width: 16,
                height: 16
            }
        );
        let whole = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        assert_eq!(
            crop(&mesh(&material, &whole), 64, 2).unwrap(),
            Crop {
                x: 0,
                y: 0,
                width: 64,
                height: 64
            }
        );
        let flat = [[0.2, 0.2], [0.2, 0.2], [0.2, 0.2]];
        assert!(crop(&mesh(&material, &flat), 64, 2).is_err());
        assert!(
            bake_crop(
                &input,
                256,
                8,
                Crop {
                    x: 240,
                    y: 0,
                    width: 32,
                    height: 16
                }
            )
            .is_err()
        );
    }

    #[test]
    fn a_detail_folder_of_the_first_format_still_loads_as_an_atlas() {
        let mut material = Material::default();
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 6.0, 0.5, 11);
        let uvs = [[0.1, 0.1], [0.4, 0.1], [0.1, 0.3]];
        let input = mesh(&material, &uvs);
        let maps = bake(&input, 128, 4).unwrap();
        let hash = input_hash(&input, 128, 4).unwrap();
        let root = Path::new("tmp").join(format!("detail-v1-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        write(&root.join("part-0003"), &hash, &maps).unwrap();
        let summary = serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "total_bytes": map_bytes(128),
            "parts": [{
                "path": "part-0003",
                "node": "sheet",
                "size": 128,
                "bytes": map_bytes(128),
                "input_hash": hash,
            }],
        }))
        .unwrap();
        fs::write(root.join("manifest.json"), &summary).unwrap();
        fs::write(root.join("manifest.sha256"), digest(&summary)).unwrap();
        let loaded = atlas::load(&root).unwrap();
        assert_eq!(loaded.parts.len(), 1);
        let part = &loaded.parts[0];
        assert_eq!(
            (part.index, part.node.as_str(), part.density),
            (3, "sheet", 128)
        );
        let crop = part.crop;
        assert_eq!(Some(crop), atlas::covered_crop(128, &maps.base));
        let region = atlas::cut(&maps, crop);
        let expected = atlas::chain(atlas::Kind::Base, crop.width, crop.height, &region.base, 1);
        let page = &loaded.base[0].levels[0];
        for y in 0..crop.height {
            let from = (y * crop.width * 4) as usize;
            let to = (((part.at[1] + y) * loaded.width + part.at[0]) * 4) as usize;
            let row = (crop.width * 4) as usize;
            assert_eq!(page[to..to + row], expected[0][from..from + row]);
        }
        assert!(!loaded.compressed());
        let replaced = root.join("manifest.json");
        let mut bytes = fs::read(&replaced).unwrap();
        bytes[0] = b' ';
        fs::write(&replaced, bytes).unwrap();
        assert!(atlas::load(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_checks_each_payload() {
        let material = Material::default();
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let input = mesh(&material, &uvs);
        let maps = bake(&input, 8, 2).unwrap();
        let hash = input_hash(&input, 8, 2).unwrap();
        let root = Path::new("tmp").join(format!("detail-artifact-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        write(&root, &hash, &maps).unwrap();
        assert_eq!(read(&root).unwrap(), (hash, maps));
        let file = root.join("base.rgba");
        let mut bytes = fs::read(&file).unwrap();
        bytes[0] ^= 1;
        fs::write(file, bytes).unwrap();
        assert!(read(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
