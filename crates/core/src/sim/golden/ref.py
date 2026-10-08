import sys
from decimal import Decimal, getcontext
from fractions import Fraction
import struct

getcontext().prec = 400
D = Decimal


def pi():
    getcontext().prec += 10
    a, b, t, p = D(1), D(1) / D(2).sqrt(), D(1) / 4, D(1)
    for _ in range(12):
        an = (a + b) / 2
        b = (a * b).sqrt()
        t -= p * (a - an) ** 2
        a = an
        p *= 2
    r = (a + b) ** 2 / (4 * t)
    getcontext().prec -= 10
    return +r


PI = pi()
HALF_PI = PI / 2
TWO_PI = PI * 2


def sin_cos(x):
    x = D(x)
    k = (x / HALF_PI).to_integral_value()
    r = x - k * HALF_PI
    q = int(k) % 4
    s, c = taylor_sin_cos(r)
    if q == 0:
        return s, c
    if q == 1:
        return c, -s
    if q == 2:
        return -s, -c
    return -c, s


def taylor_sin_cos(r):
    eps = D(10) ** -(getcontext().prec - 5)
    s = D(0)
    c = D(0)
    term = D(1)
    n = 0
    while abs(term) > eps * D("1e-5") or n < 4:
        if n % 4 == 0:
            c += term
        elif n % 4 == 1:
            s += term
        elif n % 4 == 2:
            c -= term
        else:
            s -= term
        n += 1
        term = term * r / n
    return s, c


def atan(x):
    x = D(x)
    neg = x < 0
    x = abs(x)
    inv = x > 1
    if inv:
        x = 1 / x
    n = 0
    while x > D("0.05"):
        x = x / (1 + (1 + x * x).sqrt())
        n += 1
    s = D(0)
    term = x
    k = 0
    xx = x * x
    eps = D(10) ** -(getcontext().prec - 5)
    while abs(term) > eps * D("1e-5"):
        s += term / (2 * k + 1) * (1 if k % 2 == 0 else -1)
        term *= xx
        k += 1
    s *= 2 ** n
    if inv:
        s = HALF_PI - s
    return -s if neg else s


def atan2(y, x):
    y = D(y)
    x = D(x)
    if x == 0:
        return HALF_PI if y > 0 else -HALF_PI
    a = atan(y / x)
    if x < 0:
        a += PI if y >= 0 else -PI
    return a


def exact(f):
    return D(Fraction(f).numerator) / D(Fraction(f).denominator)


def f64_from_bits(b):
    return struct.unpack("<d", struct.pack("<Q", b))[0]


def bits(f):
    return struct.unpack("<Q", struct.pack("<d", f))[0]


def to_dd(v):
    hi = float(v)
    lo = float(v - exact(hi))
    return hi, lo


def ref_value(fn, args):
    if fn == "exp":
        return exact(args[0]).exp()
    if fn == "ln":
        return exact(args[0]).ln()
    if fn == "pow":
        x = exact(args[0])
        v = (exact(args[1]) * abs(x).ln()).exp()
        if x < 0 and args[1] % 2 == 1:
            v = -v
        return v
    if fn == "sin":
        return sin_cos(exact(args[0]))[0]
    if fn == "cos":
        return sin_cos(exact(args[0]))[1]
    if fn == "atan2":
        return atan2(exact(args[0]), exact(args[1]))
    raise ValueError(fn)


def hexf(f):
    return "%016x" % bits(f)


def rust_f64(v):
    hi, lo = to_dd(v)
    return "f64::from_bits(0x%016x)" % bits(hi), "f64::from_bits(0x%016x)" % bits(lo)


def dd_const(name, v):
    h, l = to_dd(v)
    return "const %s: Dd = Dd {\n    hi: f64::from_bits(0x%016x),\n    lo: f64::from_bits(0x%016x),\n};" % (name, bits(h), bits(l))


def consts():
    out = []
    parts = []
    rem = HALF_PI
    for _ in range(3):
        part = f64_from_bits(bits(float(rem)) & ~((1 << 20) - 1))
        parts.append(part)
        rem -= exact(part)
    parts.append(float(rem))
    for name, part in zip(["P1", "P2", "P3", "P4"], parts):
        out.append("const PIO2_%s: f64 = f64::from_bits(0x%016x);" % (name, bits(part)))
    out.append(dd_const("PI_DD", PI))
    out.append(dd_const("HALF_PI_DD", HALF_PI))
    out.append(dd_const("QUARTER_PI_DD", PI / 4))
    out.append(dd_const("THREE_QUARTER_PI_DD", PI * 3 / 4))
    ln2 = D(2).ln()
    hi = f64_from_bits(bits(float(ln2)) & ~((1 << 22) - 1))
    mid = f64_from_bits(bits(float(ln2 - exact(hi))) & ~((1 << 22) - 1))
    lo = float(ln2 - exact(hi) - exact(mid))
    out.append("const LN2_HI: f64 = f64::from_bits(0x%016x);" % bits(hi))
    out.append("const LN2_MID: f64 = f64::from_bits(0x%016x);" % bits(mid))
    out.append("const LN2_LO: f64 = f64::from_bits(0x%016x);" % bits(lo))
    out.append(dd_const("LN2_DD", ln2))
    out.append(dd_const("THIRD_DD", D(1) / 3))
    out.append(dd_const("FIFTH_DD", D(1) / 5))
    rows = []
    for k in range(1, 9):
        h, l = to_dd(atan(D(k) / 8))
        rows.append("    Dd {\n        hi: f64::from_bits(0x%016x),\n        lo: f64::from_bits(0x%016x),\n    }," % (bits(h), bits(l)))
    out.append("const ATAN_TABLE: [Dd; 8] = [")
    out.extend(rows)
    out.append("];")
    twoopi = 2 / PI
    words = []
    frac = twoopi
    for _ in range(20):
        frac *= D(2) ** 64
        w = int(frac)
        words.append(w)
        frac -= w
    out.append("const TWO_OVER_PI: [u64; 20] = [")
    for w in words:
        out.append("    0x%016x," % w)
    out.append("];")
    return "\n".join(out)


class Lcg:
    def __init__(self, seed):
        self.s = seed

    def next(self):
        self.s = (self.s * 6364136223846793005 + 1442695040888963407) & (2**64 - 1)
        return self.s >> 11

    def unit(self):
        return self.next() / 2**53

    def uniform(self, lo, hi):
        return lo + (hi - lo) * self.unit()

    def log_uniform(self, lo, hi):
        import math
        return math.exp(self.uniform(math.log(lo), math.log(hi)))


def inputs():
    import math
    r = Lcg(20261005)
    rows = []
    for x in [0.0, 1.0, -1.0, 0.5, -0.5, 1e-10, -1e-10, 1e-300, 5e-324, 0.6931471805599453, -0.6931471805599453,
              2.0, 10.0, -10.0, 100.0, -100.0, 700.0, -700.0, 709.0, -708.0, -740.0, -744.0, 709.78, 2.0**-54, 1.0 + 2.0**-52]:
        rows.append(("exp", [x]))
    for _ in range(60):
        rows.append(("exp", [r.uniform(-700, 700)]))
    for _ in range(40):
        rows.append(("exp", [r.uniform(-1, 1)]))
    for k in range(-20, 21, 3):
        rows.append(("exp", [k * 0.6931471805599453 + 1e-9]))
    for x in [1.0, 2.0, 0.5, math.e, 10.0, 1e-300, 5e-324, 1.7e308, 1.0 + 2.0**-52, 1.0 - 2.0**-53, 1.0000001, 0.9999999,
              1.4142135623730951, 0.7071067811865476, 3.0, 1e10, 123456.789, 2.2250738585072014e-308]:
        rows.append(("ln", [x]))
    for _ in range(80):
        rows.append(("ln", [r.log_uniform(1e-300, 1e300)]))
    for _ in range(40):
        rows.append(("ln", [r.uniform(0.5, 2.0)]))
    for x, y in [(2.0, 10.0), (10.0, 2.0), (10.0, -3.0), (0.5, 0.5), (-2.0, 3.0), (-2.0, 4.0), (1.0000001, 1e9), (3.0, 0.3333333333333333),
                 (1e-5, 100.0), (7.0, 80.0), (2.0, -1074.0), (2.0, 1023.0), (10.0, 300.0), (10.0, -300.0), (1.5, 1.5), (100.0, 0.5),
                 (0.1, 3.0), (1.1, 100.0), (9.0, 0.5), (27.0, 1.0/3.0), (-8.0, 3.0), (5.0, 0.0), (1.0 + 2.0**-52, 1e15)]:
        rows.append(("pow", [x, y]))
    for _ in range(80):
        rows.append(("pow", [r.log_uniform(0.01, 100.0), r.uniform(-20, 20)]))
    for _ in range(20):
        rows.append(("pow", [r.log_uniform(1e-3, 1e3), r.uniform(-100, 100)]))
    specials = [0.0, 1e-10, 2.0**-30, 0.5, 0.7853981633974483, 0.7853981633974484, 1.0, 1.5707963267948966, 2.0, 3.141592653589793, 4.0,
                6.283185307179586, 1e3, 1e5, 999999.0, 1e6, 1e7, 1e10, 1e15, 1e22, 1e100, 1e300, 2.0**1000, 2.0**52 + 1, 2.0**70, 1.7976931348623157e308]
    for k in [1, 2, 3, 5, 7, 10, 20, 100, 1000, 100000, 600000, 700000]:
        specials.append(float(k * HALF_PI))
    for fn in ("sin", "cos"):
        for x in specials:
            rows.append((fn, [x]))
            rows.append((fn, [-x]))
        for _ in range(40):
            rows.append((fn, [r.uniform(-10, 10)]))
        for _ in range(30):
            rows.append((fn, [r.uniform(-1000, 1000)]))
        for _ in range(20):
            rows.append((fn, [r.log_uniform(1e6, 1e300)]))
        for _ in range(20):
            rows.append((fn, [r.uniform(-0.8, 0.8)]))
    mags = [1e-300, 1e-5, 0.1, 0.5, 1.0, 2.0, 10.0, 1e5, 1e300]
    for a in mags:
        for b in mags:
            for sy in (1, -1):
                for sx in (1, -1):
                    rows.append(("atan2", [sy * a, sx * b]))
    for _ in range(100):
        rows.append(("atan2", [r.uniform(-5, 5), r.uniform(-5, 5)]))
    for _ in range(40):
        rows.append(("atan2", [r.uniform(-1, 1), r.uniform(-1, 1) * 1e-3]))
    return rows


def to_f32(x):
    try:
        return struct.unpack("<f", struct.pack("<f", x))[0]
    except OverflowError:
        return None


def print_rows(rows, single):
    for fn, args in rows:
        if single:
            args = [to_f32(a) for a in args]
            if any(a is None for a in args):
                continue
            if fn == "ln" and args[0] <= 0.0:
                continue
            if fn == "atan2" and (args[0] == 0.0 or args[1] == 0.0):
                continue
        print(fn, *["%016x" % bits(a) for a in args])


if __name__ == "__main__":
    if sys.argv[1] in ("inputs", "inputs32"):
        print_rows(inputs(), sys.argv[1] == "inputs32")
        sys.exit(0)
    if sys.argv[1] == "consts":
        print(consts())
    elif sys.argv[1] == "refs":
        for line in sys.stdin:
            parts = line.split()
            if not parts:
                continue
            fn = parts[0]
            args = [f64_from_bits(int(p, 16)) for p in parts[1:]]
            v = ref_value(fn, args)
            hi, lo = to_dd(v)
            print(fn, *parts[1:], hexf(hi), hexf(lo))
