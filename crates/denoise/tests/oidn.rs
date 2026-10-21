use std::time::Instant;

use pfx_denoise::{Aux, Denoiser, pfm};
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

fn worst_gap(ours: &[[f32; 4]], oidn: &[[f32; 4]]) -> f32 {
    let mut worst = 0.0f32;
    for (a, b) in ours.iter().zip(oidn) {
        for c in 0..3 {
            worst = worst.max((a[c] - b[c]).abs() / (1.0 + b[c].abs()));
        }
    }
    worst
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_clean_aux_denoiser_matches_oidn_on_the_desk() {
    let color = fixture("color");
    let albedo = fixture("albedo");
    let normal = fixture("normal");
    let clean_albedo = fixture("albedo.oidn");
    let clean_normal = fixture("normal.oidn");
    let oidn = fixture("prefiltered.oidn");
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (w, h) = (color.width, color.height);
    let mut prefiltered = Denoiser::with_aux(&gpu, Aux::Prefiltered).unwrap();
    let first = prefiltered
        .denoise(&gpu, w, h, &color.pixels, &albedo.pixels, &normal.pixels)
        .unwrap();
    let ours = prefiltered
        .denoise(&gpu, w, h, &color.pixels, &albedo.pixels, &normal.pixels)
        .unwrap();
    assert_eq!(first, ours);
    let mut clean = Denoiser::with_aux(&gpu, Aux::Clean).unwrap();
    let given = clean
        .denoise(
            &gpu,
            w,
            h,
            &color.pixels,
            &clean_albedo.pixels,
            &clean_normal.pixels,
        )
        .unwrap();
    let mut noisy = Denoiser::new(&gpu).unwrap();
    let plain = noisy
        .denoise(&gpu, w, h, &color.pixels, &albedo.pixels, &normal.pixels)
        .unwrap();
    let prefiltered_gap = worst_gap(&ours, &oidn.pixels);
    let clean_gap = worst_gap(&given, &oidn.pixels);
    println!(
        "{w}x{h} against OIDN {} with clean aux: worst gap {prefiltered_gap:.2e} prefiltered, {clean_gap:.2e} given OIDN's aux",
        pfx_denoise::OIDN_VERSION
    );
    assert!(prefiltered_gap <= TOLERANCE, "{prefiltered_gap}");
    assert!(clean_gap <= TOLERANCE, "{clean_gap}");
    assert!(worst_gap(&plain, &oidn.pixels) > 10.0 * TOLERANCE);
}
