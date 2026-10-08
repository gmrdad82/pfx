use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use pfx_bake::detail::{
    self,
    atlas::{self, Atlas},
};
use pfx_gpu::{Gpu, wgpu};
use pfx_live::frame;
use pfx_live::maps::{MapImages, select_baked_detail, uploads_blocks};
use pfx_load::{ColorSpace, Image, Pixels};
use pfx_materials::Material;

use super::room::Room;
use super::{Bench, Lens, SETTLE_FRAMES, Shown, dot, manners, point, root, save_png, unit};

pub const PADDING: u32 = 8;
const LARGEST: u32 = 2048;

pub struct Part {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub uvs1: Option<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
    pub material: Material,
    pub name: String,
}

pub struct Baked {
    pub layers: Vec<Option<u32>>,
    pub atlas: Atlas,
    pub objects: BTreeSet<u32>,
    pub check: Option<Check>,
}

pub struct Check {
    plain: Atlas,
    tiles: Option<[Vec<Image>; 3]>,
}

fn normal_matrix(m: &[[f32; 4]; 4]) -> [[f32; 3]; 3] {
    let a = |row: usize, column: usize| m[column][row];
    let cofactor = |r: usize, c: usize| {
        let (r0, r1) = ((r + 1) % 3, (r + 2) % 3);
        let (c0, c1) = ((c + 1) % 3, (c + 2) % 3);
        a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0)
    };
    let det: f32 = (0..3).map(|c| a(0, c) * cofactor(0, c)).sum();
    let sign = if det < 0.0 { -1.0 } else { 1.0 };
    std::array::from_fn(|r| std::array::from_fn(|c| cofactor(r, c) * sign))
}

pub fn parts(room: &Room, materials: &[Material]) -> Vec<Part> {
    room.parts
        .iter()
        .map(|part| {
            let geometry = &room.meshes[part.mesh].1;
            let mesh = &geometry.mesh;
            let normals = normal_matrix(&part.model);
            let along = |v: [f32; 3]| {
                let w = frame::transform(part.model, [v[0], v[1], v[2], 0.0]);
                unit([w[0], w[1], w[2]])
            };
            Part {
                positions: mesh
                    .positions
                    .iter()
                    .map(|&p| point(&part.model, p))
                    .collect(),
                normals: mesh
                    .normals
                    .iter()
                    .map(|&n| unit(normals.map(|row| dot(row, n))))
                    .collect(),
                tangents: mesh
                    .tangents
                    .iter()
                    .map(|t| {
                        let v = along([t[0], t[1], t[2]]);
                        [v[0], v[1], v[2], t[3]]
                    })
                    .collect(),
                uvs: mesh.uvs.clone(),
                uvs1: geometry.uvs1.clone(),
                indices: mesh.indices.clone(),
                material: materials[part.material],
                name: part.name.clone(),
            }
        })
        .collect()
}

fn named() -> Vec<String> {
    match std::env::var("BENCH_DETAIL_PARTS") {
        Ok(names) => names
            .split(',')
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect(),
        Err(_) => super::recipe::DETAIL_PARTS
            .iter()
            .map(|name| name.to_string())
            .collect(),
    }
}

fn input(part: &Part) -> detail::Input<'_> {
    detail::Input {
        positions: &part.positions,
        normals: &part.normals,
        tangents: &part.tangents,
        uvs: &part.uvs,
        uvs1: part.uvs1.as_deref(),
        indices: &part.indices,
        material: &part.material,
    }
}

fn candidates(parts: &[Part], lens: &Lens, size: (u32, u32)) -> Vec<(usize, u32, String)> {
    let (_, camera) = lens.view(0.0, 0.0);
    let view = detail::View {
        matrix: frame::multiply(lens.projection(size.0, size.1), camera),
        size: [size.0, size.1],
    };
    let names = named();
    let mut out = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if !names.contains(&part.name) {
            continue;
        }
        if !part
            .material
            .layers
            .iter()
            .any(|layer| layer.amplitude != 0.0)
        {
            continue;
        }
        let input = input(part);
        let size = detail::resolution(&input, &view, LARGEST);
        if detail::repeats(&input, size) {
            println!(
                "detail bake: {} keeps procedural detail because its UVs repeat",
                part.name
            );
            continue;
        }
        out.push((
            index,
            size,
            detail::input_hash(&input, size, PADDING).unwrap(),
        ));
    }
    out
}

fn pieces(parts: &[Part], candidates: &[(usize, u32, String)]) -> Vec<atlas::Piece> {
    candidates
        .iter()
        .map(|(index, size, hash)| {
            let input = input(&parts[*index]);
            let crop = detail::crop(&input, *size, PADDING).unwrap();
            atlas::Piece {
                index: *index as u32,
                node: parts[*index].name.clone(),
                input_hash: hash.clone(),
                region: detail::bake_crop(&input, *size, PADDING, crop).unwrap(),
            }
        })
        .collect()
}

fn layered(parts: &[Part], atlas: Atlas) -> Baked {
    let mut layers = vec![None; parts.len()];
    let mut objects = BTreeSet::new();
    for (layer, placed) in atlas.parts.iter().enumerate() {
        let index = placed.index as usize;
        if parts.get(index).is_none_or(|part| part.name != placed.node) {
            println!(
                "detail: {} part-{:04} matches no part of the room",
                placed.node, placed.index
            );
            continue;
        }
        layers[index] = Some(layer as u32);
        objects.insert(index as u32 + 1);
    }
    Baked {
        layers,
        atlas,
        objects,
        check: None,
    }
}

pub fn bake(parts: &[Part], lens: &Lens, size: (u32, u32), check: bool) -> Baked {
    let started = Instant::now();
    let candidates = candidates(parts, lens, size);
    assert!(!candidates.is_empty(), "no room part has detail to bake");
    let keys: Vec<(u32, &str, &str)> = candidates
        .iter()
        .map(|(index, _, hash)| (*index as u32, parts[*index].name.as_str(), hash.as_str()))
        .collect();
    let hash = atlas::hash(&keys, atlas::PAGE, PADDING);
    let folder = root().join("detail-atlas");
    let cached = atlas::read(&folder)
        .ok()
        .filter(|(summary, _)| summary.input_hash == hash);
    let packed = match cached {
        Some((summary, packed)) => {
            println!(
                "detail bake: {} parts on {} pages of {}×{}, reused; {} bytes on disk, {} in VRAM",
                packed.parts.len(),
                packed.pages(),
                packed.width,
                packed.height,
                summary.total_bytes,
                summary.vram_bytes
            );
            packed
        }
        None => {
            let flat = atlas::assemble(pieces(parts, &candidates), atlas::PAGE).unwrap();
            let packed = flat.compress().unwrap();
            let psnr = atlas::Kind::ALL.map(|kind| atlas::psnr(&flat, &packed, kind));
            if folder.exists() {
                std::fs::remove_dir_all(&folder).unwrap();
            }
            let summary = atlas::write(&folder, &packed, &hash, PADDING, psnr).unwrap();
            println!(
                "detail bake: {} parts on {} pages of {}×{}, {:.2} s; {} bytes on disk, {} in VRAM, {} decoded, {} as full-tile maps; PSNR base {:?}, normal {:?}, roughness {:?} dB",
                packed.parts.len(),
                packed.pages(),
                packed.width,
                packed.height,
                started.elapsed().as_secs_f64(),
                summary.total_bytes,
                summary.vram_bytes,
                flat.decoded_vram_bytes(),
                candidates
                    .iter()
                    .map(|(_, size, _)| detail::map_bytes(*size))
                    .sum::<u64>(),
                psnr[0],
                psnr[1],
                psnr[2]
            );
            packed
        }
    };
    for part in &packed.parts {
        let material = &parts[part.index as usize].material;
        let layers: Vec<String> = material
            .layers
            .iter()
            .filter(|layer| layer.amplitude != 0.0)
            .map(|layer| format!("{:?}", layer.kind))
            .collect();
        println!(
            "detail: {} part-{:04} {}² crop {}×{} at ({}, {}) through TEXCOORD_{} on page {}, {} bytes, layers {}",
            part.node,
            part.index,
            part.density,
            part.crop.width,
            part.crop.height,
            part.crop.x,
            part.crop.y,
            part.uv_set,
            part.page,
            packed.part_bytes(part),
            layers.join(" ")
        );
    }
    let mut baked = layered(parts, packed);
    if check {
        let pieces = pieces(parts, &candidates);
        let tiles = tiles(&pieces);
        baked.check = Some(Check {
            plain: atlas::assemble(pieces, atlas::PAGE).unwrap(),
            tiles,
        });
    }
    baked
}

pub fn load(parts: &[Part], folder: &Path) -> Baked {
    let atlas = atlas::load(folder).unwrap_or_else(|error| panic!("{}: {error}", folder.display()));
    println!(
        "detail: {} parts on {} pages of {}×{} from {}",
        atlas.parts.len(),
        atlas.pages(),
        atlas.width,
        atlas.height,
        folder.display()
    );
    layered(parts, atlas)
}

pub fn apply(bench: &mut Bench, baked: &Baked) {
    bench.renderer.set_detail(&baked.atlas).unwrap();
    for (index, layer) in baked.layers.iter().enumerate() {
        let Some(layer) = *layer else { continue };
        let original = bench.placed[index].rest.material as usize;
        let mut material = bench.materials[original];
        select_baked_detail(&mut material, layer).unwrap();
        let slot = bench.materials.len() as u32;
        bench.materials.push(material);
        let bare = bench.bare[original];
        bench.bare.push(Material {
            maps: material.maps,
            ..bare
        });
        bench.placed[index].rest.material = slot;
        bench.instances[index].material = slot;
    }
}

pub fn gpu() -> Gpu {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL,
        ..Default::default()
    });
    let adapter = instance
        .enumerate_adapters(wgpu::Backends::VULKAN | wgpu::Backends::METAL)
        .into_iter()
        .find(|adapter| adapter.get_info().device_type == wgpu::DeviceType::DiscreteGpu);
    let Some(adapter) = adapter.filter(|adapter| {
        adapter
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    }) else {
        println!("detail: the GPU has no BC textures; the maps are decoded on the CPU");
        return pollster::block_on(Gpu::headless()).unwrap();
    };
    let features = adapter.features()
        & (pfx_gpu::floor::OPTIONAL_FEATURES | wgpu::Features::TEXTURE_COMPRESSION_BC);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bench with BC textures"),
        required_features: features,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    let info = adapter.get_info();
    Gpu {
        adapter,
        device,
        queue,
        info,
    }
}

fn tiles(pieces: &[atlas::Piece]) -> Option<[Vec<Image>; 3]> {
    if pieces.iter().any(|piece| piece.region.uv_set != 0) {
        println!(
            "detail check: a part bakes through TEXCOORD_1, which full-tile maps cannot sample; skipped the tile frames"
        );
        return None;
    }
    let size = pieces.first()?.region.size;
    if pieces.iter().any(|piece| piece.region.size != size) {
        println!("detail check: the parts' sizes differ, so they cannot share full-tile arrays");
        return None;
    }
    let mut out: [Vec<Image>; 3] = Default::default();
    for piece in pieces {
        let region = &piece.region;
        let crop = region.crop;
        let row = (crop.width * 4) as usize;
        for (images, map) in out
            .iter_mut()
            .zip([&region.base, &region.normal, &region.roughness])
        {
            let mut tile = vec![0u8; (size * size * 4) as usize];
            for y in 0..crop.height {
                let from = (y * crop.width * 4) as usize;
                let to = (((crop.y + y) * size + crop.x) * 4) as usize;
                tile[to..to + row].copy_from_slice(&map[from..from + row]);
            }
            images.push(Image {
                width: size,
                height: size,
                space: ColorSpace::Linear,
                pixels: Pixels::Eight(tile),
            });
        }
    }
    Some(out)
}

fn compare(
    reference: &[u8],
    frame: &[u8],
    ids: &[u32],
    objects: &BTreeSet<u32>,
) -> (u8, u8, usize, f64) {
    let mut worst = 0u8;
    let mut worst_part = 0u8;
    let mut over = 0usize;
    let mut error = 0.0f64;
    let mut count = 0usize;
    for (pixel, (a, b)) in reference
        .chunks_exact(4)
        .zip(frame.chunks_exact(4))
        .enumerate()
    {
        let difference = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap();
        worst = worst.max(difference);
        if objects.contains(&ids[pixel]) {
            worst_part = worst_part.max(difference);
            over += usize::from(difference > 1);
            for c in 0..3 {
                error += (f64::from(a[c]) - f64::from(b[c])).powi(2);
            }
            count += 3;
        }
    }
    let mean = error / count.max(1) as f64;
    let psnr = if mean == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mean).log10()
    };
    (worst, worst_part, over, psnr)
}

pub fn check(bench: &mut Bench, baked: &Baked) {
    let check = baked
        .check
        .as_ref()
        .expect("BENCH_DETAIL_CHECK needs BENCH_BAKE_DETAIL");
    let (plain, tiles) = (&check.plain, &check.tiles);
    let blocks = uploads_blocks(&bench.renderer.gpu().device);
    let decoded = baked.atlas.decode();
    let (width, height) = bench.renderer.size();
    let mut frames: Vec<(&str, Vec<u8>)> = Vec::new();
    for mode in ["tile", "cropped", "bc", "decoded", "tile again"] {
        match mode {
            "tile" | "tile again" => {
                let Some(tiles) = tiles else { continue };
                bench
                    .renderer
                    .set_maps(&MapImages {
                        base: &tiles[0],
                        normal: &tiles[1],
                        roughness: &tiles[2],
                        metal: &[],
                    })
                    .unwrap();
            }
            "cropped" => bench.renderer.set_detail(plain).unwrap(),
            "bc" => {
                if !blocks {
                    println!("detail check: this device has no BC textures; skipped the BC frame");
                    continue;
                }
                bench.renderer.set_detail(&baked.atlas).unwrap();
            }
            _ => bench.renderer.set_detail(&decoded).unwrap(),
        }
        bench.resize(width / 2, height / 2);
        bench.resize(width, height);
        let mut pace = manners::Pace::new();
        for number in 0..SETTLE_FRAMES {
            pace.frame(|| bench.render(number, true, Shown::Rest));
        }
        let pixels = bench.pixels();
        save_png(
            &root().join(format!("detail-{}.png", mode.replace(' ', "-"))),
            width,
            height,
            &pixels,
        );
        frames.push((mode, pixels));
    }
    let ids = bench.ids();
    let shown = ids.iter().filter(|id| baked.objects.contains(id)).count();
    println!(
        "detail check: {shown} pixels show the {} baked parts",
        baked.atlas.parts.len()
    );
    let Some((first, reference)) = frames.first() else {
        return;
    };
    let mut failed = false;
    for (mode, frame) in &frames[1..] {
        let (worst, worst_part, over, psnr) = compare(reference, frame, &ids, &baked.objects);
        println!(
            "detail check: {mode} against {first}: worst {worst}/255 in the frame, {worst_part}/255 on the parts, {over} part pixels over 1/255, PSNR {psnr:.1} dB"
        );
        if (*mode == "cropped" && worst_part > 1) || (*mode == "tile again" && worst > 0) {
            failed = true;
        }
    }
    assert!(!failed, "detail check failed");
}
