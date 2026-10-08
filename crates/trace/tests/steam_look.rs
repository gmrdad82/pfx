use pfx_gpu::Gpu;
use pfx_live::effects::Plume;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::{Detail, PlumeLook, Steam};
use pfx_trace::{Camera, Scene, Sun, Trace};

const SIDE: u32 = 48;
const SAMPLES: u32 = 16;

fn wall() -> Scene<Sky> {
    let wall = Material {
        base: [0.6, 0.6, 0.6],
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    };
    let (low, high, depth) = (-1.0, 1.0, -0.5);
    Scene {
        triangles: vec![
            Triangle {
                vertices: [[low, low, depth], [high, low, depth], [high, high, depth]],
                material: 0,
            },
            Triangle {
                vertices: [[low, low, depth], [high, high, depth], [low, high, depth]],
                material: 0,
            },
        ],
        shapes: Vec::new(),
        materials: vec![wall],
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        },
        camera: Camera {
            origin: [0.0, 0.1, 0.6],
            forward: [0.0, 0.0, -1.0],
            right: [0.2, 0.0, 0.0],
            up: [0.0, 0.2, 0.0],
        },
        sun: Sun {
            direction: [0.3, 0.8, 0.5],
            color: [1.0, 1.0, 1.0],
            intensity: 3.0,
        },
    }
}

fn steam() -> Steam {
    Steam {
        source: [0.0, 0.0, 0.0],
        radius: 0.02,
        density: 1.0,
        ambient: 0.3,
        anisotropy: 0.3,
        box_lo: [-0.15, -0.01, -0.15],
        box_hi: [0.15, 0.22, 0.15],
        time: 1.7,
        look: PlumeLook::default(),
    }
}

fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn traced(steam: Steam) -> Vec<u8> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let detail = Detail {
        steam: Some(steam),
        ..Detail::default()
    };
    let mut trace = Trace::new_detailed(&gpu, &wall(), &detail, SIDE, SIDE).unwrap();
    trace.sample(&gpu, SAMPLES, 7).unwrap();
    trace.readback(&gpu).unwrap().color
}

const NEUTRAL: u64 = 0x5354d689c84aff70;

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_neutral_plume_traces_the_same_bytes_with_the_default_look() {
    let color = traced(steam());
    let bare = traced(Steam {
        density: 0.0,
        ..steam()
    });
    assert_ne!(color, bare);
    assert_eq!(digest(&color), NEUTRAL);
}

fn tracer_plume() -> Plume {
    let look = PlumeLook::default();
    Plume {
        source: [0.0; 3],
        radius: 0.02,
        spread: look.spread,
        drift: look.drift,
        sway: look.sway,
        fade: look.fade,
        depth_per_width: 6.0,
        grain: look.grain,
        lift: look.lift,
        warp: look.warp,
        churn: look.churn,
        threshold: look.threshold,
    }
}

fn steam_of(plume: &Plume) -> Steam {
    let Plume {
        source,
        radius,
        spread,
        drift,
        sway,
        fade,
        depth_per_width: _,
        grain,
        lift,
        warp,
        churn,
        threshold,
    } = *plume;
    Steam {
        source,
        radius,
        look: PlumeLook {
            spread,
            drift,
            sway,
            fade,
            grain,
            lift,
            warp,
            churn,
            threshold,
        },
        ..steam()
    }
}

#[test]
fn the_tracers_default_look_is_valid_and_live_s_default_plume_maps_to_a_valid_one() {
    assert!(steam().valid());
    assert!(steam_of(&Plume::default()).valid());
    assert!(steam_of(&Plume::at([0.1, -0.2, 0.3])).valid());
    assert_eq!(steam_of(&tracer_plume()), steam());
}

#[test]
fn a_look_that_would_divide_by_zero_or_invert_a_ramp_is_invalid() {
    let broken = |edit: fn(&mut PlumeLook)| {
        let mut look = PlumeLook::default();
        edit(&mut look);
        !Steam { look, ..steam() }.valid()
    };
    assert!(broken(|look| look.spread = [0.0, 4.0]));
    assert!(broken(|look| look.spread = [0.5, -10.0]));
    assert!(broken(|look| look.fade = [0.0, 0.07, 0.2]));
    assert!(broken(|look| look.fade = [0.02, 0.3, 0.2]));
    assert!(broken(|look| look.threshold = [0.9, 0.42]));
    assert!(broken(|look| look.churn = f32::NAN));
}

#[test]
fn the_look_s_floats_match_its_shader_constants_one_to_one() {
    let look = PlumeLook::default();
    assert_eq!(PlumeLook::NAMES.len(), look.floats().len());
    assert_eq!(PlumeLook::NAMES[0], "STEAM_SPREAD_BASE");
    assert_eq!(look.floats()[0], 0.7);
    assert_eq!(PlumeLook::NAMES[26], "STEAM_THRESHOLD_HIGH");
    assert_eq!(look.floats()[26], 0.9);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_live_plume_with_the_tracers_numbers_traces_the_neutral_bytes() {
    let color = traced(steam_of(&tracer_plume()));
    assert_eq!(digest(&color), NEUTRAL);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn live_s_default_plume_traces_its_own_look() {
    let neutral = traced(steam());
    let live = traced(steam_of(&Plume::default()));
    assert_ne!(neutral, live);
    assert_eq!(live, traced(steam_of(&Plume::default())));
}
