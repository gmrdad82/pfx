use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_denoise::{Denoiser, pfm};
use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_load::scene::Scene as LoadedScene;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::Detail;
use pfx_trace::shapes::Shape;
use pfx_trace::{Camera, Projection, Scene, Sun, Trace};

fn usage() -> ! {
    eprintln!(
        "usage: compare stage <dir> <width> <spp>\n       compare denoise <dir> <name>\n       compare bench <width> <height>"
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    match args.first().map(String::as_str) {
        Some("stage") if args.len() == 4 => stage(
            &gpu,
            Path::new(&args[1]),
            args[2].parse().unwrap(),
            args[3].parse().unwrap(),
        ),
        Some("denoise") if args.len() == 3 => denoise(&gpu, Path::new(&args[1]), &args[2]),
        Some("bench") if args.len() == 3 => {
            bench(&gpu, args[1].parse().unwrap(), args[2].parse().unwrap())
        }
        _ => usage(),
    }
}

fn pixels(bytes: &[u8]) -> Vec<[f32; 4]> {
    bytemuck::cast_slice::<u8, f32>(bytes)
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect()
}

fn write(trace: &mut Trace, gpu: &Gpu, dir: &Path, name: &str, width: u32, height: u32, spp: u32) {
    trace.sample(gpu, spp, 7).unwrap();
    let output = trace.readback(gpu).unwrap();
    for (part, bytes) in [
        ("color", &output.color),
        ("albedo", &output.albedo),
        ("normal", &output.normal),
    ] {
        std::fs::write(
            dir.join(format!("{name}.{part}.pfm")),
            pfm::write(width, height, &pixels(bytes)),
        )
        .unwrap();
    }
    println!("{name}: {width}x{height} at {spp} spp");
}

fn stage(gpu: &Gpu, dir: &Path, width: u32, spp: u32) {
    std::fs::create_dir_all(dir).unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scenes = manifest.join("../load/tests/scenes");
    let desk = dir.join("desk-scene");
    std::fs::create_dir_all(&desk).unwrap();
    for entry in std::fs::read_dir(&scenes).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), desk.join(entry.file_name())).unwrap();
    }
    for file in ["desk.scene.toml", "desk.materials.toml"] {
        std::fs::copy(
            manifest.join("../bake/tests/plates").join(file),
            desk.join(file),
        )
        .unwrap();
    }
    std::fs::copy(
        manifest.join("../text/fonts/EBGaramond[wght].ttf"),
        desk.join("serif.ttf"),
    )
    .unwrap();
    for (name, path) in [
        ("desk", desk.join("desk.scene.toml")),
        ("room", desk.join("room.scene.toml")),
    ] {
        let height = width * 9 / 16;
        let scene = LoadedScene::open(&path).unwrap();
        let staged = pfx_trace::stage::scene(&scene, width as f32 / height as f32).unwrap();
        let mut trace = staged.trace(gpu, width, height).unwrap();
        write(&mut trace, gpu, dir, name, width, height, spp);
    }
    let height = width * 5 / 8;
    let detail = Detail {
        projection: Projection::Orthographic {
            width: 3.2,
            height: 2.0,
        },
        ..Detail::default()
    };
    let mut trace = Trace::new_detailed(gpu, &dense_glass(48, 32), &detail, width, height).unwrap();
    write(&mut trace, gpu, dir, "glass", width, height, spp);
}

fn load(dir: &Path, name: &str, part: &str) -> pfm::Pfm {
    pfm::read(&std::fs::read(dir.join(format!("{name}.{part}.pfm"))).unwrap()).unwrap()
}

fn display(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn denoise(gpu: &Gpu, dir: &Path, name: &str) {
    let color = load(dir, name, "color");
    let albedo = load(dir, name, "albedo");
    let normal = load(dir, name, "normal");
    let mut denoiser = Denoiser::new(gpu).unwrap();
    let (w, h) = (color.width, color.height);
    let run = |denoiser: &mut Denoiser| {
        denoiser
            .denoise(gpu, w, h, &color.pixels, &albedo.pixels, &normal.pixels)
            .unwrap()
    };
    let start = Instant::now();
    let out = run(&mut denoiser);
    let first = start.elapsed().as_secs_f64() * 1e3;
    let start = Instant::now();
    let again = run(&mut denoiser);
    let warm = start.elapsed().as_secs_f64() * 1e3;
    assert_eq!(out, again, "the denoiser is not deterministic");
    let target: PathBuf = dir.join(format!("{name}.pfx.pfm"));
    std::fs::write(&target, pfm::write(w, h, &out)).unwrap();
    println!(
        "{name}: {w}x{h}, first {first:.1} ms (with pipelines), warm {warm:.1} ms, wrote {}",
        target.display()
    );
    let reference = dir.join(format!("{name}.oidn.pfm"));
    if let Ok(bytes) = std::fs::read(&reference) {
        let oidn = pfm::read(&bytes).unwrap();
        let mut max = 0.0f32;
        let mut sum = 0.0f64;
        let mut squared = 0.0f64;
        let mut display_max = 0.0f32;
        for (a, b) in out.iter().zip(&oidn.pixels) {
            for c in 0..3 {
                let gap = (a[c] - b[c]).abs();
                max = max.max(gap);
                sum += f64::from(gap);
                let d = (display(a[c]) - display(b[c])).abs();
                display_max = display_max.max(d);
                squared += f64::from(d * d);
            }
        }
        let count = (out.len() * 3) as f64;
        let mse = squared / count;
        let psnr = if mse > 0.0 {
            10.0 * (1.0 / mse).log10()
        } else {
            f64::INFINITY
        };
        println!(
            "{name} against OIDN: linear max {max:.6} mean {:.3e}; display max {display_max:.6} ({:.2} of 255), PSNR {psnr:.2} dB",
            sum / count,
            display_max * 255.0
        );
    }
}

fn texture(gpu: &Gpu, width: u32, height: u32, pixels: &[[f32; 4]]) -> wgpu::Texture {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(pixels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 16),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture
}

fn bench(gpu: &Gpu, width: u32, height: u32) {
    let count = (width * height) as usize;
    let mut color = Vec::with_capacity(count);
    let mut seed = 0x9e37_79b9u32;
    for i in 0..count {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let noise = (seed >> 8) as f32 / (1u32 << 24) as f32;
        let x = (i % width as usize) as f32 / width as f32;
        color.push([x * 2.0 * noise, 0.5 * noise, (1.0 - x) * noise, 1.0]);
    }
    let albedo = vec![[0.6, 0.5, 0.4, 1.0]; count];
    let normal = vec![[0.0, 0.0, 1.0, 0.0]; count];
    let mut denoiser = Denoiser::new(gpu).unwrap();
    let start = Instant::now();
    denoiser
        .denoise(gpu, width, height, &color, &albedo, &normal)
        .unwrap();
    let first = start.elapsed().as_secs_f64() * 1e3;
    let start = Instant::now();
    denoiser
        .denoise(gpu, width, height, &color, &albedo, &normal)
        .unwrap();
    let slices = start.elapsed().as_secs_f64() * 1e3;
    let inputs = [&color, &albedo, &normal].map(|pixels| texture(gpu, width, height, pixels));
    let output = texture(gpu, width, height, &albedo);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mut runs = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        denoiser
            .denoise_textures_into(gpu, &inputs[0], &inputs[1], &inputs[2], &output)
            .unwrap();
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        runs.push(start.elapsed().as_secs_f64() * 1e3);
    }
    runs.sort_by(f64::total_cmp);
    let (tile_w, tile_h, tiles) = denoiser.tiles().unwrap();
    println!(
        "{width}x{height}: textures to texture median {:.0} ms (best {:.0}); slices with upload and readback {slices:.0} ms; first call with pipelines {first:.0} ms; {tiles} tile(s) of {tile_w}x{tile_h}",
        runs[2], runs[0]
    );
}

fn quad(corners: [[f32; 3]; 4], material: u32) -> [Triangle; 2] {
    [
        Triangle {
            vertices: [corners[0], corners[1], corners[2]],
            material,
        },
        Triangle {
            vertices: [corners[0], corners[2], corners[3]],
            material,
        },
    ]
}

fn glass_material(base: [f32; 3], roughness: f32, ior: f32) -> Material {
    Material {
        base,
        roughness,
        transmission: 1.0,
        ior,
        specular: 0.04,
        ..Material::default()
    }
}

fn sheet(y: f32, x0: f32, x1: f32, z0: f32, z1: f32, material: u32) -> [Triangle; 2] {
    quad(
        [[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]],
        material,
    )
}

fn liquid_surface(material: u32, nx: u32, nz: u32) -> Vec<Triangle> {
    let (x0, x1, z0, z1) = (-1.6, 1.6, -2.4, -0.2);
    let mut points = Vec::with_capacity(((nx + 1) * (nz + 1)) as usize);
    for j in 0..=nz {
        for i in 0..=nx {
            let u = i as f32 / nx as f32;
            let v = j as f32 / nz as f32;
            let y = 0.12 + 0.07 * (u * 14.0).sin() * (v * 10.0).cos();
            points.push([x0 + (x1 - x0) * u, y, z0 + (z1 - z0) * v]);
        }
    }
    let at = |i: u32, j: u32| points[(j * (nx + 1) + i) as usize];
    let mut triangles = Vec::new();
    for j in 0..nz {
        for i in 0..nx {
            triangles.extend(quad(
                [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)],
                material,
            ));
        }
    }
    triangles.extend(sheet(0.02, x0, x1, z0, z1, material));
    triangles
}

fn liquid_volume() -> Vec<Shape> {
    let mut shapes = Vec::new();
    for (x, z) in [(-0.7, -1.7), (0.5, -1.3), (-0.2, -0.8), (0.9, -2.0)] {
        let left = shapes.len();
        shapes.push(Shape::Ellipsoid {
            center: [x, 0.35, z],
            radii: [0.45, 0.28, 0.38],
            material: 1,
        });
        shapes.push(Shape::Ellipsoid {
            center: [x + 0.28, 0.42, z + 0.16],
            radii: [0.32, 0.22, 0.3],
            material: 1,
        });
        shapes.push(Shape::SmoothUnion {
            left,
            right: left + 1,
            radius: 0.18,
            material: 1,
        });
    }
    shapes
}

fn dense_glass(nx: u32, nz: u32) -> Scene {
    let mut triangles = Vec::new();
    triangles.extend(sheet(0.0, -4.0, 4.0, -3.2, 3.2, 2));
    for (pane, y) in [0.55, 0.95, 1.35, 1.75].into_iter().enumerate() {
        let shift = if pane % 2 == 0 { -0.05 } else { 0.05 };
        triangles.extend(sheet(y, -1.7, 1.7, -2.45 + shift, -0.15, 0));
    }
    triangles.extend(liquid_surface(1, nx, nz));
    Scene {
        triangles,
        shapes: liquid_volume(),
        materials: vec![
            glass_material([0.92, 0.96, 1.0], 0.02, 1.5),
            glass_material([0.75, 0.88, 0.95], 0.08, 1.33),
            Material {
                base: [0.55, 0.52, 0.48],
                roughness: 0.8,
                specular: 0.03,
                ..Material::default()
            },
        ],
        sky: Sky {
            width: 2,
            height: 1,
            texels: vec![[1.0, 0.95, 0.9, 1.0], [0.75, 0.82, 0.95, 1.0]],
        },
        camera: Camera {
            origin: [0.0, 6.0, -1.3],
            forward: [0.0, -1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
        },
        sun: Sun {
            direction: [0.2, 0.9, 0.1],
            color: [1.0, 0.98, 0.94],
            intensity: 2.0,
        },
    }
}
