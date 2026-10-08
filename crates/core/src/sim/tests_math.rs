use super::*;
use std::collections::BTreeMap;

const F64_TABLE: &str = include_str!("golden/math.txt");
const F32_TABLE: &str = include_str!("golden/math32.txt");

fn parse(h: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(h, 16).unwrap())
}

fn ulp(x: f64) -> f64 {
    let e = (x.abs().to_bits() >> 52) as i64;
    if e <= 52 {
        f64::from_bits(1u64 << e.max(1).saturating_sub(1))
    } else {
        f64::from_bits(((e - 52) as u64) << 52)
    }
}

fn ulp32(x: f64) -> f64 {
    let e = (x.abs().to_bits() >> 52) as i64;
    f64::from_bits(((e - 23).max(1) as u64) << 52).max(f64::from_bits(((1023 - 149) as u64) << 52))
}

struct Row {
    name: &'static str,
    args: Vec<f64>,
    hi: f64,
    lo: f64,
    gold: u64,
}

fn rows(table: &'static str) -> Vec<Row> {
    table
        .lines()
        .map(|line| {
            let p: Vec<&str> = line.split(' ').collect();
            let name = match p[0] {
                "exp" => "exp",
                "ln" => "ln",
                "sin" => "sin",
                "cos" => "cos",
                "pow" => "pow",
                "atan2" => "atan2",
                other => panic!("unknown function {other}"),
            };
            let n = if name == "pow" || name == "atan2" {
                2
            } else {
                1
            };
            Row {
                name,
                args: p[1..1 + n].iter().map(|h| parse(h)).collect(),
                hi: parse(p[1 + n]),
                lo: parse(p[2 + n]),
                gold: u64::from_str_radix(p[3 + n], 16).unwrap(),
            }
        })
        .collect()
}

fn eval64(name: &str, a: &[f64]) -> f64 {
    match name {
        "exp" => exp(a[0]),
        "ln" => ln(a[0]),
        "sin" => sin(a[0]),
        "cos" => cos(a[0]),
        "pow" => pow(a[0], a[1]),
        "atan2" => atan2(a[0], a[1]),
        _ => unreachable!(),
    }
}

fn eval32(name: &str, a: &[f64]) -> f32 {
    let a: Vec<f32> = a.iter().map(|&v| v as f32).collect();
    match name {
        "exp" => expf(a[0]),
        "ln" => lnf(a[0]),
        "sin" => sinf(a[0]),
        "cos" => cosf(a[0]),
        "pow" => powf(a[0], a[1]),
        "atan2" => atan2f(a[0], a[1]),
        _ => unreachable!(),
    }
}

#[test]
fn f64_outputs_match_their_golden_bits() {
    let rows = rows(F64_TABLE);
    assert_eq!(rows.len(), 1236);
    for row in &rows {
        let out = eval64(row.name, &row.args);
        assert_eq!(
            out.to_bits(),
            row.gold,
            "{} {:?}: got {:e}",
            row.name,
            row.args,
            out
        );
    }
}

#[test]
fn f64_functions_stay_within_one_ulp_of_the_reference() {
    let mut worst: BTreeMap<&str, f64> = BTreeMap::new();
    for row in rows(F64_TABLE) {
        let out = eval64(row.name, &row.args);
        let err = ((out - row.hi) - row.lo).abs() / ulp(row.hi);
        assert!(
            err <= 1.0,
            "{} {:?}: {err} ulp (got {out:e}, want {:e})",
            row.name,
            row.args,
            row.hi
        );
        let slot = worst.entry(row.name).or_insert(0.0);
        *slot = slot.max(err);
    }
    assert_eq!(worst.len(), 6);
}

#[test]
fn f32_outputs_match_their_golden_bits() {
    let rows = rows(F32_TABLE);
    assert_eq!(rows.len(), 984);
    for row in &rows {
        let out = eval32(row.name, &row.args);
        assert_eq!(
            out.to_bits() as u64,
            row.gold,
            "{} {:?}: got {:e}",
            row.name,
            row.args,
            out
        );
    }
}

#[test]
fn f32_functions_round_the_reference_to_within_half_an_ulp_and_a_hair() {
    for row in rows(F32_TABLE) {
        if row.hi.abs() < f32::MIN_POSITIVE as f64 || row.hi.abs() > f32::MAX as f64 {
            continue;
        }
        let out = eval32(row.name, &row.args) as f64;
        let err = ((out - row.hi) - row.lo).abs() / ulp32(row.hi);
        assert!(
            err <= 0.501,
            "{} {:?}: {err} ulp32 (got {out:e}, want {:e})",
            row.name,
            row.args,
            row.hi
        );
    }
}

fn close_to_std(got: f64, want: f64, ulps: f64) -> bool {
    if got.is_nan() || want.is_nan() {
        return got.is_nan() && want.is_nan();
    }
    got == want || (got - want).abs() <= ulps * ulp(want)
}

#[test]
fn sweeps_agree_with_std_to_a_few_ulp() {
    let mut rng = Rng::new(0x51d_a7a);
    for _ in 0..20000 {
        let x = rng.range_f64(-745.0, 709.0);
        assert!(close_to_std(exp(x), x.exp(), 4.0), "exp {x}");
        let x = rng.range_f64(-60.0, 60.0);
        assert!(close_to_std(sin(x), x.sin(), 4.0), "sin {x}");
        assert!(close_to_std(cos(x), x.cos(), 4.0), "cos {x}");
        let x = rng.range_f64(-1.0e7, 1.0e7);
        assert!(close_to_std(sin(x), x.sin(), 4.0), "sin {x}");
        assert!(close_to_std(cos(x), x.cos(), 4.0), "cos {x}");
        let x = f64::from_bits(rng.next_u64() & 0x7fef_ffff_ffff_ffff);
        assert!(close_to_std(ln(x), x.ln(), 4.0), "ln {x:e}");
        let (a, b) = (rng.range_f64(-50.0, 50.0), rng.range_f64(-50.0, 50.0));
        assert!(close_to_std(atan2(a, b), a.atan2(b), 4.0), "atan2 {a} {b}");
        let (a, b) = (rng.range_f64(0.001, 1000.0), rng.range_f64(-12.0, 12.0));
        assert!(close_to_std(pow(a, b), a.powf(b), 4.0), "pow {a} {b}");
    }
}

#[test]
fn trigonometric_identities_hold_over_a_sweep() {
    let mut rng = Rng::new(99);
    for _ in 0..5000 {
        let x = rng.range_f64(-1000.0, 1000.0);
        let s = sin(x);
        let c = cos(x);
        assert!((s * s + c * c - 1.0).abs() < 4.0e-16, "{x}");
        assert_eq!(sin(-x).to_bits(), (-s).to_bits());
        assert_eq!(cos(-x).to_bits(), c.to_bits());
        let a = atan2(s, c);
        let wrapped = (a - x).rem_euclid(core::f64::consts::TAU);
        assert!(
            wrapped < 1.0e-12 || core::f64::consts::TAU - wrapped < 1.0e-12,
            "{x}"
        );
    }
}

#[test]
fn exp_and_ln_are_inverse_to_a_few_ulp() {
    let mut rng = Rng::new(5);
    for _ in 0..5000 {
        let x = rng.range_f64(-700.0, 700.0);
        let back = ln(exp(x));
        assert!((back - x).abs() <= 2.0e-16 * x.abs().max(1.0), "{x}");
    }
}

#[test]
fn exp_special_values() {
    assert_eq!(exp(0.0), 1.0);
    assert_eq!(exp(-0.0), 1.0);
    assert_eq!(exp(f64::INFINITY), f64::INFINITY);
    assert_eq!(exp(f64::NEG_INFINITY), 0.0);
    assert!(exp(f64::NAN).is_nan());
    assert!(exp(709.0).is_finite() && exp(709.0) > 8.0e307);
    assert_eq!(exp(710.0), f64::INFINITY);
    assert_eq!(exp(1.0e300), f64::INFINITY);
    assert_eq!(exp(-1.0e300), 0.0);
    assert_eq!(exp(-746.0), 0.0);
    assert_eq!(exp(-745.0).to_bits(), 1);
    assert!(exp(-740.0) > 0.0 && exp(-740.0) < f64::MIN_POSITIVE);
    assert_eq!(exp(1.0).to_bits(), core::f64::consts::E.to_bits());
}

#[test]
fn ln_special_values() {
    assert_eq!(ln(1.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(ln(0.0), f64::NEG_INFINITY);
    assert_eq!(ln(-0.0), f64::NEG_INFINITY);
    assert_eq!(ln(f64::INFINITY), f64::INFINITY);
    assert!(ln(-1.0).is_nan());
    assert!(ln(f64::NEG_INFINITY).is_nan());
    assert!(ln(f64::NAN).is_nan());
    assert_eq!(ln(core::f64::consts::E).to_bits(), 1.0f64.to_bits());
    assert_eq!(
        ln(f64::from_bits(1)).to_bits(),
        (-744.4400719213812f64).to_bits()
    );
    assert_eq!(ln(f64::MAX).to_bits(), 709.782712893384f64.to_bits());
}

#[test]
fn sin_cos_special_values() {
    assert_eq!(sin(0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(sin(-0.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(cos(0.0), 1.0);
    assert_eq!(cos(-0.0), 1.0);
    for bad in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        assert!(sin(bad).is_nan());
        assert!(cos(bad).is_nan());
    }
    assert_eq!(sin(f64::from_bits(1)).to_bits(), 1);
    assert!(cos(f64::MAX).abs() <= 1.0);
    assert!(sin(f64::MAX).abs() <= 1.0);
}

#[test]
fn atan2_special_values() {
    use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    let inf = f64::INFINITY;
    assert_eq!(atan2(0.0, 1.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(atan2(-0.0, 1.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(atan2(0.0, -1.0), PI);
    assert_eq!(atan2(-0.0, -1.0), -PI);
    assert_eq!(atan2(0.0, 0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(atan2(0.0, -0.0), PI);
    assert_eq!(atan2(-0.0, -0.0), -PI);
    assert_eq!(atan2(1.0, 0.0), FRAC_PI_2);
    assert_eq!(atan2(-1.0, -0.0), -FRAC_PI_2);
    assert_eq!(atan2(1.0, 1.0), FRAC_PI_4);
    assert_eq!(atan2(inf, inf), FRAC_PI_4);
    assert_eq!(atan2(inf, -inf), 3.0 * FRAC_PI_4);
    assert_eq!(atan2(-inf, -inf), -3.0 * FRAC_PI_4);
    assert_eq!(atan2(inf, 5.0), FRAC_PI_2);
    assert_eq!(atan2(5.0, inf).to_bits(), 0.0f64.to_bits());
    assert_eq!(atan2(5.0, -inf), PI);
    assert_eq!(atan2(-5.0, -inf), -PI);
    assert!(atan2(f64::NAN, 1.0).is_nan());
    assert!(atan2(1.0, f64::NAN).is_nan());
    assert_eq!(atan2(f64::from_bits(1), 1.0).to_bits(), 1);
}

#[test]
fn pow_special_values() {
    let inf = f64::INFINITY;
    assert_eq!(pow(f64::NAN, 0.0), 1.0);
    assert_eq!(pow(1.0, f64::NAN), 1.0);
    assert_eq!(pow(2.0, 0.0), 1.0);
    assert!(pow(f64::NAN, 2.0).is_nan());
    assert!(pow(2.0, f64::NAN).is_nan());
    assert_eq!(pow(0.0, 3.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(pow(-0.0, 3.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(pow(-0.0, 2.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(pow(0.0, -1.0), inf);
    assert_eq!(pow(-0.0, -1.0), -inf);
    assert_eq!(pow(-0.0, -2.0), inf);
    assert_eq!(pow(-1.0, inf), 1.0);
    assert_eq!(pow(2.0, inf), inf);
    assert_eq!(pow(2.0, -inf), 0.0);
    assert_eq!(pow(0.5, inf), 0.0);
    assert_eq!(pow(0.5, -inf), inf);
    assert_eq!(pow(inf, 2.0), inf);
    assert_eq!(pow(inf, -2.0), 0.0);
    assert_eq!(pow(-inf, 3.0), -inf);
    assert_eq!(pow(-inf, 2.0), inf);
    assert_eq!(pow(-inf, -3.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(pow(-inf, -2.0).to_bits(), 0.0f64.to_bits());
    assert!(pow(-2.0, 0.5).is_nan());
    assert_eq!(pow(-2.0, 3.0), -8.0);
    assert_eq!(pow(-2.0, 4.0), 16.0);
    assert_eq!(pow(-3.0, 2.0), 9.0);
    assert_eq!(pow(2.0, 10.0), 1024.0);
    assert_eq!(pow(2.0, -1074.0).to_bits(), 1);
    assert_eq!(pow(2.0, 1024.0), inf);
    assert_eq!(pow(2.0, -1080.0), 0.0);
    assert_eq!(pow(1.5, 1.0), 1.5);
    assert_eq!(pow(10.0, 400.0), inf);
    assert_eq!(pow(10.0, -400.0), 0.0);
    assert_eq!(pow(1.0 + f64::EPSILON, 1.0e300), inf);
    assert_eq!(pow(1.0 - f64::EPSILON / 2.0, 1.0e300), 0.0);
    assert_eq!(pow(4.0, 0.5), 2.0);
}

#[test]
fn sqrt_is_exact() {
    assert_eq!(sqrt(4.0), 2.0);
    assert_eq!(sqrt(0.25), 0.5);
    assert_eq!(sqrt(2.0).to_bits(), 0x3ff6_a09e_667f_3bcd);
    assert_eq!(sqrt(-0.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(sqrt(f64::INFINITY), f64::INFINITY);
    assert!(sqrt(-1.0).is_nan());
    assert!(sqrt(f64::NAN).is_nan());
    assert_eq!(sqrtf(2.0).to_bits(), 0x3fb5_04f3);
    assert_eq!(sqrtf(9.0), 3.0);
    assert!(sqrtf(-1.0).is_nan());
}

#[test]
fn f32_special_values() {
    assert_eq!(expf(0.0), 1.0);
    assert_eq!(expf(89.0), f32::INFINITY);
    assert_eq!(expf(-104.0), 0.0);
    assert_eq!(lnf(1.0).to_bits(), 0.0f32.to_bits());
    assert_eq!(lnf(0.0), f32::NEG_INFINITY);
    assert!(lnf(-1.0).is_nan());
    assert_eq!(sinf(0.0).to_bits(), 0.0f32.to_bits());
    assert_eq!(cosf(0.0), 1.0);
    assert!(sinf(f32::INFINITY).is_nan());
    assert_eq!(powf(2.0, 10.0), 1024.0);
    assert_eq!(powf(2.0, 128.0), f32::INFINITY);
    assert_eq!(
        atan2f(1.0, 1.0).to_bits(),
        core::f32::consts::FRAC_PI_4.to_bits()
    );
    assert!(atan2f(f32::NAN, 1.0).is_nan());
}

#[test]
fn nan_results_are_canonical() {
    let canonical = f64::NAN.to_bits();
    let weird = f64::from_bits(0x7ff8_0000_dead_beef);
    assert_eq!(sin(weird).to_bits(), canonical);
    assert_eq!(cos(weird).to_bits(), canonical);
    assert_eq!(exp(weird).to_bits(), canonical);
    assert_eq!(ln(weird).to_bits(), canonical);
    assert_eq!(pow(weird, 2.0).to_bits(), canonical);
    assert_eq!(atan2(weird, 1.0).to_bits(), canonical);
    assert_eq!(sqrt(weird).to_bits(), canonical);
}
