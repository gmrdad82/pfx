use std::path::Path;
use std::time::Instant;

use pfx_denoise::{Denoiser, pfm};
use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::Sky;
use pfx_load::scene::Scene as LoadedScene;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::Detail;
use pfx_trace::shapes::Shape;
use pfx_trace::{Adaptive, Camera, Projection, Scene, Sun, Trace};

fn usage() -> ! {
    eprintln!(
        "usage: rt_bench time <desk|glass|room> <final|draft> <width> <height>\n       rt_bench still <desk|glass|room> <width> <height> <spp> <seed> <out.pfm>"
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    match args.first().map(String::as_str) {
        Some("time") if args.len() == 5 => {
            let adaptive = match args[2].as_str() {
                "final" => Adaptive {
                    threshold: 0.01,
                    min_samples: 32,
                    max_samples: 128,
                    growth: 1.5,
                },
                "draft" => Adaptive {
                    threshold: 0.05,
                    min_samples: 8,
                    max_samples: 16,
                    growth: 1.5,
                },
                _ => usage(),
            };
            let size = [args[3].parse().unwrap(), args[4].parse().unwrap()];
            let began = Instant::now();
            let mut trace = staged(&gpu, &args[1], size);
            let staged_s = began.elapsed().as_secs_f64();
            let mut pacer = Pacer::default();
            let mut turns = Turns::default();
            let traced = Instant::now();
            let converged = trace
                .sample_adaptive(&gpu, adaptive, 7, &mut pacer, |ms| turns.add(ms))
                .unwrap();
            let traced_s = traced.elapsed().as_secs_f64();
            let mut denoiser = Denoiser::new(&gpu).unwrap();
            let clean = denoiser
                .denoise_textures(
                    &gpu,
                    &trace.color.texture,
                    &trace.albedo.texture,
                    &trace.normal.texture,
                )
                .unwrap();
            assert_eq!(clean.len(), (size[0] * size[1]) as usize);
            println!(
                "{} {} {}x{} rt={} total {:.2} s stage {:.2} s trace {:.2} s mean {:.1} spp",
                args[1],
                args[2],
                size[0],
                size[1],
                hardware(&trace),
                began.elapsed().as_secs_f64(),
                staged_s,
                traced_s,
                converged.mean_samples()
            );
        }
        Some("still") if args.len() == 7 => {
            let size = [args[2].parse().unwrap(), args[3].parse().unwrap()];
            let spp: u32 = args[4].parse().unwrap();
            let seed: u32 = args[5].parse().unwrap();
            let mut trace = staged(&gpu, &args[1], size);
            let began = Instant::now();
            trace.sample(&gpu, spp, seed).unwrap();
            let color = trace.readback_color(&gpu).unwrap();
            let seconds = began.elapsed().as_secs_f64();
            let pixels: Vec<[f32; 4]> = bytemuck::cast_slice::<u8, f32>(&color)
                .chunks_exact(4)
                .map(|p| [p[0], p[1], p[2], p[3]])
                .collect();
            std::fs::write(&args[6], pfm::write(size[0], size[1], &pixels)).unwrap();
            let mean = pixels
                .iter()
                .map(|p| f64::from(p[0] + p[1] + p[2]) / 3.0)
                .sum::<f64>()
                / pixels.len() as f64;
            println!(
                "{} still {}x{} rt={} {spp} spp in {seconds:.2} s, {:.2} M samples/s, mean {mean:.5}",
                args[1],
                size[0],
                size[1],
                hardware(&trace),
                f64::from(size[0] * size[1]) * f64::from(spp) / seconds / 1e6
            );
        }
        _ => usage(),
    }
}

fn hardware(trace: &Trace) -> bool {
    trace.hardware_rays()
}

fn staged(gpu: &Gpu, name: &str, size: [u32; 2]) -> Trace {
    if name == "glass" {
        let detail = Detail {
            projection: Projection::Orthographic {
                width: 3.2,
                height: 3.2 * size[1] as f32 / size[0] as f32,
            },
            ..Detail::default()
        };
        return Trace::new_detailed(gpu, &dense_glass(48, 32), &detail, size[0], size[1]).unwrap();
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../../tmp/rt-bench-scenes");
    std::fs::create_dir_all(&root).unwrap();
    for entry in std::fs::read_dir(manifest.join("../load/tests/scenes")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    for file in ["desk.materials.toml", "desk.scene.toml"] {
        std::fs::copy(
            manifest.join("../bake/tests/plates").join(file),
            root.join(file),
        )
        .unwrap();
    }
    std::fs::copy(
        manifest.join("../text/fonts/EBGaramond[wght].ttf"),
        root.join("serif.ttf"),
    )
    .unwrap();
    let scene = LoadedScene::open(root.join(format!("{name}.scene.toml"))).unwrap();
    let staged = pfx_trace::stage::scene_at(&scene, size[0] as f32 / size[1] as f32, 0.0).unwrap();
    staged.trace(gpu, size[0], size[1]).unwrap()
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
