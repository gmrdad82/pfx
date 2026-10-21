#[path = "support/street.rs"]
mod street;

use std::path::Path;

use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::scene::Scene;
use street::{Street, fnv};

const SIZE: [u32; 2] = [160, 90];
const SAMPLES: u32 = 2;
const SEED: u32 = 5;

fn hashes(gpu: &Gpu, scene: &Scene) -> [u64; 3] {
    let staged = pfx_trace::stage::scene(scene, SIZE[0] as f32 / SIZE[1] as f32).unwrap();
    let mut trace = staged.trace(gpu, SIZE[0], SIZE[1]).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    for pass in 0..SAMPLES {
        trace
            .sample_paced(gpu, 1, SEED.wrapping_add(pass), &mut pacer, |ms| {
                turns.add(ms)
            })
            .unwrap();
    }
    turns.turn();
    let output = trace.readback(gpu).unwrap();
    [fnv(&output.color), fnv(&output.albedo), fnv(&output.normal)]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_draw_distance_leaves_traced_frames_as_they_were() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let plain = street::scene(base, "distance-plain", Street::default());
    let root = street::folder(base, "distance-near", Street::default());
    let file = root.join("street.scene.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("[[object]]\n"));
    std::fs::write(
        &file,
        text.replace("[[object]]\n", "[[object]]\nmax_distance = 0.5\n"),
    )
    .unwrap();
    let near = Scene::open(&file).unwrap();
    assert!(near.objects.iter().all(|object| object.max_distance == 0.5));
    assert!(
        plain
            .objects
            .iter()
            .all(|object| object.max_distance == f32::INFINITY)
    );
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    assert_eq!(hashes(&gpu, &plain), hashes(&gpu, &near));
}
