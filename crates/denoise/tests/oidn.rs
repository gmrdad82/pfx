use std::time::Instant;

use pfx_denoise::{Denoiser, pfm};
use pfx_gpu::Gpu;

const TOLERANCE: f32 = 1e-3;

fn fixture(part: &str) -> pfm::Pfm {
    let path = format!(
        "{}/tests/fixtures/desk.{part}.pfm",
        env!("CARGO_MANIFEST_DIR")
    );
    pfm::read(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_denoiser_matches_oidn_on_the_desk() {
    let color = fixture("color");
    let albedo = fixture("albedo");
    let normal = fixture("normal");
    let oidn = fixture("oidn");
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut denoiser = Denoiser::new(&gpu).unwrap();
    let run = |denoiser: &mut Denoiser| {
        denoiser
            .denoise(
                &gpu,
                color.width,
                color.height,
                &color.pixels,
                &albedo.pixels,
                &normal.pixels,
            )
            .unwrap()
    };
    let start = Instant::now();
    let first = run(&mut denoiser);
    let cold = start.elapsed();
    let start = Instant::now();
    let ours = run(&mut denoiser);
    let warm = start.elapsed();
    assert_eq!(first, ours);
    let mut worst = 0.0f32;
    for (a, b) in ours.iter().zip(&oidn.pixels) {
        for c in 0..3 {
            worst = worst.max((a[c] - b[c]).abs() / (1.0 + b[c].abs()));
        }
    }
    println!(
        "{}x{} against OIDN {}: worst gap {worst:.2e}, {cold:.1?} with pipelines, {warm:.1?} warm",
        color.width,
        color.height,
        pfx_denoise::OIDN_VERSION
    );
    assert!(worst <= TOLERANCE, "{worst}");
}
