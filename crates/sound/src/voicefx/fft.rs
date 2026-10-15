use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Complex {
    pub(crate) re: f32,
    pub(crate) im: f32,
}

impl Complex {
    pub(crate) const ZERO: Complex = Complex { re: 0.0, im: 0.0 };

    pub(crate) fn new(re: f32, im: f32) -> Complex {
        Complex { re, im }
    }

    pub(crate) fn polar(phase: f32) -> Complex {
        Complex {
            re: phase.cos(),
            im: phase.sin(),
        }
    }

    pub(crate) fn add(self, b: Complex) -> Complex {
        Complex::new(self.re + b.re, self.im + b.im)
    }

    pub(crate) fn sub(self, b: Complex) -> Complex {
        Complex::new(self.re - b.re, self.im - b.im)
    }

    pub(crate) fn scale(self, s: f32) -> Complex {
        Complex::new(self.re * s, self.im * s)
    }

    pub(crate) fn div(self, s: f32) -> Complex {
        Complex::new(self.re / s, self.im / s)
    }

    pub(crate) fn mul(self, b: Complex) -> Complex {
        Complex::new(
            self.re * b.re - self.im * b.im,
            self.re * b.im + self.im * b.re,
        )
    }

    pub(crate) fn mul_conj(self, b: Complex) -> Complex {
        Complex::new(
            b.re * self.re + b.im * self.im,
            b.re * self.im - b.im * self.re,
        )
    }

    pub(crate) fn norm(self) -> f32 {
        self.re * self.re + self.im * self.im
    }
}

struct Pow2 {
    twiddles: Vec<Complex>,
    size: usize,
    work_r: Vec<f32>,
    work_i: Vec<f32>,
}

impl Pow2 {
    fn new(size: usize) -> Pow2 {
        let twiddles = (0..size * 3 / 4)
            .map(|i| Complex::polar((-2.0 * PI * i as f64 / size as f64) as f32))
            .collect();
        Pow2 {
            twiddles,
            size,
            work_r: vec![0.0; size],
            work_i: vec![0.0; size],
        }
    }

    fn run<const INVERSE: bool>(
        &mut self,
        in_r: &[f32],
        in_i: &[f32],
        out_r: &mut [f32],
        out_i: &mut [f32],
    ) {
        let size = self.size;
        if size <= 1 {
            out_r[0] = in_r[0];
            out_i[0] = in_i[0];
            return;
        }
        let mut work_r = std::mem::take(&mut self.work_r);
        let mut work_i = std::mem::take(&mut self.work_i);
        self.pass::<INVERSE>(size, 1, in_r, in_i, out_r, out_i, &mut work_r, &mut work_i);
        self.work_r = work_r;
        self.work_i = work_i;
    }

    #[allow(clippy::too_many_arguments)]
    fn pass<const INVERSE: bool>(
        &self,
        size: usize,
        stride: usize,
        in_r: &[f32],
        in_i: &[f32],
        out_r: &mut [f32],
        out_i: &mut [f32],
        work_r: &mut [f32],
        work_i: &mut [f32],
    ) {
        if size / 4 > 1 {
            self.pass::<INVERSE>(
                size / 4,
                stride * 4,
                in_r,
                in_i,
                work_r,
                work_i,
                out_r,
                out_i,
            );
            self.combine4::<INVERSE>(size, stride, work_r, work_i, out_r, out_i);
        } else if size == 4 {
            self.combine4::<INVERSE>(4, stride, in_r, in_i, out_r, out_i);
        } else {
            for s in 0..stride {
                let (ar, ai) = (in_r[s], in_i[s]);
                let (br, bi) = (in_r[s + stride], in_i[s + stride]);
                out_r[s] = ar + br;
                out_i[s] = ai + bi;
                out_r[s + stride] = ar - br;
                out_i[s + stride] = ai - bi;
            }
        }
    }

    fn twist<const INVERSE: bool>(a: Complex, b: Complex) -> Complex {
        if INVERSE {
            Complex::new(a.re * b.re + a.im * b.im, a.im * b.re - a.re * b.im)
        } else {
            Complex::new(a.re * b.re - a.im * b.im, a.im * b.re + a.re * b.im)
        }
    }

    fn combine4<const INVERSE: bool>(
        &self,
        size: usize,
        stride: usize,
        in_r: &[f32],
        in_i: &[f32],
        out_r: &mut [f32],
        out_i: &mut [f32],
    ) {
        let step = self.size / size;
        let quarter = size / 4;
        for i in 0..quarter {
            let tb = self.twiddles[i * step];
            let tc = self.twiddles[i * 2 * step];
            let td = self.twiddles[i * 3 * step];
            let ia = 4 * i * stride;
            let ib = (4 * i + 1) * stride;
            let ic = (4 * i + 2) * stride;
            let id = (4 * i + 3) * stride;
            let oa = i * stride;
            let ob = (i + quarter) * stride;
            let oc = (i + quarter * 2) * stride;
            let od = (i + quarter * 3) * stride;
            for s in 0..stride {
                let a = Complex::new(in_r[ia + s], in_i[ia + s]);
                let b = Self::twist::<INVERSE>(Complex::new(in_r[ib + s], in_i[ib + s]), tb);
                let c = Self::twist::<INVERSE>(Complex::new(in_r[ic + s], in_i[ic + s]), tc);
                let d = Self::twist::<INVERSE>(Complex::new(in_r[id + s], in_i[id + s]), td);
                let ac0 = a.add(c);
                let ac1 = a.sub(c);
                let bd0 = b.add(d);
                let bd1 = if INVERSE { b.sub(d) } else { d.sub(b) };
                let bd1i = Complex::new(-bd1.im, bd1.re);
                out_r[oa + s] = ac0.re + bd0.re;
                out_i[oa + s] = ac0.im + bd0.im;
                out_r[ob + s] = ac1.re + bd1i.re;
                out_i[ob + s] = ac1.im + bd1i.im;
                out_r[oc + s] = ac0.re - bd0.re;
                out_i[oc + s] = ac0.im - bd0.im;
                out_r[od + s] = ac1.re - bd1i.re;
                out_i[od + s] = ac1.im - bd1i.im;
            }
        }
    }
}

struct Split {
    inner_size: usize,
    outer_size: usize,
    inner: Pow2,
    tmp_r: Vec<f32>,
    tmp_i: Vec<f32>,
    twiddles_r: Vec<f32>,
    twiddles_i: Vec<f32>,
}

impl Split {
    fn fast_size_above(size: usize) -> usize {
        let mut pow2 = 1;
        while pow2 < 16 && pow2 < size {
            pow2 *= 2;
        }
        while pow2 * 8 < size {
            pow2 *= 2;
        }
        let mut multiple = size.div_ceil(pow2);
        if multiple == 7 {
            multiple += 1;
        }
        multiple * pow2
    }

    fn new(size: usize) -> Split {
        let mut inner_size = 1;
        let mut outer_size = size;
        while outer_size & 1 == 0 && (outer_size > 1 || inner_size < 32) {
            inner_size *= 2;
            outer_size /= 2;
        }
        let count = inner_size * outer_size.saturating_sub(1);
        let mut twiddles_r = vec![0.0; count];
        let mut twiddles_i = vec![0.0; count];
        for i in 0..inner_size {
            for s in 1..outer_size {
                let phase = (-2.0 * PI * i as f64 / inner_size as f64 * s as f64
                    / outer_size as f64) as f32;
                let t = Complex::polar(phase);
                twiddles_r[i + (s - 1) * inner_size] = t.re;
                twiddles_i[i + (s - 1) * inner_size] = t.im;
            }
        }
        Split {
            inner_size,
            outer_size,
            inner: Pow2::new(inner_size),
            tmp_r: vec![0.0; size],
            tmp_i: vec![0.0; size],
            twiddles_r,
            twiddles_i,
        }
    }

    fn run<const INVERSE: bool>(
        &mut self,
        in_r: &[f32],
        in_i: &[f32],
        out_r: &mut [f32],
        out_i: &mut [f32],
    ) {
        let n = self.inner_size;
        let outer = self.outer_size;
        if outer <= 1 {
            self.inner.run::<INVERSE>(in_r, in_i, out_r, out_i);
            return;
        }
        for bi in 0..n {
            for ai in 0..outer {
                self.tmp_r[bi + ai * n] = in_r[bi * outer + ai];
                self.tmp_i[bi + ai * n] = in_i[bi * outer + ai];
            }
        }
        for s in 0..outer {
            let at = s * n;
            self.inner.run::<INVERSE>(
                &self.tmp_r[at..at + n],
                &self.tmp_i[at..at + n],
                &mut out_r[at..at + n],
                &mut out_i[at..at + n],
            );
        }
        for k in 0..n * (outer - 1) {
            let (br, bi) = (out_r[n + k], out_i[n + k]);
            let (cr, ci) = (self.twiddles_r[k], self.twiddles_i[k]);
            if INVERSE {
                out_r[n + k] = cr * br + ci * bi;
                out_i[n + k] = cr * bi - ci * br;
            } else {
                out_r[n + k] = br * cr - bi * ci;
                out_i[n + k] = br * ci + bi * cr;
            }
        }
        match outer {
            3 => self.final3::<INVERSE>(out_r, out_i),
            5 => self.final5::<INVERSE>(out_r, out_i),
            _ => self.final_n::<INVERSE>(out_r, out_i),
        }
    }

    fn final3<const INVERSE: bool>(&self, r: &mut [f32], i: &mut [f32]) {
        let n = self.inner_size;
        let tw1r = -0.5f32;
        let tw1i = (-(0.75f64.sqrt()) * if INVERSE { -1.0 } else { 1.0 }) as f32;
        for k in 0..n {
            let (ar, ai, br, bi, cr, ci) =
                (r[k], i[k], r[k + n], i[k + n], r[k + 2 * n], i[k + 2 * n]);
            r[k] = ar + br + cr;
            i[k] = ai + bi + ci;
            r[k + n] = ar + br * tw1r - bi * tw1i + cr * tw1r + ci * tw1i;
            i[k + n] = ai + bi * tw1r + br * tw1i - cr * tw1i + ci * tw1r;
            r[k + 2 * n] = ar + br * tw1r + bi * tw1i + cr * tw1r - ci * tw1i;
            i[k + 2 * n] = ai + bi * tw1r - br * tw1i + cr * tw1i + ci * tw1r;
        }
    }

    fn final5<const INVERSE: bool>(&self, r: &mut [f32], i: &mut [f32]) {
        let n = self.inner_size;
        let sign = if INVERSE { -1.0f64 } else { 1.0 };
        let tw1r = 0.309_016_994_374_947_45f64 as f32;
        let tw1i = (-0.951_056_516_295_153_5f64 * sign) as f32;
        let tw2r = -0.809_016_994_374_947_3f64 as f32;
        let tw2i = (-0.587_785_252_292_473_2f64 * sign) as f32;
        for k in 0..n {
            let (ar, ai) = (r[k], i[k]);
            let (br, bi) = (r[k + n], i[k + n]);
            let (cr, ci) = (r[k + 2 * n], i[k + 2 * n]);
            let (dr, di) = (r[k + 3 * n], i[k + 3 * n]);
            let (er, ei) = (r[k + 4 * n], i[k + 4 * n]);
            let (be0r, be0i) = (br + er, bi + ei);
            let (be1r, be1i) = (ei - bi, br - er);
            let (cd0r, cd0i) = (cr + dr, ci + di);
            let (cd1r, cd1i) = (di - ci, cr - dr);
            let (b01r, b01i) = (be0r * tw1r + cd0r * tw2r, be0i * tw1r + cd0i * tw2r);
            let (b02r, b02i) = (be0r * tw2r + cd0r * tw1r, be0i * tw2r + cd0i * tw1r);
            let (b11r, b11i) = (be1r * tw1i + cd1r * tw2i, be1i * tw1i + cd1i * tw2i);
            let (b12r, b12i) = (be1r * tw2i - cd1r * tw1i, be1i * tw2i - cd1i * tw1i);
            r[k] = ar + be0r + cd0r;
            i[k] = ai + be0i + cd0i;
            r[k + n] = ar + b01r + b11r;
            i[k + n] = ai + b01i + b11i;
            r[k + 2 * n] = ar + b02r + b12r;
            i[k + 2 * n] = ai + b02i + b12i;
            r[k + 3 * n] = ar + b02r - b12r;
            i[k + 3 * n] = ai + b02i - b12i;
            r[k + 4 * n] = ar + b01r - b11r;
            i[k + 4 * n] = ai + b01i - b11i;
        }
    }

    fn final_n<const INVERSE: bool>(&self, r: &mut [f32], i: &mut [f32]) {
        let n = self.inner_size;
        let outer = self.outer_size;
        let twists: Vec<Complex> = (0..outer)
            .map(|s| Complex::polar((-2.0 * PI * s as f64 / outer as f64) as f32))
            .collect();
        let mut tr = vec![0.0f32; outer];
        let mut ti = vec![0.0f32; outer];
        for k in 0..n {
            let (mut sum_r, mut sum_i) = (0.0f32, 0.0f32);
            for j in 0..outer {
                tr[j] = r[k + j * n];
                ti[j] = i[k + j * n];
                sum_r += tr[j];
                sum_i += ti[j];
            }
            r[k] = sum_r;
            i[k] = sum_i;
            for f in 1..outer {
                let (mut sum_r, mut sum_i) = (tr[0], ti[0]);
                for j in 1..outer {
                    let t = twists[(j * f) % outer];
                    let t = if INVERSE {
                        Complex::new(t.re, -t.im)
                    } else {
                        t
                    };
                    sum_r += tr[j] * t.re - ti[j] * t.im;
                    sum_i += ti[j] * t.re + tr[j] * t.im;
                }
                r[k + f * n] = sum_r;
                i[k + f * n] = sum_i;
            }
        }
    }
}

pub(crate) struct ModifiedRealFft {
    complex: Split,
    time_r: Vec<f32>,
    time_i: Vec<f32>,
    freq_r: Vec<f32>,
    freq_i: Vec<f32>,
    twiddles: Vec<Complex>,
    half_bin: Vec<Complex>,
}

impl ModifiedRealFft {
    pub(crate) fn fast_size_above(size: usize) -> usize {
        Split::fast_size_above(size.div_ceil(2)) * 2
    }

    pub(crate) fn new(size: usize) -> ModifiedRealFft {
        let half = size / 2;
        let twiddles = (0..half / 2 + 1)
            .map(|i| {
                Complex::polar(((i as f64 + 0.5) * (-2.0 * PI / size as f64) - PI / 2.0) as f32)
            })
            .collect();
        let half_bin = (0..half)
            .map(|i| Complex::polar((-2.0 * PI * i as f64 / size as f64) as f32))
            .collect();
        ModifiedRealFft {
            complex: Split::new(half),
            time_r: vec![0.0; half],
            time_i: vec![0.0; half],
            freq_r: vec![0.0; half],
            freq_i: vec![0.0; half],
            twiddles,
            half_bin,
        }
    }

    pub(crate) fn fft(&mut self, time: &[f32], freq: &mut [Complex]) {
        let half = self.time_r.len();
        for i in 0..half {
            let (tr, ti) = (time[2 * i], time[2 * i + 1]);
            let t = self.half_bin[i];
            self.time_r[i] = tr * t.re - ti * t.im;
            self.time_i[i] = ti * t.re + tr * t.im;
        }
        self.complex.run::<false>(
            &self.time_r,
            &self.time_i,
            &mut self.freq_r,
            &mut self.freq_i,
        );
        let (fr, fi) = (&self.freq_r, &self.freq_i);
        for i in 0..half / 2 + 1 {
            let c = half - 1 - i;
            let t = self.twiddles[i];
            let odd_r = (fr[i] + fr[c]) * 0.5;
            let odd_i = (fi[i] - fi[c]) * 0.5;
            let even_r = (fr[i] - fr[c]) * 0.5;
            let even_i = (fi[i] + fi[c]) * 0.5;
            let rot_r = even_r * t.re - even_i * t.im;
            let rot_i = even_i * t.re + even_r * t.im;
            freq[i] = Complex::new(odd_r + rot_r, odd_i + rot_i);
            freq[c] = Complex::new(odd_r - rot_r, rot_i - odd_i);
        }
    }

    pub(crate) fn ifft(&mut self, freq: &[Complex], time: &mut [f32]) {
        let half = self.time_r.len();
        for i in 0..half / 2 + 1 {
            let c = half - 1 - i;
            let t = self.twiddles[i];
            let odd = Complex::new(freq[i].re + freq[c].re, freq[i].im - freq[c].im);
            let rot = Complex::new(freq[i].re - freq[c].re, freq[i].im + freq[c].im);
            let even = Complex::new(rot.re * t.re + rot.im * t.im, rot.im * t.re - rot.re * t.im);
            self.freq_r[i] = odd.re + even.re;
            self.freq_i[i] = odd.im + even.im;
            self.freq_r[c] = odd.re - even.re;
            self.freq_i[c] = even.im - odd.im;
        }
        self.complex.run::<true>(
            &self.freq_r,
            &self.freq_i,
            &mut self.time_r,
            &mut self.time_i,
        );
        for i in 0..half {
            let (tr, ti) = (self.time_r[i], self.time_i[i]);
            let t = self.half_bin[i];
            time[2 * i] = tr * t.re + ti * t.im;
            time[2 * i + 1] = ti * t.re - tr * t.im;
        }
    }
}
