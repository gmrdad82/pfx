use super::{Format, NonFinite, NumberError, Precision, Rounding, Style, Suffixes};
use crate::sim::Rng;
use std::fmt;

fn f64s(format: &Format, v: f64) -> String {
    format.format(v).unwrap()
}

fn base() -> Format<'static> {
    Format::default()
}

#[test]
fn grouping_and_marks() {
    let f = base();
    assert_eq!(f.format(1234567).unwrap(), "1,234,567.00");
    assert_eq!(f.format(-1234567i64).unwrap(), "-1,234,567.00");
    assert_eq!(f.format(0u8).unwrap(), "0.00");
    let f = Format {
        group_sep: Some(' '),
        precision: Precision::Decimals(0),
        ..base()
    };
    assert_eq!(f.format(1234567).unwrap(), "1 234 567");
    assert_eq!(f.format(999).unwrap(), "999");
    let f = Format {
        group_sep: None,
        ..base()
    };
    assert_eq!(f.format(1234.5).unwrap(), "1234.50");
    let f = Format {
        group_sep: Some('.'),
        decimal_mark: ',',
        ..base()
    };
    assert_eq!(f64s(&f, 1234.5), "1.234,50");
    let f = Format {
        group_size: 4,
        precision: Precision::Decimals(0),
        ..base()
    };
    assert_eq!(f.format(12345678).unwrap(), "1234,5678");
}

#[test]
fn integers_are_exact() {
    let f = Format {
        precision: Precision::Decimals(0),
        group_sep: None,
        ..base()
    };
    assert_eq!(
        f.format(i128::MIN).unwrap(),
        "-170141183460469231731687303715884105728"
    );
    assert_eq!(
        f.format(u128::MAX).unwrap(),
        "340282366920938463463374607431768211455"
    );
    assert_eq!(f.format(i64::MIN).unwrap(), "-9223372036854775808");
    assert_eq!(f.format(u64::MAX).unwrap(), "18446744073709551615");
}

#[test]
fn rounding_modes() {
    let away = Format {
        precision: Precision::Decimals(0),
        ..base()
    };
    let even = Format {
        rounding: Rounding::HalfEven,
        ..away
    };
    assert_eq!(f64s(&away, 2.5), "3");
    assert_eq!(f64s(&even, 2.5), "2");
    assert_eq!(f64s(&even, 3.5), "4");
    assert_eq!(f64s(&away, 0.5), "1");
    assert_eq!(f64s(&even, 0.5), "0");
    assert_eq!(f64s(&away, 0.49), "0");
    let one = |r| Format {
        precision: Precision::Decimals(1),
        rounding: r,
        ..base()
    };
    assert_eq!(f64s(&one(Rounding::HalfAway), 0.05), "0.1");
    assert_eq!(f64s(&one(Rounding::HalfEven), 0.05), "0.0");
    assert_eq!(f64s(&one(Rounding::HalfEven), 0.15), "0.2");
    assert_eq!(f64s(&one(Rounding::HalfEven), 0.25), "0.2");
    let two = |r| Format {
        rounding: r,
        ..base()
    };
    assert_eq!(f64s(&two(Rounding::HalfAway), 2.675), "2.68");
    assert_eq!(f64s(&two(Rounding::HalfEven), 2.665), "2.66");
    assert_eq!(f64s(&two(Rounding::HalfAway), 2.665), "2.67");
    assert_eq!(f64s(&two(Rounding::HalfEven), 1.005), "1.00");
    assert_eq!(f64s(&base(), 0.1 + 0.2), "0.30");
}

#[test]
fn significant_digits_and_trim() {
    let sig = |s, trim| Format {
        precision: Precision::Significant(s),
        trim_zeros: trim,
        ..base()
    };
    assert_eq!(sig(3, false).format(1234567).unwrap(), "1,230,000");
    assert_eq!(f64s(&sig(3, false), 0.001234), "0.00123");
    assert_eq!(f64s(&sig(3, false), 1.5), "1.50");
    assert_eq!(f64s(&sig(3, true), 1.5), "1.5");
    assert_eq!(f64s(&sig(2, false), 9.99), "10");
    assert_eq!(f64s(&sig(17, false), 0.1 + 0.2), "0.30000000000000004");
    assert_eq!(f64s(&sig(0, false), 7.7), "8");
    assert_eq!(f64s(&sig(3, false), 0.0), "0.00");
    let trim = Format {
        trim_zeros: true,
        ..base()
    };
    assert_eq!(trim.format(1234567).unwrap(), "1,234,567");
    assert_eq!(f64s(&trim, 1234.5), "1,234.5");
    assert_eq!(f64s(&trim, 0.0), "0");
}

#[test]
fn negatives_and_zero() {
    let f = base();
    assert_eq!(f64s(&f, -1234.5), "-1,234.50");
    assert_eq!(f64s(&f, -0.001), "0.00");
    assert_eq!(f64s(&f, -0.0), "0.00");
    assert_eq!(f64s(&f, 0.004), "0.00");
    assert_eq!(f64s(&f, 0.005), "0.01");
}

#[test]
fn scientific_and_engineering() {
    let sci = Format {
        style: Style::Scientific,
        from_exp: 6,
        ..base()
    };
    assert_eq!(sci.format(123456).unwrap(), "123,456.00");
    assert_eq!(sci.format(1234567).unwrap(), "1.23e6");
    assert_eq!(f64s(&sci, 1.23e15), "1.23e15");
    assert_eq!(f64s(&sci, 999999.999), "1.00e6");
    assert_eq!(f64s(&sci, -4.5e20), "-4.50e20");
    let trim = Format {
        trim_zeros: true,
        ..sci
    };
    assert_eq!(f64s(&trim, 1e30), "1e30");
    let eng = Format {
        style: Style::Engineering,
        from_exp: 6,
        precision: Precision::Significant(3),
        ..base()
    };
    assert_eq!(f64s(&eng, 1.2345e16), "12.3e15");
    assert_eq!(f64s(&eng, 1.2345e18), "1.23e18");
    assert_eq!(f64s(&eng, 1.2345e17), "123e15");
    assert_eq!(f64s(&eng, 9.996e17), "1.00e18");
    let low = Format {
        style: Style::Engineering,
        from_exp: -9,
        precision: Precision::Significant(3),
        ..base()
    };
    assert_eq!(f64s(&low, 0.05), "50.0e-3");
}

#[test]
fn short_suffixes() {
    let short = Format {
        style: Style::Short,
        precision: Precision::Decimals(1),
        ..base()
    };
    assert_eq!(short.format(999).unwrap(), "999.0");
    assert_eq!(short.format(1500).unwrap(), "1.5K");
    assert_eq!(short.format(4_500_000).unwrap(), "4.5M");
    assert_eq!(short.format(2_000_000_000u64).unwrap(), "2.0B");
    assert_eq!(f64s(&short, 1e12), "1.0T");
    assert_eq!(f64s(&short, 999.96), "1.0K");
    assert_eq!(f64s(&short, 999_950.0), "1.0M");
    assert_eq!(f64s(&short, -12_345.0), "-12.3K");
    let trim = Format {
        trim_zeros: true,
        ..short
    };
    assert_eq!(trim.format(1_000_000).unwrap(), "1M");
    assert_eq!(f64s(&trim, 1e15), "1e15");
    let late = Format {
        from_exp: 6,
        ..short
    };
    assert_eq!(late.format(123456).unwrap(), "123,456.0");
    assert_eq!(late.format(1234567).unwrap(), "1.2M");
    let sig = Format {
        precision: Precision::Significant(3),
        ..short
    };
    assert_eq!(sig.format(1_234_567).unwrap(), "1.23M");
    assert_eq!(sig.format(123_456).unwrap(), "123K");
    assert_eq!(sig.format(12_345).unwrap(), "12.3K");
}

#[test]
fn short_beyond_the_list() {
    let alpha = Format {
        style: Style::Short,
        suffixes: Suffixes::Alpha,
        ..base()
    };
    assert_eq!(f64s(&alpha, 1e15), "1.00aa");
    assert_eq!(f64s(&alpha, 1e18), "1.00ab");
    assert_eq!(f64s(&alpha, 1e12), "1.00T");
    assert_eq!(f64s(&alpha, 1e3 * 1e3f64.powi(4 + 26)), "1.00ba");
    let game = Format {
        style: Style::Short,
        suffixes: Suffixes::Named(&["k", "m"]),
        ..base()
    };
    assert_eq!(game.format(2_500_000).unwrap(), "2.50m");
    assert_eq!(game.format(2_500_000_000u64).unwrap(), "2.50e9");
}

#[test]
fn non_finite_policies() {
    let text = base();
    assert_eq!(f64s(&text, f64::NAN), "NaN");
    assert_eq!(f64s(&text, f64::INFINITY), "inf");
    assert_eq!(f64s(&text, f64::NEG_INFINITY), "-inf");
    assert_eq!(text.format(f32::NAN).unwrap(), "NaN");
    let own = Format {
        nan_text: "?",
        inf_text: "∞",
        ..base()
    };
    assert_eq!(f64s(&own, f64::NEG_INFINITY), "-∞");
    assert_eq!(f64s(&own, f64::NAN), "?");
    let clamp = Format {
        non_finite: NonFinite::Clamp,
        style: Style::Scientific,
        ..base()
    };
    assert_eq!(f64s(&clamp, f64::INFINITY), "1.80e308");
    assert_eq!(f64s(&clamp, f64::NEG_INFINITY), "-1.80e308");
    assert_eq!(f64s(&clamp, f64::NAN), "0.00");
    assert_eq!(clamp.format(f32::INFINITY).unwrap(), "3.40e38");
    let error = Format {
        non_finite: NonFinite::Error,
        ..base()
    };
    assert_eq!(error.format(f64::NAN), Err(NumberError::NotANumber));
    assert_eq!(error.format(f64::INFINITY), Err(NumberError::Infinite));
    assert_eq!(error.format(f32::NEG_INFINITY), Err(NumberError::Infinite));
    assert_eq!(error.format(5), Ok("5.00".to_string()));
}

#[test]
fn f32_uses_its_own_shortest_digits() {
    let f = Format {
        precision: Precision::Significant(9),
        trim_zeros: true,
        ..base()
    };
    assert_eq!(f.format(0.1f32).unwrap(), "0.1");
    assert_eq!(f.format(16777216f32).unwrap(), "16,777,216");
    assert_eq!(f64s(&f, f64::from(0.1f32)), "0.100000001");
}

struct Refuse;

impl fmt::Write for Refuse {
    fn write_str(&mut self, _: &str) -> fmt::Result {
        Err(fmt::Error)
    }
}

#[test]
fn a_refusing_writer_is_an_error() {
    assert_eq!(base().write(&mut Refuse, 12), Err(NumberError::Write));
    assert_eq!(base().write(&mut Refuse, f64::NAN), Err(NumberError::Write));
}

#[test]
fn a_buffer_is_reused() {
    let f = base();
    let mut text = String::new();
    f.write(&mut text, 1).unwrap();
    text.push('|');
    f.write(&mut text, 2.5).unwrap();
    assert_eq!(text, "1.00|2.50");
}

fn mixed(rng: &mut Rng, count: usize) -> (Vec<i64>, Vec<f64>) {
    let ints = (0..count)
        .map(|_| (rng.next_u64() >> (1 + rng.below(63))) as i64)
        .collect();
    let floats = (0..count)
        .map(|_| {
            let scale = 10f64.powi(rng.range_i32(-6, 24));
            (rng.next_f64() * 1000.0) * scale
        })
        .collect();
    (ints, floats)
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_styles() {
    let count = 100_000;
    let mut rng = Rng::new(7);
    let (ints, floats) = mixed(&mut rng, count);
    let styles = [
        ("plain", Style::Plain),
        ("scientific", Style::Scientific),
        ("engineering", Style::Engineering),
        ("short", Style::Short),
    ];
    for (name, style) in styles {
        let f = Format {
            style,
            from_exp: 3,
            precision: Precision::Significant(4),
            trim_zeros: true,
            ..base()
        };
        let mut text = String::with_capacity(64);
        let mut total = 0usize;
        let start = std::time::Instant::now();
        for i in 0..count {
            text.clear();
            f.write(&mut text, ints[i]).unwrap();
            total += text.len();
            text.clear();
            f.write(&mut text, floats[i]).unwrap();
            total += text.len();
        }
        let elapsed = start.elapsed();
        println!(
            "{name}: {:.1} ns per call ({} bytes)",
            elapsed.as_nanos() as f64 / (2 * count) as f64,
            total
        );
    }
}

fn long_style() -> Format<'static> {
    Format {
        style: Style::Scientific,
        from_exp: 12,
        precision: Precision::Decimals(0),
        big_precision: Some(Precision::Significant(2)),
        rounding: Rounding::Truncate,
        ..base()
    }
}

fn card_style() -> Format<'static> {
    Format {
        style: Style::Short,
        from_exp: 4,
        sci_from: Some(12),
        precision: Precision::Decimals(0),
        big_precision: Some(Precision::AtLeast(2)),
        rounding: Rounding::Truncate,
        ..base()
    }
}

#[test]
fn long_style_groups_then_goes_scientific() {
    let f = long_style();
    assert_eq!(f.format(999_999_999_999u64).unwrap(), "999,999,999,999");
    assert_eq!(f.format(0).unwrap(), "0");
    assert_eq!(f.format(1_000_000_000_000u64).unwrap(), "1.0e12");
    assert_eq!(f.format(1_990_000_000_000u64).unwrap(), "1.9e12");
    assert_eq!(f.format(-2_500_000_000_000i64).unwrap(), "-2.5e12");
    assert_eq!(f64s(&f, 1.5e30), "1.5e30");
}

#[test]
fn card_style_truncates_to_short_names() {
    let f = card_style();
    assert_eq!(f.format(999).unwrap(), "999");
    assert_eq!(f.format(9_999).unwrap(), "9,999");
    assert_eq!(f.format(12_000).unwrap(), "12K");
    assert_eq!(f.format(12_999).unwrap(), "12K");
    assert_eq!(f.format(999_999).unwrap(), "999K");
    assert_eq!(f.format(3_450_000).unwrap(), "3.4M");
    assert_eq!(f.format(34_500_000).unwrap(), "34M");
    assert_eq!(f.format(999_999_999_999u64).unwrap(), "999B");
    assert_eq!(f.format(1_234_000_000_000u64).unwrap(), "1.2e12");
    assert_eq!(f.format(1_000_000_000).unwrap(), "1.0B");
    let trim = Format {
        trim_zeros: true,
        ..f
    };
    assert_eq!(trim.format(3_000_000).unwrap(), "3M");
    assert_eq!(f64s(&f, 9_999.9), "9,999");
}

#[test]
fn ceil_and_truncate() {
    let ceil = Format {
        rounding: Rounding::Ceil,
        precision: Precision::Decimals(0),
        ..base()
    };
    let cut = Format {
        rounding: Rounding::Truncate,
        ..ceil
    };
    assert_eq!(f64s(&ceil, 0.0001), "1");
    assert_eq!(f64s(&ceil, 0.0), "0");
    assert_eq!(f64s(&ceil, 2.0), "2");
    assert_eq!(f64s(&ceil, 2.01), "3");
    assert_eq!(f64s(&ceil, -2.9), "-2");
    assert_eq!(f64s(&ceil, -0.5), "0");
    assert_eq!(f64s(&cut, 2.99), "2");
    assert_eq!(f64s(&cut, -2.99), "-2");
    assert_eq!(f64s(&cut, 0.99), "0");
    let two = Format {
        precision: Precision::Decimals(2),
        ..ceil
    };
    assert_eq!(f64s(&two, 0.0004), "0.01");
    assert_eq!(f64s(&two, 1.231), "1.24");
    assert_eq!(f64s(&two, 1.23), "1.23");
    let live = Format {
        style: Style::Short,
        from_exp: 4,
        ..ceil
    };
    assert_eq!(f64s(&live, 9_999.2), "10K");
}

#[test]
fn one_text_for_every_non_finite() {
    let f = Format {
        non_finite: NonFinite::OneText,
        ..base()
    };
    assert_eq!(f64s(&f, f64::NAN), "NaN");
    assert_eq!(f64s(&f, f64::INFINITY), "NaN");
    assert_eq!(f64s(&f, f64::NEG_INFINITY), "NaN");
    assert_eq!(f.format(f32::NEG_INFINITY).unwrap(), "NaN");
}
