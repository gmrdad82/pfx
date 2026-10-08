use pfx_post::{Chain, Image, Pass, Tone, parse_str};

const FINISH: &str = include_str!("fixtures/finish.toml");

fn finish() -> Chain {
    parse_str(FINISH).unwrap()
}

fn ramp() -> Image {
    Image::from_fn(16, 8, |x, y| {
        let t = x as f32 / 15.0;
        let hot = if x == 12 && y == 4 { 6.0 } else { 0.0 };
        [t * 0.8 + hot, 0.3 + 0.05 * y as f32, 1.0 - t * 0.7, 1.0]
    })
}

#[test]
fn the_neutral_finish_parses_into_its_own_passes() {
    let chain = finish();
    assert_eq!(chain.seed, 11);
    let mut seen = Vec::new();
    for pass in &chain.passes {
        match pass {
            Pass::Exposure(amount) => {
                assert_eq!(*amount, 1.1);
                seen.push("exposure");
            }
            Pass::Bloom(bloom) => {
                assert_eq!(bloom.strength, 0.3);
                assert_eq!(bloom.threshold, 1.2);
                seen.push("bloom");
            }
            Pass::Saturation(saturation) => {
                assert_eq!(saturation.amount, 1.05);
                seen.push("saturation");
            }
            Pass::Contrast(_) => seen.push("contrast"),
            Pass::Tone(tone) => {
                assert!(matches!(tone, Tone::Agx));
                seen.push("tone");
            }
            Pass::Dither(dither) => {
                assert_eq!(dither.amplitude, 0.5);
                seen.push("dither");
            }
            Pass::Encode => seen.push("encode"),
            other => panic!("the neutral finish holds no {other:?}"),
        }
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        [
            "bloom",
            "contrast",
            "dither",
            "encode",
            "exposure",
            "saturation",
            "tone"
        ]
    );
}

#[test]
fn the_neutral_finish_runs_on_the_cpu_the_same_way_twice() {
    let chain = finish();
    let first = chain.apply(&ramp(), None, None);
    let second = finish().apply(&ramp(), None, None);
    assert_eq!(first.pixels, second.pixels);
    assert_eq!((first.width, first.height), (16, 8));
    assert!(
        first
            .pixels
            .iter()
            .flatten()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    );
    let dark = first.pixels[0];
    let bright = first.pixels[15];
    assert!(bright[0] > dark[0], "{bright:?} {dark:?}");
    assert_ne!(first.pixels, ramp().pixels);
    let near = first.pixels[(4 * 16 + 11) as usize];
    let far = first.pixels[(4 * 16 + 2) as usize];
    let plain = parse_str(&FINISH.replace("strength = 0.3", "strength = 0.0"))
        .unwrap()
        .apply(&ramp(), None, None);
    assert!(near[0] >= plain.pixels[(4 * 16 + 11) as usize][0]);
    assert!((far[0] - plain.pixels[(4 * 16 + 2) as usize][0]).abs() < 0.5);
}

#[test]
fn a_finish_with_an_unknown_key_is_refused() {
    let error = parse_str(&format!("{FINISH}glow = 1.0\n")).unwrap_err();
    assert!(error.to_string().contains("glow"));
    let error = parse_str(&FINISH.replace("kind = \"rings\"", "kind = \"halo\"")).unwrap_err();
    assert!(error.to_string().contains("halo"));
}
