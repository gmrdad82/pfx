use std::time::Instant;

use pfx_denoise::Denoiser;
use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::scene::Scene;
use pfx_trace::Adaptive;

fn desk() -> Scene {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("budgets");
    std::fs::create_dir_all(&root).unwrap();
    for file in [
        "block.gltf",
        "panel.gltf",
        "pillar.gltf",
        "ball.gltf",
        "screen.png",
    ] {
        std::fs::copy(
            manifest.join("../load/tests/scenes").join(file),
            root.join(file),
        )
        .unwrap();
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
    Scene::open(root.join("desk.scene.toml")).unwrap()
}

fn frame(gpu: &Gpu, scene: &Scene, size: [u32; 2], adaptive: Adaptive) -> (f64, f64) {
    let began = Instant::now();
    let staged = pfx_trace::stage::scene_at(scene, size[0] as f32 / size[1] as f32, 0.0).unwrap();
    let mut trace = staged.trace(gpu, size[0], size[1]).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let converged = trace
        .sample_adaptive(gpu, adaptive, 7, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    let mut denoiser = Denoiser::new(gpu).unwrap();
    let clean = denoiser
        .denoise_textures(
            gpu,
            &trace.color.texture,
            &trace.albedo.texture,
            &trace.normal.texture,
        )
        .unwrap();
    assert_eq!(clean.len(), (size[0] * size[1]) as usize);
    (began.elapsed().as_secs_f64(), converged.mean_samples())
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_desk_renders_within_the_budgets() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let scene = desk();
    let final_sampling = Adaptive {
        threshold: 0.01,
        min_samples: 32,
        max_samples: 128,
        growth: 1.5,
    };
    let draft_sampling = Adaptive {
        threshold: 0.05,
        min_samples: 8,
        max_samples: 16,
        growth: 1.5,
    };
    let baseline = baseline();
    for (label, key, size, adaptive, budget) in [
        ("4K draft", "draft_4k", [3840, 2160], draft_sampling, 10.0),
        ("4K final", "final_4k", [3840, 2160], final_sampling, 120.0),
        ("5K final", "final_5k", [5120, 2880], final_sampling, 240.0),
    ] {
        let (first, mean) = frame(&gpu, &scene, size, adaptive);
        let (second, _) = frame(&gpu, &scene, size, adaptive);
        let seconds = first.min(second);
        let base = baseline[key];
        eprintln!(
            "{label}: {}x{} in {seconds:.1} s (budget {budget} s, baseline {base} s), mean {mean:.1} samples",
            size[0], size[1]
        );
        assert!(seconds <= budget, "{label}: {seconds:.1} s");
        assert!(
            seconds <= base * 1.05,
            "{label}: {seconds:.1} s is more than 5% over the baseline {base} s"
        );
    }
}

fn baseline() -> std::collections::HashMap<String, f64> {
    let text = include_str!("baseline.toml");
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| {
            (
                key.trim().to_string(),
                value.trim().parse().expect("a baseline in seconds"),
            )
        })
        .collect()
}
