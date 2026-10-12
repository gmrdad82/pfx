use std::io::Write;
use std::path::Path;
use std::time::Instant;

use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_load::scene::Scene as LoadedScene;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::Detail;
use pfx_trace::shapes::Shape;
use pfx_trace::sobol::{self, Sampler};
use pfx_trace::{Camera, Projection, Scene, Sun, Trace};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;
const COUNTS: [u32; 5] = [1, 4, 16, 64, 256];
const SEED: u32 = 7;
const REFERENCE_SEED: u32 = 1013;

fn say(dir: &Path, line: String) {
    println!("{line}");
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("log.txt"))
        .unwrap();
    writeln!(log, "{line}").unwrap();
}

fn usage() -> ! {
    eprintln!("usage: sobol <dir> <reference spp> [desk|room|glass]...");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(dir), Some(Ok(reference_spp))) = (args.first(), args.get(1).map(|a| a.parse()))
    else {
        usage()
    };
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir).unwrap();
    let names: Vec<&str> = if args.len() > 2 {
        args[2..].iter().map(String::as_str).collect()
    } else {
        vec!["desk", "room", "glass"]
    };
    for sequence in [256, 1024, 4096, 8192] {
        let start = Instant::now();
        let table = sobol::table(sequence);
        say(
            dir,
            format!(
                "table {sequence} x {} patterns: {} texels, {:.1} MB, {:.1} ms",
                sobol::PATTERNS,
                table.len(),
                (table.len() * 16) as f64 / 1e6,
                start.elapsed().as_secs_f64() * 1e3
            ),
        );
    }
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    stage_files(dir);
    for name in names {
        measure(&gpu, dir, name, reference_spp);
    }
}

fn stage_files(dir: &Path) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let desk = dir.join("desk-scene");
    std::fs::create_dir_all(&desk).unwrap();
    for entry in std::fs::read_dir(manifest.join("../load/tests/scenes")).unwrap() {
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
}

fn trace(gpu: &Gpu, dir: &Path, name: &str, sampler: Sampler) -> Trace {
    if name == "glass" {
        let detail = Detail {
            projection: Projection::Orthographic {
                width: 3.2,
                height: 1.8,
            },
            sampler,
            ..Detail::default()
        };
        return Trace::new_detailed(gpu, &dense_glass(48, 32), &detail, WIDTH, HEIGHT).unwrap();
    }
    let path = dir.join("desk-scene").join(format!("{name}.scene.toml"));
    let scene = LoadedScene::open(&path).unwrap();
    let mut staged = pfx_trace::stage::scene(&scene, WIDTH as f32 / HEIGHT as f32).unwrap();
    staged.detail.sampler = sampler;
    staged.trace(gpu, WIDTH, HEIGHT).unwrap()
}

fn rgb(bytes: &[u8]) -> Vec<f32> {
    floats(bytes)
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn mse(a: &[f32], b: &[f32]) -> f64 {
    let sum: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            let d = f64::from(*x) - f64::from(*y);
            d * d
        })
        .sum();
    sum / a.len() as f64
}

fn timed(gpu: &Gpu, trace: &mut Trace, count: u32) -> f64 {
    let start = Instant::now();
    trace.sample(gpu, count, SEED).unwrap();
    gpu.device
        .poll(pfx_gpu::wgpu::PollType::wait_indefinitely())
        .unwrap();
    start.elapsed().as_secs_f64()
}

fn reference(gpu: &Gpu, dir: &Path, name: &str, spp: u32) -> Vec<f32> {
    let path = dir.join(format!("{name}.reference.{spp}.f32"));
    if let Ok(bytes) = std::fs::read(&path) {
        return floats(&bytes);
    }
    let mut trace = trace(gpu, dir, name, Sampler::Pcg);
    let start = Instant::now();
    trace.sample(gpu, spp, REFERENCE_SEED).unwrap();
    let color = rgb(&trace.readback_color(gpu).unwrap());
    say(
        dir,
        format!(
            "{name}: reference {spp} spp PCG in {:.1} s",
            start.elapsed().as_secs_f64()
        ),
    );
    let bytes: Vec<u8> = color.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(&path, bytes).unwrap();
    color
}

fn run(gpu: &Gpu, dir: &Path, name: &str, sampler: Sampler, truth: &[f32]) -> (Vec<f64>, f64) {
    let mut trace = trace(gpu, dir, name, sampler);
    let mut done = 0;
    let mut seconds = 0.0;
    let mut errors = Vec::new();
    for count in COUNTS {
        seconds += timed(gpu, &mut trace, count - done);
        done = count;
        errors.push(mse(&rgb(&trace.readback_color(gpu).unwrap()), truth));
        say(
            dir,
            format!("{name} {sampler:?}: {count} spp after {seconds:.1} s"),
        );
    }
    let paths = f64::from(WIDTH * HEIGHT) * f64::from(done) / seconds;
    (errors, paths)
}

fn spp_at(errors: &[f64], target: f64) -> f64 {
    let points: Vec<(f64, f64)> = COUNTS
        .iter()
        .zip(errors)
        .map(|(n, e)| (f64::from(*n).ln(), e.ln()))
        .collect();
    let t = target.ln();
    let segment = points
        .windows(2)
        .find(|w| t <= w[0].1 && t >= w[1].1)
        .unwrap_or(if t > points[0].1 {
            &points[0..2]
        } else {
            &points[points.len() - 2..]
        });
    let ((x0, y0), (x1, y1)) = (segment[0], segment[1]);
    (x0 + (t - y0) * (x1 - x0) / (y1 - y0)).exp()
}

fn measure(gpu: &Gpu, dir: &Path, name: &str, reference_spp: u32) {
    let truth = reference(gpu, dir, name, reference_spp);
    let (pcg, pcg_paths) = run(gpu, dir, name, Sampler::Pcg, &truth);
    let (sob, sob_paths) = run(gpu, dir, name, Sampler::Sobol { samples: 256 }, &truth);
    let floor = pcg[0] / f64::from(reference_spp);
    let corrected = |errors: &[f64]| -> Vec<f64> {
        errors
            .iter()
            .map(|e| (e - floor).max(e * 1e-3).sqrt())
            .collect()
    };
    let (pcg_rmse, sob_rmse) = (corrected(&pcg), corrected(&sob));
    say(
        dir,
        format!("{name}: {WIDTH}x{HEIGHT}, reference {reference_spp} spp PCG"),
    );
    say(
        dir,
        "  spp   PCG rmse    Sobol rmse  (raw PCG, raw Sobol)".to_string(),
    );
    for (i, count) in COUNTS.iter().enumerate() {
        say(
            dir,
            format!(
                "  {count:>4}  {:.6}  {:.6}  ({:.6}, {:.6})",
                pcg_rmse[i],
                sob_rmse[i],
                pcg[i].sqrt(),
                sob[i].sqrt()
            ),
        );
    }
    let squared = |errors: &[f64]| errors.iter().map(|e| e * e).collect::<Vec<_>>();
    let sob_mse = squared(&sob_rmse);
    for (i, count) in COUNTS.iter().enumerate().skip(2) {
        let need = spp_at(&sob_mse, pcg_rmse[i] * pcg_rmse[i]);
        say(
            dir,
            format!(
                "  PCG {count:>3} spp -> Sobol {need:.1} spp at equal RMSE (x{:.2})",
                f64::from(*count) / need
            ),
        );
    }
    say(
        dir,
        format!(
            "  paths/s: PCG {:.2} M, Sobol {:.2} M",
            pcg_paths / 1e6,
            sob_paths / 1e6
        ),
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
