const MANT_MASK: u64 = 0x000f_ffff_ffff_ffff;
const ONE_EXP: u64 = 0x3ff0_0000_0000_0000;

const PIO2_P1: f64 = f64::from_bits(0x3ff921fb54400000);
const PIO2_P2: f64 = f64::from_bits(0x3dd0b4611a600000);
const PIO2_P3: f64 = f64::from_bits(0x3ba3198a2e000000);
const PIO2_P4: f64 = f64::from_bits(0x397b839a252049c1);
const PI_DD: Dd = Dd {
    hi: f64::from_bits(0x400921fb54442d18),
    lo: f64::from_bits(0x3ca1a62633145c07),
};
const HALF_PI_DD: Dd = Dd {
    hi: f64::from_bits(0x3ff921fb54442d18),
    lo: f64::from_bits(0x3c91a62633145c07),
};
const QUARTER_PI_DD: Dd = Dd {
    hi: f64::from_bits(0x3fe921fb54442d18),
    lo: f64::from_bits(0x3c81a62633145c07),
};
const THREE_QUARTER_PI_DD: Dd = Dd {
    hi: f64::from_bits(0x4002d97c7f3321d2),
    lo: f64::from_bits(0x3c9a79394c9e8a0a),
};
const LN2_HI: f64 = f64::from_bits(0x3fe62e42fec00000);
const LN2_MID: f64 = f64::from_bits(0x3dfd1cf79a800000);
const LN2_LO: f64 = f64::from_bits(0x3c0e4f1d9cc01f98);
const LN2_DD: Dd = Dd {
    hi: f64::from_bits(0x3fe62e42fefa39ef),
    lo: f64::from_bits(0x3c7abc9e3b39803f),
};
const THIRD_DD: Dd = Dd {
    hi: f64::from_bits(0x3fd5555555555555),
    lo: f64::from_bits(0x3c75555555555555),
};
const FIFTH_DD: Dd = Dd {
    hi: f64::from_bits(0x3fc999999999999a),
    lo: f64::from_bits(0xbc6999999999999a),
};
const ATAN_TABLE: [Dd; 8] = [
    Dd {
        hi: f64::from_bits(0x3fbfd5ba9aac2f6e),
        lo: f64::from_bits(0xbc4cd37686760c17),
    },
    Dd {
        hi: f64::from_bits(0x3fcf5b75f92c80dd),
        lo: f64::from_bits(0x3c68ab6e3cf7afbd),
    },
    Dd {
        hi: f64::from_bits(0x3fd6f61941e4def1),
        lo: f64::from_bits(0xbc7c63aae6f6e918),
    },
    Dd {
        hi: f64::from_bits(0x3fddac670561bb4f),
        lo: f64::from_bits(0x3c7a2b7f222f65e2),
    },
    Dd {
        hi: f64::from_bits(0x3fe1e00babdefeb4),
        lo: f64::from_bits(0xbc5928df287a668f),
    },
    Dd {
        hi: f64::from_bits(0x3fe4978fa3269ee1),
        lo: f64::from_bits(0x3c72419a87f2a458),
    },
    Dd {
        hi: f64::from_bits(0x3fe700a7c5784634),
        lo: f64::from_bits(0xbc78c34d25aadef6),
    },
    Dd {
        hi: f64::from_bits(0x3fe921fb54442d18),
        lo: f64::from_bits(0x3c81a62633145c07),
    },
];
const TWO_OVER_PI: [u64; 20] = [
    0xa2f9836e4e441529,
    0xfc2757d1f534ddc0,
    0xdb6295993c439041,
    0xfe5163abdebbc561,
    0xb7246e3a424dd2e0,
    0x06492eea09d1921c,
    0xfe1deb1cb129a73e,
    0xe88235f52ebb4484,
    0xe99c7026b45f7e41,
    0x3991d639835339f4,
    0x9c845f8bbdf9283b,
    0x1ff897ffde05980f,
    0xef2f118b5a0a6d1f,
    0x6d367ecf27cb09b7,
    0x4f463f669e5fea2d,
    0x7527bac7ebe5f17b,
    0x3d0739f78a5292ea,
    0x6bfb5fb11f8d5d08,
    0x56033046fc7b6bab,
    0xf0cfbc209af4361d,
];

const INV_LN2: f64 = core::f64::consts::LOG2_E;
const TWO_OVER_PI_F64: f64 = core::f64::consts::FRAC_2_PI;
const CODY_WAITE_LIMIT: f64 = 1.0e6;
const QUARTER_PI: f64 = core::f64::consts::FRAC_PI_4;
const TWO_POW_54: f64 = 18014398509481984.0;
const TWO_POW_53: f64 = 9007199254740992.0;

const fn factorial(n: usize) -> f64 {
    let mut f = 1.0;
    let mut i = 2;
    while i <= n {
        f *= i as f64;
        i += 1;
    }
    f
}

const fn exp_tail() -> [f64; 14] {
    let mut c = [0.0; 14];
    let mut i = 0;
    while i < 14 {
        c[i] = 1.0 / factorial(i + 2);
        i += 1;
    }
    c
}

const fn sin_tail() -> [f64; 9] {
    let mut c = [0.0; 9];
    let mut i = 0;
    while i < 9 {
        let sign = if i % 2 == 0 { -1.0 } else { 1.0 };
        c[i] = sign / factorial(2 * i + 3);
        i += 1;
    }
    c
}

const fn cos_tail() -> [f64; 10] {
    let mut c = [0.0; 10];
    let mut i = 0;
    while i < 10 {
        let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
        c[i] = sign / factorial(2 * i + 4);
        i += 1;
    }
    c
}

const fn ln_tail() -> [f64; 12] {
    let mut c = [0.0; 12];
    let mut i = 0;
    while i < 12 {
        c[i] = 1.0 / (7 + 2 * i) as f64;
        i += 1;
    }
    c
}

const fn atan_tail() -> [f64; 9] {
    let mut c = [0.0; 9];
    let mut i = 0;
    while i < 9 {
        let sign = if i % 2 == 0 { -1.0 } else { 1.0 };
        c[i] = sign / (3 + 2 * i) as f64;
        i += 1;
    }
    c
}

const EXP_TAIL: [f64; 14] = exp_tail();
const SIN_TAIL: [f64; 9] = sin_tail();
const COS_TAIL: [f64; 10] = cos_tail();
const LN_TAIL: [f64; 12] = ln_tail();
const ATAN_TAIL: [f64; 9] = atan_tail();

#[derive(Clone, Copy)]
struct Dd {
    hi: f64,
    lo: f64,
}

impl Dd {
    const ONE: Dd = Dd { hi: 1.0, lo: 0.0 };
    const ZERO: Dd = Dd { hi: 0.0, lo: 0.0 };

    fn of(hi: f64) -> Dd {
        Dd { hi, lo: 0.0 }
    }

    fn neg(self) -> Dd {
        Dd {
            hi: -self.hi,
            lo: -self.lo,
        }
    }
}

fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    (s, b - (s - a))
}

fn split(a: f64) -> (f64, f64) {
    let t = 134217729.0 * a;
    let hi = t - (t - a);
    (hi, a - hi)
}

fn two_prod(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    let (ah, al) = split(a);
    let (bh, bl) = split(b);
    (p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
}

fn dd_add(a: Dd, b: Dd) -> Dd {
    let (s1, s2) = two_sum(a.hi, b.hi);
    let (t1, t2) = two_sum(a.lo, b.lo);
    let (s1, s2) = fast_two_sum(s1, s2 + t1);
    let (hi, lo) = fast_two_sum(s1, s2 + t2);
    Dd { hi, lo }
}

fn dd_sub(a: Dd, b: Dd) -> Dd {
    dd_add(a, b.neg())
}

fn dd_add_f(a: Dd, b: f64) -> Dd {
    let (s1, s2) = two_sum(a.hi, b);
    let (hi, lo) = fast_two_sum(s1, s2 + a.lo);
    Dd { hi, lo }
}

fn dd_mul(a: Dd, b: Dd) -> Dd {
    let (p1, p2) = two_prod(a.hi, b.hi);
    let (hi, lo) = fast_two_sum(p1, p2 + (a.hi * b.lo + a.lo * b.hi));
    Dd { hi, lo }
}

fn dd_mul_f(a: Dd, b: f64) -> Dd {
    let (p1, p2) = two_prod(a.hi, b);
    let (hi, lo) = fast_two_sum(p1, p2 + a.lo * b);
    Dd { hi, lo }
}

fn dd_div(a: Dd, b: Dd) -> Dd {
    let q1 = a.hi / b.hi;
    let r = dd_sub(a, dd_mul_f(b, q1));
    let q2 = r.hi / b.hi;
    let r = dd_sub(r, dd_mul_f(b, q2));
    let q3 = r.hi / b.hi;
    let (q1, q2) = fast_two_sum(q1, q2);
    dd_add_f(Dd { hi: q1, lo: q2 }, q3)
}

fn pow2i(n: i32) -> f64 {
    f64::from_bits(((n + 1023) as u64) << 52)
}

fn scalbn(mut y: f64, mut n: i32) -> f64 {
    if n > 1023 {
        y *= pow2i(1023);
        n -= 1023;
        if n > 1023 {
            y *= pow2i(1023);
            n -= 1023;
            if n > 1023 {
                n = 1023;
            }
        }
    } else if n < -1022 {
        y *= pow2i(-1022) * TWO_POW_53;
        n += 1022 - 53;
        if n < -1022 {
            y *= pow2i(-1022) * TWO_POW_53;
            n += 1022 - 53;
            if n < -1022 {
                n = -1022;
            }
        }
    }
    y * pow2i(n)
}

fn exp_parts(x: f64, xl: f64) -> f64 {
    if x > 710.0 {
        return f64::INFINITY;
    }
    if x < -746.0 {
        return 0.0;
    }
    let k = (x * INV_LN2 + 0.5).floor();
    let r1 = x - k * LN2_HI;
    let (r, e) = two_sum(r1, -(k * LN2_MID));
    let small = (e - k * LN2_LO) + xl;
    let r2 = r * r;
    let mut q = 0.0;
    for c in EXP_TAIL.iter().rev() {
        q = q * r + c;
    }
    let g = r + r2 * q;
    let (eh, el) = two_sum(1.0, g);
    scalbn(eh + (el + eh * small), k as i32)
}

pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    exp_parts(x, 0.0)
}

fn ln_dd(x: f64) -> Dd {
    let mut bits = x.to_bits();
    let mut k = 0i32;
    if bits >> 52 == 0 {
        bits = (x * TWO_POW_54).to_bits();
        k = -54;
    }
    k += ((bits >> 52) & 0x7ff) as i32 - 1023;
    let mut m = f64::from_bits((bits & MANT_MASK) | ONE_EXP);
    if m > core::f64::consts::SQRT_2 {
        m *= 0.5;
        k += 1;
    }
    let f = m - 1.0;
    let (p, pe) = two_sum(m, 1.0);
    let s = dd_div(Dd::of(f), Dd { hi: p, lo: pe });
    let z = dd_mul(s, s);
    let zh = z.hi;
    let mut t = 0.0;
    for c in LN_TAIL.iter().rev() {
        t = t * zh + c;
    }
    let inner = dd_add(Dd::ONE, dd_mul(z, THIRD_DD));
    let inner = dd_add(inner, dd_mul(dd_mul(z, z), FIFTH_DD));
    let inner = dd_add_f(inner, zh * zh * zh * t);
    let lnm = dd_mul(dd_mul_f(s, 2.0), inner);
    if k == 0 {
        lnm
    } else {
        dd_add(dd_mul_f(LN2_DD, k as f64), lnm)
    }
}

pub fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x == f64::INFINITY {
        return x;
    }
    ln_dd(x).hi
}

fn is_integer(y: f64) -> bool {
    y.floor() == y
}

fn is_odd_integer(y: f64) -> bool {
    y.abs() < TWO_POW_53 && is_integer(y) && !is_integer(y * 0.5)
}

fn pow_signed(v: f64, negative: bool) -> f64 {
    if negative { -v } else { v }
}

pub fn pow(x: f64, y: f64) -> f64 {
    if y == 0.0 || x == 1.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let y_integer = is_integer(y);
    let odd = is_odd_integer(y);
    if x == 0.0 {
        let negative_zero = x.is_sign_negative() && odd;
        return if y < 0.0 {
            pow_signed(f64::INFINITY, negative_zero)
        } else {
            pow_signed(0.0, negative_zero)
        };
    }
    if y.is_infinite() {
        let a = x.abs();
        return if a == 1.0 {
            1.0
        } else if (a > 1.0) == (y > 0.0) {
            f64::INFINITY
        } else {
            0.0
        };
    }
    if x.is_infinite() {
        let negative = x < 0.0 && odd;
        return if y > 0.0 {
            pow_signed(f64::INFINITY, negative)
        } else {
            pow_signed(0.0, negative)
        };
    }
    if x < 0.0 && !y_integer {
        return f64::NAN;
    }
    let negative = x < 0.0 && odd;
    let a = x.abs();
    if y == 1.0 {
        return x;
    }
    if a.to_bits() & MANT_MASK == 0 && a.to_bits() >> 52 != 0 && y.abs() < 4096.0 && y_integer {
        let e = ((a.to_bits() >> 52) as i32 - 1023) as f64 * y;
        let v = if e > 1100.0 {
            f64::INFINITY
        } else if e < -1100.0 {
            0.0
        } else {
            scalbn(1.0, e as i32)
        };
        return pow_signed(v, negative);
    }
    let l = ln_dd(a);
    let estimate = y * l.hi;
    let v = if estimate > 1000.0 {
        f64::INFINITY
    } else if estimate < -1000.0 {
        0.0
    } else {
        let t = dd_mul_f(l, y);
        exp_parts(t.hi, t.lo)
    };
    pow_signed(v, negative)
}

fn sin_kernel(r: f64, rl: f64) -> f64 {
    let z = r * r;
    let mut p = 0.0;
    for c in SIN_TAIL.iter().rev() {
        p = p * z + c;
    }
    let tail = r * z * p + rl * (1.0 - 0.5 * z);
    r + tail
}

fn cos_kernel(r: f64, rl: f64) -> f64 {
    let z = r * r;
    let mut q = 0.0;
    for c in COS_TAIL.iter().rev() {
        q = q * z + c;
    }
    let hz = 0.5 * z;
    let w = 1.0 - hz;
    let err = (1.0 - w) - hz;
    w + (z * z * q - r * rl + err)
}

fn reduce_small(ax: f64) -> (u32, f64, f64) {
    let k = (ax * TWO_OVER_PI_F64 + 0.5).floor();
    let r1 = ax - k * PIO2_P1;
    let (s, e) = two_sum(r1, -(k * PIO2_P2));
    let rl = (e - k * PIO2_P3) - k * PIO2_P4;
    let (r, rl) = fast_two_sum(s, rl);
    ((k as u64 & 3) as u32, r, rl)
}

fn two_over_pi_bit(j: i32) -> u64 {
    if !(1..=1280).contains(&j) {
        return 0;
    }
    let idx = (j - 1) as usize;
    (TWO_OVER_PI[idx / 64] >> (63 - idx % 64)) & 1
}

fn reduce_large(ax: f64) -> (u32, f64, f64) {
    let bits = ax.to_bits();
    let m = (bits & MANT_MASK) | (1 << 52);
    let e = ((bits >> 52) & 0x7ff) as i32 - 1075;
    let mut v = [0u64; 4];
    for p in 0..194i32 {
        if two_over_pi_bit(e + 192 - p) == 1 {
            v[(p / 64) as usize] |= 1 << (p % 64);
        }
    }
    let mut prod = [0u64; 4];
    let mut carry = 0u128;
    for i in 0..4 {
        let t = (m as u128) * (v[i] as u128) + carry;
        prod[i] = t as u64;
        carry = t >> 64;
    }
    let mut n = (prod[3] & 3) as u32;
    let mut f = [prod[0], prod[1], prod[2]];
    let mut negative = false;
    if f[2] >> 63 == 1 {
        negative = true;
        n = (n + 1) & 3;
        let mut carry = true;
        for limb in f.iter_mut() {
            let (s, c) = (!*limb).overflowing_add(carry as u64);
            *limb = s;
            carry = c;
        }
    }
    let lz = if f[2] != 0 {
        f[2].leading_zeros()
    } else if f[1] != 0 {
        64 + f[1].leading_zeros()
    } else if f[0] != 0 {
        128 + f[0].leading_zeros()
    } else {
        return (n, 0.0, 0.0);
    };
    let words = (lz / 64) as usize;
    let bitshift = lz % 64;
    let mut out = [0u64; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut w = 0u64;
        if i >= words {
            w |= f[i - words] << bitshift;
            if bitshift > 0 && i > words {
                w |= f[i - words - 1] >> (64 - bitshift);
            }
        }
        *slot = w;
    }
    let top = out[2] >> 11;
    let next = ((out[2] & 0x7ff) << 42) | (out[1] >> 22);
    let scale = pow2i(-(53 + lz as i32));
    let (hi, lo) = fast_two_sum(top as f64 * scale, next as f64 * pow2i(-(106 + lz as i32)));
    let r = dd_mul(Dd { hi, lo }, HALF_PI_DD);
    if negative {
        (n, -r.hi, -r.lo)
    } else {
        (n, r.hi, r.lo)
    }
}

fn reduce(ax: f64) -> (u32, f64, f64) {
    if ax <= QUARTER_PI {
        (0, ax, 0.0)
    } else if ax < CODY_WAITE_LIMIT {
        reduce_small(ax)
    } else {
        reduce_large(ax)
    }
}

pub fn sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x == 0.0 {
        return x;
    }
    let (q, r, rl) = reduce(x.abs());
    let v = match q {
        0 => sin_kernel(r, rl),
        1 => cos_kernel(r, rl),
        2 => -sin_kernel(r, rl),
        _ => -cos_kernel(r, rl),
    };
    if x < 0.0 { -v } else { v }
}

pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (q, r, rl) = reduce(x.abs());
    match q {
        0 => cos_kernel(r, rl),
        1 => -sin_kernel(r, rl),
        2 => -cos_kernel(r, rl),
        _ => sin_kernel(r, rl),
    }
}

fn atan_unit(t: f64) -> Dd {
    if t == 0.0 {
        return Dd::ZERO;
    }
    let k = (t * 8.0 + 0.5).floor() as usize;
    let (base, u) = if k == 0 {
        (Dd::ZERO, Dd::of(t))
    } else {
        let c = k as f64 / 8.0;
        let (p, pe) = two_prod(t, c);
        let den = dd_add_f(Dd { hi: p, lo: pe }, 1.0);
        (ATAN_TABLE[k - 1], dd_div(Dd::of(t - c), den))
    };
    let s = u.hi;
    let z = s * s;
    let mut p = 0.0;
    for c in ATAN_TAIL.iter().rev() {
        p = p * z + c;
    }
    let ser = s * z * p + u.lo;
    dd_add_f(dd_add_f(base, s), ser)
}

pub fn atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let negative = y.is_sign_negative();
    let signed = |v: Dd| if negative { -v.hi } else { v.hi };
    if y == 0.0 {
        return if x.is_sign_negative() {
            signed(PI_DD)
        } else {
            signed(Dd::ZERO)
        };
    }
    if x == 0.0 {
        return signed(HALF_PI_DD);
    }
    if x.is_infinite() {
        return if y.is_infinite() {
            signed(if x > 0.0 {
                QUARTER_PI_DD
            } else {
                THREE_QUARTER_PI_DD
            })
        } else if x > 0.0 {
            signed(Dd::ZERO)
        } else {
            signed(PI_DD)
        };
    }
    if y.is_infinite() {
        return signed(HALF_PI_DD);
    }
    let ay = y.abs();
    let ax = x.abs();
    let swap = ay > ax;
    let t = if swap { ax / ay } else { ay / ax };
    let mut a = atan_unit(t);
    if swap {
        a = dd_sub(HALF_PI_DD, a);
    }
    if x < 0.0 {
        a = dd_sub(PI_DD, a);
    }
    signed(a)
}

pub fn sqrt(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    x.sqrt()
}

pub fn sinf(x: f32) -> f32 {
    sin(x as f64) as f32
}

pub fn cosf(x: f32) -> f32 {
    cos(x as f64) as f32
}

pub fn atan2f(y: f32, x: f32) -> f32 {
    atan2(y as f64, x as f64) as f32
}

pub fn expf(x: f32) -> f32 {
    exp(x as f64) as f32
}

pub fn lnf(x: f32) -> f32 {
    ln(x as f64) as f32
}

pub fn powf(x: f32, y: f32) -> f32 {
    pow(x as f64, y as f64) as f32
}

pub fn sqrtf(x: f32) -> f32 {
    if x.is_nan() {
        return f32::NAN;
    }
    x.sqrt()
}
