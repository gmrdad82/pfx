use super::*;
use crate::block_on;

#[test]
fn every_srgb_code_decodes_and_encodes_back_to_itself() {
    for code in 0..=255_u8 {
        let back = encoded(linear(code));
        assert!((back - f64::from(code)).abs() < 1e-9, "{code}: {back}");
    }
    assert_eq!(linear(0), 0.0);
    assert!((linear(255) - 1.0).abs() < 1e-12);
    assert_eq!(encoded(-1.0), 0.0);
    assert!((encoded(2.0) - 255.0).abs() < 1e-9);
}

#[test]
fn the_reference_mean_averages_light_not_codes_and_counts_weights() {
    let black = [0_u8, 0, 0, 255];
    let white = [255_u8, 255, 255, 255];
    let mean = mean_of(&[(&black, 1), (&white, 1)]);
    let half = encoded(0.5);
    assert!((mean[0] - half).abs() < 1e-9 && half > 187.0 && half < 188.0, "{mean:?}");
    assert!((mean[3] - 255.0).abs() < 1e-9);
    let weighted = mean_of(&[(&black, 3), (&white, 1)]);
    assert!((weighted[1] - encoded(0.25)).abs() < 1e-9);
    let same = mean_of(&[(&white, 2), (&white, 5)]);
    assert!(same.iter().all(|value| (value - 255.0).abs() < 1e-9));
    let clear = [0_u8, 0, 0, 0];
    let alpha = mean_of(&[(&clear, 1), (&white, 1)]);
    assert!((alpha[3] - 127.5).abs() < 1e-9, "alpha averages as stored: {alpha:?}");
    assert!(mean_of(&[]).is_empty());
}

fn uploaded(gpu: &Gpu, size: [u32; 2], rgba: &[u8]) -> OffscreenTarget {
    let target = gpu.offscreen(size[0], size[1], OUT).unwrap();
    gpu.upload_rgba8(&target, rgba).unwrap();
    target
}

fn ramp(size: [u32; 2], shift: u32) -> Vec<u8> {
    (0..size[0] * size[1])
        .flat_map(|index| {
            let value = |salt: u32| ((index * 7 + shift * 31 + salt * 53) % 256) as u8;
            [value(0), value(1), value(2), 255]
        })
        .collect()
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -p pfx-editor-shell -- --ignored"]
fn the_gpu_mean_is_the_linear_mean_of_its_frames_within_one_code() {
    let since = std::time::Instant::now();
    let gpu = block_on(Gpu::headless()).unwrap();
    let size = [61, 37];
    let frames: Vec<Vec<u8>> = (0..5).map(|shift| ramp(size, shift)).collect();
    let weights = [1_u32, 4, 1, 2, 1];
    let mut mean = Mean::new(&gpu, size).unwrap();
    assert_eq!((mean.size(), mean.total()), (size, 0));
    for (frame, weight) in frames.iter().zip(weights) {
        mean.add(&gpu, &uploaded(&gpu, size, frame).view, weight);
    }
    assert_eq!(mean.total(), 9);
    let got = mean.resolve(&gpu).unwrap();
    assert_eq!(mean.total(), 0, "a resolve starts the next frame's sum");
    let parts: Vec<(&[u8], u32)> = frames
        .iter()
        .zip(weights)
        .map(|(frame, weight)| (frame.as_slice(), weight))
        .collect();
    let want = mean_of(&parts);
    let worst = got
        .iter()
        .zip(&want)
        .map(|(got, want)| (f64::from(*got) - want).abs())
        .fold(0.0, f64::max);
    assert!(worst <= 1.0, "the GPU mean is {worst:.3} codes off");
    let one = ramp(size, 9);
    mean.add(&gpu, &uploaded(&gpu, size, &one).view, 1);
    assert_eq!(mean.resolve(&gpu).unwrap(), one, "one frame's mean is the frame");
    mean.add(&gpu, &uploaded(&gpu, size, &one).view, 3);
    let tripled = mean.resolve(&gpu).unwrap();
    assert!(
        tripled.iter().zip(&one).all(|(a, b)| a.abs_diff(*b) <= 1),
        "a weighted frame alone is the frame"
    );
    for frame in &frames {
        mean.add(&gpu, &uploaded(&gpu, size, frame).view, 1);
    }
    let first = mean.resolve(&gpu).unwrap();
    for frame in &frames {
        mean.add(&gpu, &uploaded(&gpu, size, frame).view, 1);
    }
    assert_eq!(mean.resolve(&gpu).unwrap(), first, "the same frames give the same bytes");
    assert!(mean.resolve(&gpu).is_err(), "a mean of nothing is refused");
    mean.clear();
    assert_eq!(mean.total(), 0);
    crate::gpu_turn(since);
}
