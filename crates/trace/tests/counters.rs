use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::counters::Counts;
use pfx_trace::detail::Detail;
use pfx_trace::fog::Fog;
use pfx_trace::shapes::Shape;
use pfx_trace::{Camera, Scene, Sun, Trace};

const SIZE: [u32; 2] = [48, 32];
const SAMPLES: u32 = 4;
const SEED: u32 = 5;

fn scene() -> Scene {
    let half = 60.0;
    let corners = [
        [-half, 0.0, -half],
        [half, 0.0, -half],
        [half, 0.0, half],
        [-half, 0.0, half],
    ];
    Scene {
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
            center: [0.0, 0.4, 0.0],
            radii: [0.4; 3],
            material: 1,
        }],
        materials: vec![
            Material {
                base: [0.7; 3],
                roughness: 0.8,
                ..Material::default()
            },
            Material {
                base: [0.8, 0.5, 0.3],
                roughness: 0.3,
                ..Material::default()
            },
        ],
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.2, 0.25, 0.3, 1.0]],
        },
        camera: Camera {
            origin: [0.0, 2.0, 2.0],
            forward: [0.0, -0.7, -0.7],
            right: [0.5, 0.0, 0.0],
            up: [0.0, 0.25, -0.25],
        },
        sun: Sun {
            direction: [0.3, 0.9, 0.2],
            color: [1.0; 3],
            intensity: 3.0,
        },
    }
}

fn counted(gpu: &Gpu, detail: &Detail) -> (Counts, Vec<u8>, bool) {
    let mut trace = Trace::new_detailed(gpu, &scene(), detail, SIZE[0], SIZE[1]).unwrap();
    assert!(trace.counting());
    trace.sample(gpu, SAMPLES, SEED).unwrap();
    let counts = trace.take_counts(gpu).unwrap().unwrap();
    let color = trace.readback_color(gpu).unwrap();
    assert_eq!(trace.take_counts(gpu).unwrap(), Some(Counts::default()));
    (counts, color, trace.hardware_rays())
}

fn counting() -> Detail {
    Detail {
        counters: true,
        ..Detail::default()
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_counters_count_every_path_and_its_rays_by_kind() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (counts, color, hardware) = counted(&gpu, &counting());
    let paths = u64::from(SIZE[0] * SIZE[1] * SAMPLES);
    assert_eq!(counts.paths, paths);
    assert_eq!(counts.primary, paths);
    assert_eq!(counts.closest, counts.primary + counts.bounce);
    assert!(counts.bounce > 0);
    assert!(counts.surface_shadow >= paths);
    assert_eq!(counts.shadow, counts.surface_shadow);
    assert_eq!((counts.mover_queries, counts.mover_walks), (0, 0));
    if hardware {
        assert!(counts.closest_candidates > 0);
        assert!(counts.shadow_candidates > 0);
    } else {
        assert_eq!(
            (counts.closest_candidates, counts.shadow_candidates),
            (0, 0)
        );
    }
    let (again, same, _) = counted(&gpu, &counting());
    assert_eq!(again, counts);
    assert_eq!(same, color);
    let mut plain = Trace::new(&gpu, &scene(), SIZE[0], SIZE[1]).unwrap();
    assert!(!plain.counting());
    plain.sample(&gpu, SAMPLES, SEED).unwrap();
    assert_eq!(plain.take_counts(&gpu).unwrap(), None);
    assert_eq!(plain.readback_color(&gpu).unwrap(), color);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn fog_shadow_rays_count_apart_from_the_surfaces() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let fogged = Detail {
        fog: Some(Fog {
            height: 0.0,
            falloff: 0.0,
            density: 0.2,
            reach: 30.0,
            shafts: 1.0,
            color: [1.0; 3],
            phase: 0.3,
            bounces: 1,
        }),
        ..counting()
    };
    let (counts, _, _) = counted(&gpu, &fogged);
    let paths = u64::from(SIZE[0] * SIZE[1] * SAMPLES);
    assert_eq!(counts.primary, paths);
    assert!(counts.shadow > counts.surface_shadow);
    assert!(counts.rays_per_path().other_shadow > 0.0);
    assert_eq!(counts.closest, counts.primary + counts.bounce);
}
