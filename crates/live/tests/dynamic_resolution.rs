#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::Path;

use pfx_gpu::pace::Pace;
use pfx_gpu::screens::Budget;
use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;

const CONTENT: [u32; 2] = [160, 96];
const SEED: u32 = 7;

fn run(budget: Option<Budget>, frames: u64) -> (Renderer, Vec<(u32, u32)>) {
    let scene = Scene::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes/room.scene.toml"),
    )
    .unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu.clone(), CONTENT[0], CONTENT[1]).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_dynamic_resolution(budget).unwrap();
    let mut staged = renderer.stage(&scene).unwrap();
    let mut pace = Pace::new();
    let mut sizes = Vec::new();
    let mut target: Option<OffscreenTarget> = None;
    for frame in 0..frames {
        let (width, height) = renderer.apply_dynamic_resolution(CONTENT).unwrap();
        sizes.push((width, height));
        if target
            .as_ref()
            .is_none_or(|target| (target.width, target.height) != (width, height))
        {
            target = Some(output(&gpu, width, height));
        }
        let view = &target.as_ref().unwrap().view;
        let finish = staged.finish();
        let drawn = staged.frame(width as f32 / height as f32, frame as f32 / 60.0, SEED);
        let renderer = &mut renderer;
        pace.frame(|| {
            renderer
                .submit(&drawn.scene, &drawn.text, &drawn.effects, finish, view)
                .unwrap();
            renderer.wait_frames(1).unwrap();
        });
    }
    renderer.wait_frames(0).unwrap();
    (renderer, sizes)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_renderer_feeds_dynamic_resolution_from_its_own_gpu_times() {
    let tight = Budget {
        gpu_ms: 1e-4,
        ..Budget::new(1.0)
    };
    let (renderer, sizes) = run(Some(tight), 10);
    let resolution = renderer.dynamic_resolution().unwrap();
    assert_eq!(resolution.scale(), 0.5, "{sizes:?}");
    assert_eq!(resolution.changes(), 1, "{sizes:?}");
    assert!(resolution.cost_ms_at_full().unwrap() > 0.0);
    assert_eq!(sizes[0], (160, 96));
    assert_eq!(*sizes.last().unwrap(), (80, 48));
    assert_eq!(renderer.size(), (80, 48));

    let roomy = Budget {
        gpu_ms: 1e4,
        ..Budget::new(1.0)
    };
    let (renderer, sizes) = run(Some(roomy), 6);
    let resolution = renderer.dynamic_resolution().unwrap();
    assert_eq!(resolution.scale(), 1.0);
    assert_eq!(resolution.changes(), 0);
    assert!(resolution.cost_ms_at_full().unwrap() > 0.0);
    assert!(sizes.iter().all(|size| *size == (160, 96)));

    let (renderer, sizes) = run(None, 2);
    assert!(renderer.dynamic_resolution().is_none());
    assert!(sizes.iter().all(|size| *size == (160, 96)));
}
