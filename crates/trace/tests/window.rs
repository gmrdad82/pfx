use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::adaptive::Adaptive;
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::{Detail, Lens};
use pfx_trace::shapes::Shape;
use pfx_trace::{Camera, Scene, Sun, Trace};

const FULL: [u32; 2] = [96, 64];
const CROP: [u32; 4] = [30, 20, 32, 24];

fn scene() -> (Scene, Detail) {
    let half = 2.0;
    let corners = [
        [-half, 0.0, -half],
        [half, 0.0, -half],
        [half, 0.0, half],
        [-half, 0.0, half],
    ];
    let scene = Scene {
        triangles: vec![
            Triangle {
                vertices: [corners[0], corners[1], corners[2]],
                material: 0,
            },
            Triangle {
                vertices: [corners[0], corners[2], corners[3]],
                material: 0,
            },
        ],
        shapes: vec![Shape::Ellipsoid {
            center: [0.2, 0.5, -0.3],
            radii: [0.5; 3],
            material: 1,
        }],
        materials: vec![
            Material {
                base: [0.8; 3],
                roughness: 0.9,
                ..Material::default()
            },
            Material {
                base: [0.9, 0.6, 0.3],
                roughness: 0.25,
                metalness: 1.0,
                ..Material::default()
            },
        ],
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.3, 0.35, 0.4, 1.0]],
        },
        camera: Camera {
            origin: [0.0, 1.2, 2.6],
            forward: [0.0, -0.35, -1.0],
            right: [0.6, 0.0, 0.0],
            up: [0.0, 0.4, -0.14],
        },
        sun: Sun {
            direction: [0.4, 0.8, 0.3],
            color: [1.0, 0.95, 0.9],
            intensity: 3.0,
        },
    };
    let detail = Detail {
        lens: Lens {
            shift: [0.05, -0.03],
            aperture: 0.02,
            focus_distance: 2.6,
        },
        ..Detail::default()
    };
    (scene, detail)
}

fn region(color: &[u8], width: u32, rect: [u32; 4]) -> Vec<u8> {
    let [x, y, w, h] = rect.map(|v| v as usize);
    let row = width as usize * 16;
    (y..y + h)
        .flat_map(|r| color[r * row + x * 16..r * row + (x + w) * 16].to_vec())
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_window_traces_the_same_pixels_as_the_full_frame() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (scene, detail) = scene();
    let adaptive = Adaptive {
        threshold: 0.02,
        min_samples: 8,
        max_samples: 128,
        growth: 1.5,
    };
    let seed = 9;
    let adaptive_run = |trace: &mut Trace| {
        let mut pacer = Pacer::default();
        let mut turns = Turns::default();
        let mut reached = Vec::new();
        let converged = trace
            .sample_adaptive_with(
                &gpu,
                adaptive,
                seed,
                &mut pacer,
                |ms| turns.add(ms),
                |round| reached.push(round),
            )
            .unwrap();
        assert_eq!(reached, converged.rounds);
        trace.readback_color(&gpu).unwrap()
    };

    let mut full = Trace::new_detailed(&gpu, &scene, &detail, FULL[0], FULL[1]).unwrap();
    let full_adaptive = adaptive_run(&mut full);
    let mut full = Trace::new_detailed(&gpu, &scene, &detail, FULL[0], FULL[1]).unwrap();
    full.sample(&gpu, 16, seed).unwrap();
    let full_fixed = full.readback_color(&gpu).unwrap();

    let [x, y, w, h] = CROP;
    let mut crop = Trace::new_detailed(&gpu, &scene, &detail, w, h).unwrap();
    crop.set_window(FULL, [x, y]).unwrap();
    crop.sample(&gpu, 16, seed).unwrap();
    assert!(
        crop.readback_color(&gpu).unwrap() == region(&full_fixed, FULL[0], CROP),
        "a fixed-sample window differs from the full frame"
    );

    let margin = adaptive.margin();
    let padded = [x - margin, y - margin, w + 2 * margin, h + 2 * margin];
    let mut crop = Trace::new_detailed(&gpu, &scene, &detail, padded[2], padded[3]).unwrap();
    crop.set_window(FULL, [padded[0], padded[1]]).unwrap();
    let color = adaptive_run(&mut crop);
    assert!(
        region(&color, padded[2], [margin, margin, w, h]) == region(&full_adaptive, FULL[0], CROP),
        "an adaptive window padded by the margin differs from the full frame"
    );

    let mut outside = Trace::new_detailed(&gpu, &scene, &detail, w, h).unwrap();
    assert!(outside.set_window(FULL, [FULL[0] - w + 1, 0]).is_err());
}
