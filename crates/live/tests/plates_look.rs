#[path = "support/plates_desk.rs"]
mod plates_desk;

use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_live::plates::PlateSettings;
use pfx_live::scene::Override;
use pfx_load::scene::Scene;
use pfx_post::color::encode_channel;

use plates_desk::{Desk, Viewer, debug_dir, save_png};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;

fn traced(scene: &Scene) -> Vec<[f32; 3]> {
    let staged = pfx_trace::stage::scene_at(scene, WIDTH as f32 / HEIGHT as f32, 0.0).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    trace
        .sample_adaptive(
            &gpu,
            pfx_trace::Adaptive {
                threshold: 0.005,
                min_samples: 32,
                max_samples: 1024,
                growth: 2.0,
            },
            11,
            &mut pacer,
            |ms| turns.add(ms),
        )
        .unwrap();
    turns.turn();
    trace
        .readback(&gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

fn lab(linear: [f32; 3]) -> [f64; 3] {
    let display = linear.map(|v| encode_channel(v.clamp(0.0, 1.0)));
    let rgb = display.map(|v| pfx_post::color::srgb_channel(v) as f64);
    let x = 0.4124564 * rgb[0] + 0.3575761 * rgb[1] + 0.1804375 * rgb[2];
    let y = 0.2126729 * rgb[0] + 0.7151522 * rgb[1] + 0.0721750 * rgb[2];
    let z = 0.0193339 * rgb[0] + 0.1191920 * rgb[1] + 0.9503041 * rgb[2];
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            t.cbrt()
        } else {
            (24389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(x / 0.95047), f(y), f(z / 1.08883));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn delta_e(a: [f32; 3], b: [f32; 3]) -> f64 {
    let [l1, a1, b1] = lab(a);
    let [l2, a2, b2] = lab(b);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let mean_c = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (mean_c.powi(7) / (mean_c.powi(7) + 25f64.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let hue = |b: f64, a: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            let h = b.atan2(a).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let h1 = hue(b1, a1p);
    let h2 = hue(b2, a2p);
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 - h1 > 180.0 {
        h2 - h1 - 360.0
    } else {
        h2 - h1 + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let mean_l = (l1 + l2) / 2.0;
    let mean_cp = (c1p + c2p) / 2.0;
    let mean_h = if c1p * c2p == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        (h1 + h2) / 2.0
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (mean_h - 30.0).to_radians().cos()
        + 0.24 * (2.0 * mean_h).to_radians().cos()
        + 0.32 * (3.0 * mean_h + 6.0).to_radians().cos()
        - 0.20 * (4.0 * mean_h - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((mean_h - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (mean_cp.powi(7) / (mean_cp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (mean_l - 50.0).powi(2) / (20.0 + (mean_l - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * mean_cp;
    let sh = 1.0 + 0.015 * mean_cp * t;
    let rt = -(2.0 * dtheta).to_radians().sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh))
        .sqrt()
}

#[derive(Debug, Clone, Copy)]
struct Stats {
    mean: f64,
    p95: f64,
    pixels: usize,
}

fn stats(a: &[[f32; 3]], b: &[[f32; 3]], keep: impl Fn(usize) -> bool) -> Stats {
    let mut errors: Vec<f64> = (0..a.len())
        .filter(|&i| keep(i))
        .map(|i| delta_e(a[i], b[i]))
        .collect();
    errors.sort_by(f64::total_cmp);
    let pixels = errors.len();
    let mean = errors.iter().sum::<f64>() / pixels.max(1) as f64;
    let p95 = errors
        .get(((pixels as f64 * 0.95) as usize).min(pixels.saturating_sub(1)))
        .copied()
        .unwrap_or(0.0);
    Stats { mean, p95, pixels }
}

fn heat(a: &[[f32; 3]], b: &[[f32; 3]]) -> Vec<[f32; 3]> {
    (0..a.len())
        .map(|i| {
            let e = (delta_e(a[i], b[i]) / 6.0).min(1.0) as f32;
            [e, (e * 0.4).min(1.0), 1.0 - e].map(|v| pfx_post::color::srgb_channel(v * 0.9))
        })
        .collect()
}

fn half(image: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let (w, h) = (WIDTH as usize / 2, HEIGHT as usize / 2);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = image[(y * 2 + dy) * WIDTH as usize + x * 2 + dx];
                for k in 0..3 {
                    sum[k] += p[k].clamp(0.0, 1.0) * 0.25;
                }
            }
            out.push(sum);
        }
    }
    out
}

fn sheet(rows: &[Vec<Vec<[f32; 3]>>]) -> (u32, u32, Vec<[f32; 3]>) {
    let (w, h) = (WIDTH as usize / 2, HEIGHT as usize / 2);
    let columns = rows[0].len();
    let gap = 4;
    let width = columns * w + (columns - 1) * gap;
    let height = rows.len() * h + (rows.len() - 1) * gap;
    let mut out = vec![[0.02f32; 3]; width * height];
    for (r, row) in rows.iter().enumerate() {
        for (c, tile) in row.iter().enumerate() {
            let small = half(tile);
            for y in 0..h {
                for x in 0..w {
                    out[(r * (h + gap) + y) * width + c * (w + gap) + x] = small[y * w + x];
                }
            }
        }
    }
    (width as u32, height as u32, out)
}

fn live(scene: &Scene, hide: bool, relight: bool) -> Vec<[f32; 3]> {
    let mut viewer = Viewer::new(scene, WIDTH, HEIGHT);
    viewer.renderer.set_plate_settings(PlateSettings {
        relight,
        ..PlateSettings::default()
    });
    if hide {
        for name in scene.dynamic_names() {
            viewer.staged.set_override(
                &name,
                Override {
                    hidden: true,
                    ..Override::default()
                },
            );
        }
    }
    viewer.frames(0.0, 24)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn plates_look_like_the_trace_at_an_anchor_and_between_anchors() {
    let wide = Desk::new("look", "", [WIDTH, HEIGHT]);
    wide.bake_with(&[12.0, 17.0], 0.005, 1024);
    let near = Desk::new("look-near", "", [WIDTH, HEIGHT]);
    near.bake_with(&[14.0, 15.0], 0.005, 1024);
    let dir = debug_dir();
    let mut rows = Vec::new();
    let mut table = Vec::new();
    for (label, desk, hour, relight) in [
        ("at 12 h, anchors 12 and 17", &wide, 12.0f64, true),
        ("at 14.5 h, anchors 12 and 17, blended", &wide, 14.5, false),
        ("at 14.5 h, anchors 12 and 17, relit", &wide, 14.5, true),
        ("at 14.5 h, anchors 14 and 15, relit", &near, 14.5, true),
    ] {
        let plated = Scene::open_at(desk.path(), hour).unwrap();
        let bare = Scene::open_at(desk.root.join("bare.scene.toml"), hour).unwrap();
        let reference = traced(&bare);
        let with = live(&plated, false, relight);
        let without_moving = live(&plated, true, relight);
        let raster = live(&bare, false, relight);
        let moving: Vec<bool> = (0..with.len())
            .map(|i| delta_e(with[i], without_moving[i]) > 0.5)
            .collect();
        for name in ["all", "static", "dynamic"] {
            let keep = |i: usize| match name {
                "static" => !moving[i],
                "dynamic" => moving[i],
                _ => true,
            };
            let plate = stats(&with, &reference, keep);
            let full = stats(&raster, &reference, keep);
            table.push((label, name, plate, full));
        }
        let tag = format!(
            "{}-{hour}h{}",
            desk.root.file_name().unwrap().to_string_lossy(),
            if relight { "" } else { "-blended" }
        )
        .replace('.', "_");
        save_png(
            &dir.join(format!("trace-{tag}.png")),
            WIDTH,
            HEIGHT,
            &reference,
        );
        save_png(&dir.join(format!("plates-{tag}.png")), WIDTH, HEIGHT, &with);
        save_png(
            &dir.join(format!("raster-{tag}.png")),
            WIDTH,
            HEIGHT,
            &raster,
        );
        let error = heat(&with, &reference);
        save_png(&dir.join(format!("error-{tag}.png")), WIDTH, HEIGHT, &error);
        let raster_error = heat(&raster, &reference);
        rows.push(vec![reference, with, raster, error, raster_error]);
    }
    let (width, height, pixels) = sheet(&rows);
    save_png(&dir.join("contact.png"), width, height, &pixels);
    println!(
        "case | region | pixels | plates mean p95 | full raster mean p95 (CIEDE2000 against the full trace)"
    );
    for (label, name, plate, full) in &table {
        println!(
            "{label} | {name} | {} | {:.2} {:.2} | {:.2} {:.2}",
            plate.pixels, plate.mean, plate.p95, full.mean, full.p95
        );
    }
    let find = |label: &str, name: &str| {
        table
            .iter()
            .find(|row| row.0 == label && row.1 == name)
            .map(|row| row.2)
            .unwrap()
    };
    let anchor = find("at 12 h, anchors 12 and 17", "static");
    assert!(anchor.mean < 1.0 && anchor.p95 < 2.5, "{anchor:?}");
    let blended = find("at 14.5 h, anchors 12 and 17, blended", "static");
    let relit = find("at 14.5 h, anchors 12 and 17, relit", "static");
    assert!(relit.mean < blended.mean, "{relit:?} {blended:?}");
    let close = find("at 14.5 h, anchors 14 and 15, relit", "static");
    assert!(close.mean < 3.5, "{close:?}");
}
