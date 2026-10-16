use super::spectrum::Grid;

pub const LOW_HZ: f32 = 40.0;
pub const HIGH_HZ: f32 = 400.0;
const STEPS_PER_OCTAVE: f32 = 96.0;
const COMPRESSION: f32 = 0.84;
const MAX_HARMONICS: usize = 30;
const TOP_HZ: f32 = 5000.0;
const RANGE_DB: f32 = 80.0;
const JUMP: usize = 8;
const STEP_COST: f32 = 0.5;
const VOICED_DB: f32 = 3.0;
const MAIN_LOBE: f32 = 1.0;
const ROLL: f32 = 1.5;
pub(crate) const FLOOR_DB: f32 = -60.0;

pub(crate) struct Track {
    pub(crate) f0: Vec<f32>,
    pub(crate) voiced: f32,
}

fn levels(mags: &[f32], bins: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(mags.len());
    for row in mags.chunks_exact(bins) {
        let db: Vec<f32> = row.iter().map(|m| 20.0 * (m + 1e-12).log10()).collect();
        let top = db.iter().copied().fold(f32::MIN, f32::max);
        out.extend(db.into_iter().map(|d| d.max(top - RANGE_DB)));
    }
    out
}

fn gap(row: &[f32], from: f32, to: f32) -> f32 {
    let last = (row.len() - 1) as f32;
    let a = from.clamp(0.0, last).round() as usize;
    let b = to.clamp(0.0, last).round() as usize;
    row[a..=b.max(a)].iter().copied().fold(f32::MIN, f32::max)
}

fn peak(row: &[f32], bin: f32) -> f32 {
    let b = bin.clamp(0.0, (row.len() - 1) as f32);
    let i = b.floor() as usize;
    let j = (i + 1).min(row.len() - 1);
    row[i].max(row[j])
}

fn score(row: &[f32], grid: Grid, f0: f32) -> f32 {
    let top = TOP_HZ.min(grid.rate as f32 * 0.45);
    let count = ((top / f0) as usize).min(MAX_HARMONICS);
    if count == 0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut weights = 0.0;
    let mut w = 1.0;
    for h in 1..=count {
        let h = h as f32;
        let crest = peak(row, grid.bin(h * f0));
        let valley = gap(row, grid.bin((h + 0.25) * f0), grid.bin((h + 0.75) * f0));
        sum += w * (crest - valley);
        weights += w;
        w *= COMPRESSION;
    }
    sum / weights
}

pub(crate) fn track(mags: &[f32], grid: Grid, low: f32, high: f32) -> Track {
    let rows = levels(mags, grid.bins);
    let steps = ((high / low).log2() * STEPS_PER_OCTAVE).ceil() as usize + 1;
    let candidate = |c: f32| low * (c / STEPS_PER_OCTAVE).exp2();
    let frames = grid.frames;
    let mut scores = vec![0.0f32; frames * steps];
    for t in 0..frames {
        let row = &rows[t * grid.bins..(t + 1) * grid.bins];
        for c in 0..steps {
            scores[t * steps + c] = score(row, grid, candidate(c as f32));
        }
    }
    let mut total = scores[..steps].to_vec();
    let mut back = vec![0u16; frames * steps];
    let mut next = vec![0.0f32; steps];
    for t in 1..frames {
        for (c, slot) in next.iter_mut().enumerate() {
            let from = c.saturating_sub(JUMP);
            let to = (c + JUMP).min(steps - 1);
            let mut best = f32::MIN;
            let mut arg = c;
            for (p, value) in total.iter().enumerate().take(to + 1).skip(from) {
                let v = value - STEP_COST * p.abs_diff(c) as f32;
                if v > best {
                    best = v;
                    arg = p;
                }
            }
            *slot = best + scores[t * steps + c];
            back[t * steps + c] = arg as u16;
        }
        std::mem::swap(&mut total, &mut next);
    }
    let mut c = (0..steps)
        .max_by(|a, b| total[*a].total_cmp(&total[*b]).then(b.cmp(a)))
        .unwrap_or(0);
    let mut path = vec![0usize; frames];
    for t in (0..frames).rev() {
        path[t] = c;
        if t > 0 {
            c = back[t * steps + c] as usize;
        }
    }
    let mut voiced = 0usize;
    let f0 = path
        .iter()
        .enumerate()
        .map(|(t, &c)| {
            let s = &scores[t * steps..(t + 1) * steps];
            if s[c] > VOICED_DB {
                voiced += 1;
            }
            let shift = if c > 0 && c + 1 < steps {
                let (a, b, d) = (s[c - 1], s[c], s[c + 1]);
                let den = a - 2.0 * b + d;
                if den < 0.0 {
                    (0.5 * (a - d) / den).clamp(-0.5, 0.5)
                } else {
                    0.0
                }
            } else {
                0.0
            };
            candidate(c as f32 + shift)
        })
        .collect();
    Track {
        f0,
        voiced: voiced as f32 / frames.max(1) as f32,
    }
}

pub(crate) fn comb(track: &Track, grid: Grid) -> Vec<f32> {
    let floor = 10f32.powf(FLOOR_DB / 20.0);
    let bin_hz = grid.bin_hz();
    let frames = grid.frames;
    let mut gains = vec![0.0f32; frames * grid.bins];
    for t in 0..frames {
        let f0 = track.f0[t];
        let before = track.f0[t.saturating_sub(1)];
        let after = track.f0[(t + 1).min(frames - 1)];
        let drift = (after - before).abs() * 0.5;
        let row = &mut gains[t * grid.bins..(t + 1) * grid.bins];
        let mut h = 1usize;
        loop {
            let centre = grid.bin(h as f32 * f0);
            let half = MAIN_LOBE + 2.0 * h as f32 * drift / bin_hz;
            if centre - half - ROLL > grid.bins as f32 {
                break;
            }
            let from = (centre - half - ROLL).floor().max(0.0) as usize;
            let to = ((centre + half + ROLL).ceil() as usize).min(grid.bins - 1);
            for (k, g) in row.iter_mut().enumerate().take(to + 1).skip(from) {
                let x = (k as f32 - centre).abs();
                let lobe = if x <= half {
                    1.0
                } else if x < half + ROLL {
                    0.5 + 0.5 * (std::f32::consts::PI * (x - half) / ROLL).cos()
                } else {
                    0.0
                };
                *g = g.max(lobe);
            }
            h += 1;
        }
        for g in row.iter_mut() {
            *g = floor + (1.0 - floor) * *g;
        }
    }
    gains
}
