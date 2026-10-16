use super::spectrum::{self, Grid};

pub(crate) const N_FFT: usize = 1024;
const N_STD_THRESH_STATIONARY: f64 = 1.5;
const TOP_DB: f64 = 80.0;
const FREQ_MASK_SMOOTH_HZ: f64 = 500.0;
const TIME_MASK_SMOOTH_MS: f64 = 50.0;
pub(crate) const PROP_DECREASE: f32 = 1.0;
pub(crate) const PRINT_SECONDS: f32 = 1.0;

fn amp_to_db(mags: &[f32], frames: usize, bins: usize) -> Vec<f64> {
    let mut db: Vec<f64> = mags
        .iter()
        .map(|m| 20.0 * (f64::from(*m) + f64::EPSILON).log10())
        .collect();
    for k in 0..bins {
        let top = (0..frames)
            .map(|t| db[t * bins + k])
            .fold(f64::MIN, f64::max);
        for t in 0..frames {
            let v = &mut db[t * bins + k];
            *v = v.max(top - TOP_DB);
        }
    }
    db
}

pub(crate) fn threshold(print: &[f32], rate: u32) -> Vec<f64> {
    let grid = Grid::new(N_FFT, rate, print.len());
    let mags = spectrum::analyse(print, grid).magnitudes();
    let db = amp_to_db(&mags, grid.frames, grid.bins);
    let n = grid.frames as f64;
    (0..grid.bins)
        .map(|k| {
            let mean = (0..grid.frames).map(|t| db[t * grid.bins + k]).sum::<f64>() / n;
            let var = (0..grid.frames)
                .map(|t| (db[t * grid.bins + k] - mean).powi(2))
                .sum::<f64>()
                / n;
            mean + var.sqrt() * N_STD_THRESH_STATIONARY
        })
        .collect()
}

pub(crate) fn quietest(signal: &[f32], rate: u32) -> (usize, usize) {
    let length = ((PRINT_SECONDS * rate as f32) as usize).max(N_FFT);
    if signal.len() <= length * 2 {
        return (0, signal.len());
    }
    let hop = N_FFT / 4;
    let mut prefix = Vec::with_capacity(signal.len() + 1);
    prefix.push(0.0f64);
    let mut sum = 0.0f64;
    for s in signal {
        sum += f64::from(*s) * f64::from(*s);
        prefix.push(sum);
    }
    let mut best = (f64::MAX, 0usize);
    let mut from = 0;
    while from + length <= signal.len() {
        let e = prefix[from + length] - prefix[from];
        if e < best.0 {
            best = (e, from);
        }
        from += hop;
    }
    (best.1, best.1 + length)
}

fn triangle(n: usize) -> Vec<f64> {
    let up = (1..=n).map(|i| i as f64 / (n + 1) as f64);
    let down = (0..=n).map(|i| 1.0 - i as f64 / (n + 1) as f64);
    up.chain(down).collect()
}

fn smoothing(rate: u32, hop: usize) -> (Vec<f64>, Vec<f64>) {
    let n_grad_freq = (FREQ_MASK_SMOOTH_HZ / (f64::from(rate) / (N_FFT as f64 / 2.0))) as usize;
    let n_grad_time = (TIME_MASK_SMOOTH_MS / ((hop as f64 / f64::from(rate)) * 1000.0)) as usize;
    let freq = triangle(n_grad_freq.max(1));
    let time = triangle(n_grad_time.max(1));
    let sf: f64 = freq.iter().sum();
    let st: f64 = time.iter().sum();
    (
        freq.into_iter().map(|v| v / sf).collect(),
        time.into_iter().map(|v| v / st).collect(),
    )
}

fn convolve_same(data: &[f64], frames: usize, bins: usize, freq: &[f64], time: &[f64]) -> Vec<f64> {
    let hf = (freq.len() / 2) as isize;
    let ht = (time.len() / 2) as isize;
    let mut across = vec![0.0f64; data.len()];
    let mut reach = vec![0.0f64; bins];
    for (k, r) in reach.iter_mut().enumerate() {
        *r = freq
            .iter()
            .enumerate()
            .filter(|(j, _)| (0..bins as isize).contains(&(k as isize + *j as isize - hf)))
            .map(|(_, w)| w)
            .sum();
    }
    for t in 0..frames {
        for k in 0..bins {
            let mut sum = 0.0;
            for (j, w) in freq.iter().enumerate() {
                let kk = k as isize + j as isize - hf;
                if kk >= 0 && (kk as usize) < bins {
                    sum += w * data[t * bins + kk as usize];
                }
            }
            across[t * bins + k] = sum / reach[k];
        }
    }
    let mut out = vec![0.0f64; data.len()];
    for t in 0..frames {
        let mut weight = 0.0;
        for (j, w) in time.iter().enumerate() {
            let tt = t as isize + j as isize - ht;
            if tt < 0 || tt as usize >= frames {
                continue;
            }
            weight += w;
            let src = &across[tt as usize * bins..(tt as usize + 1) * bins];
            let dst = &mut out[t * bins..(t + 1) * bins];
            for (d, s) in dst.iter_mut().zip(src) {
                *d += w * s;
            }
        }
        for d in &mut out[t * bins..(t + 1) * bins] {
            *d /= weight;
        }
    }
    out
}

pub(crate) fn mask(mags: &[f32], grid: Grid, threshold: &[f64], prop_decrease: f32) -> Vec<f32> {
    let db = amp_to_db(mags, grid.frames, grid.bins);
    let prop = f64::from(prop_decrease);
    let hard: Vec<f64> = db
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let above = if *d > threshold[i % grid.bins] {
                1.0
            } else {
                0.0
            };
            above * prop + (1.0 - prop)
        })
        .collect();
    let (freq, time) = smoothing(grid.rate, grid.hop);
    convolve_same(&hard, grid.frames, grid.bins, &freq, &time)
        .into_iter()
        .map(|v| v as f32)
        .collect()
}
