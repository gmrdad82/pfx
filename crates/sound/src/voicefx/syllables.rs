use super::vocoder::band_powers;
use crate::Resample;
use crate::interp::{Kernel, MAX_RATIO, MIN_RATIO, hermite};
use crate::synth::sine;
use pfx_core::sim::Rng;

const AMIN: f32 = 1e-10;
const TOP_DB: f32 = 80.0;
const PRE_MAX: f32 = 0.03;
const POST_MAX: f32 = 0.0;
const PRE_AVG: f32 = 0.10;
const POST_AVG: f32 = 0.10;
const WAIT: f32 = 0.03;
const DELTA: f32 = 0.07;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Syllable {
    pub start: usize,
    pub end: usize,
}

impl Syllable {
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    pub hop: f32,
    pub open_db: f32,
    pub close_db: f32,
    pub min_gap: f32,
    pub min_length: f32,
    pub onsets: bool,
}

impl Default for Split {
    fn default() -> Self {
        Self {
            hop: 0.01,
            open_db: -30.0,
            close_db: -40.0,
            min_gap: 0.04,
            min_length: 0.06,
            onsets: false,
        }
    }
}

pub fn syllables(samples: &[f32], channels: usize, rate: u32, split: &Split) -> Vec<Syllable> {
    let channels = channels.max(1);
    let rate_f = rate.max(1) as f32;
    let frames = samples.len() / channels;
    let hop = ((split.hop * rate_f).round() as usize).max(1);
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    let energy: Vec<f32> = mono
        .chunks(hop)
        .map(|chunk| chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32)
        .collect();
    let loudest = energy.iter().copied().fold(0.0f32, f32::max);
    if loudest <= AMIN {
        return Vec::new();
    }
    let db = |power: f32| 10.0 * power.max(AMIN).log10();
    let peak = db(loudest);
    let open = peak + split.open_db.min(0.0);
    let close = peak + split.close_db.min(split.open_db.min(0.0));
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (h, power) in energy.iter().enumerate() {
        let level = db(*power);
        match start {
            None if level >= open => start = Some(h),
            Some(s) if level < close => {
                runs.push((s, h));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push((s, energy.len()));
    }
    let in_hops = |seconds: f32| (seconds.max(0.0) * rate_f / hop as f32).ceil() as usize;
    let gap = in_hops(split.min_gap);
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(runs.len());
    for run in runs {
        match merged.last_mut() {
            Some(last) if run.0 - last.1 < gap => last.1 = run.1,
            _ => merged.push(run),
        }
    }
    let shortest = in_hops(split.min_length).max(1);
    if split.onsets {
        let onsets = onsets(&mono, rate, hop);
        merged = merged
            .into_iter()
            .flat_map(|(s, e)| {
                let mut pieces = Vec::new();
                let mut from = s;
                for onset in onsets.iter().copied().filter(|o| *o > s && *o < e) {
                    if onset - from >= shortest && e - onset >= shortest {
                        pieces.push((from, onset));
                        from = onset;
                    }
                }
                pieces.push((from, e));
                pieces
            })
            .collect();
    }
    merged
        .into_iter()
        .filter(|(s, e)| e - s >= shortest)
        .map(|(s, e)| Syllable {
            start: s * hop,
            end: (e * hop).min(frames),
        })
        .collect()
}

fn onsets(mono: &[f32], rate: u32, hop: usize) -> Vec<usize> {
    let powers = band_powers(mono, rate, hop);
    let db: Vec<Vec<f32>> = powers
        .iter()
        .map(|bands| bands.iter().map(|p| 10.0 * p.max(AMIN).log10()).collect())
        .collect();
    let top = db.iter().flatten().copied().fold(f32::MIN, f32::max) - TOP_DB;
    let mut flux = vec![0.0f32; db.len()];
    for t in 1..db.len() {
        let rise: f32 = db[t]
            .iter()
            .zip(&db[t - 1])
            .map(|(now, before)| (now.max(top) - before.max(top)).max(0.0))
            .sum();
        flux[t] = rise / db[t].len() as f32;
    }
    let low = flux.iter().copied().fold(f32::MAX, f32::min);
    let high = flux.iter().copied().fold(f32::MIN, f32::max) - low;
    if flux.is_empty() || high <= 0.0 {
        return Vec::new();
    }
    for x in &mut flux {
        *x = (*x - low) / (high + f32::MIN_POSITIVE);
    }
    let per_hop = rate.max(1) as f32 / hop as f32;
    let frames = |seconds: f32| (seconds * per_hop).floor() as usize;
    peak_pick(
        &flux,
        frames(PRE_MAX),
        frames(POST_MAX) + 1,
        frames(PRE_AVG),
        frames(POST_AVG) + 1,
        DELTA,
        frames(WAIT),
    )
}

fn peak_pick(
    x: &[f32],
    pre_max: usize,
    post_max: usize,
    pre_avg: usize,
    post_avg: usize,
    delta: f32,
    wait: usize,
) -> Vec<usize> {
    let len = x.len();
    let max_of = |range: std::ops::Range<usize>| x[range].iter().copied().fold(f32::MIN, f32::max);
    let mean_of = |range: std::ops::Range<usize>| {
        let count = range.len().max(1);
        x[range].iter().sum::<f32>() / count as f32
    };
    let mut peaks = Vec::new();
    if len == 0 {
        return peaks;
    }
    let mut n = 1;
    if x[0] >= max_of(0..post_max.min(len)) && x[0] >= mean_of(0..post_avg.min(len)) + delta {
        peaks.push(0);
        n = wait + 1;
    }
    while n < len {
        let local = max_of(n.saturating_sub(pre_max)..(n + post_max).min(len));
        if x[n] == local
            && x[n] >= mean_of(n.saturating_sub(pre_avg)..(n + post_avg).min(len)) + delta
        {
            peaks.push(n);
            n += wait + 1;
        } else {
            n += 1;
        }
    }
    peaks
}

pub fn cut(samples: &[f32], channels: usize, syllables: &[Syllable]) -> Vec<Vec<f32>> {
    let channels = channels.max(1);
    syllables
        .iter()
        .map(|s| {
            let end = (s.end * channels).min(samples.len());
            let start = (s.start * channels).min(end);
            samples[start..end].to_vec()
        })
        .collect()
}

pub fn reverse(grain: &[f32], channels: usize) -> Vec<f32> {
    grain
        .chunks(channels.max(1))
        .rev()
        .flatten()
        .copied()
        .collect()
}

pub fn repitch(grain: &[f32], channels: usize, ratio: f32, resample: Resample) -> Vec<f32> {
    let channels = channels.max(1);
    let ratio = if ratio.is_finite() {
        ratio.clamp(MIN_RATIO, MAX_RATIO)
    } else {
        1.0
    };
    let frames = grain.len() / channels;
    if frames == 0 {
        return Vec::new();
    }
    let produced = (frames as f64 / f64::from(ratio)).round() as usize;
    let at = |k: i64, c: usize| grain[k.clamp(0, frames as i64 - 1) as usize * channels + c];
    let kernel = matches!(resample, Resample::Sinc).then(Kernel::new);
    let mut out = Vec::with_capacity(produced * channels);
    for i in 0..produced {
        let position = i as f64 * f64::from(ratio);
        let index = position.floor() as i64;
        let t = (position - index as f64) as f32;
        match &kernel {
            Some(kernel) => {
                let taps = kernel.taps(position, ratio);
                for c in 0..channels {
                    let mut sum = 0.0f32;
                    for (j, weight) in taps.weights[..taps.count].iter().enumerate() {
                        let k = taps.first + j as i64;
                        if k >= 0 && k < frames as i64 {
                            sum += grain[k as usize * channels + c] * weight;
                        }
                    }
                    out.push(sum);
                }
            }
            None => {
                for c in 0..channels {
                    out.push(match resample {
                        Resample::Linear => at(index, c) + (at(index + 1, c) - at(index, c)) * t,
                        _ => hermite(
                            at(index - 1, c),
                            at(index, c),
                            at(index + 1, c),
                            at(index + 2, c),
                            t,
                        ),
                    });
                }
            }
        }
    }
    out
}

pub fn permutation(len: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    Rng::new(seed).shuffle(&mut order);
    order
}

pub fn reorder<T: Clone>(items: &[T], seed: u64) -> Vec<T> {
    permutation(items.len(), seed)
        .into_iter()
        .map(|i| items[i].clone())
        .collect()
}

pub fn join(grains: &[Vec<f32>], channels: usize, rate: u32, crossfade: f32) -> Vec<f32> {
    let channels = channels.max(1);
    let fade = (crossfade.max(0.0) * rate as f32).round() as usize;
    let mut out: Vec<f32> = Vec::with_capacity(grains.iter().map(Vec::len).sum());
    let mut tail = 0usize;
    for grain in grains {
        let frames = grain.len() / channels;
        let overlap = fade.min(frames).min(tail);
        let start = out.len() / channels - overlap;
        for j in 0..overlap {
            let t = (j as f64 + 0.5) / overlap as f64;
            let rise = sine(t * 0.25);
            let fall = sine(0.25 + t * 0.25);
            for c in 0..channels {
                let slot = &mut out[(start + j) * channels + c];
                *slot = *slot * fall + grain[j * channels + c] * rise;
            }
        }
        out.extend_from_slice(&grain[overlap * channels..frames * channels]);
        tail = frames - overlap;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound::Rng as Noise;

    fn bursts(rate: u32, plan: &[(f32, f32, f32)]) -> (Vec<f32>, Vec<(usize, usize)>) {
        let mut noise = Noise::new(9);
        let mut out = Vec::new();
        let mut truth = Vec::new();
        for (gap, length, hz) in plan {
            out.extend((0..(gap * rate as f32) as usize).map(|_| 0.0005 * noise.signed()));
            let start = out.len();
            let frames = (length * rate as f32) as usize;
            out.extend((0..frames).map(|i| {
                let ramp = (i.min(frames - i) as f32 / 240.0).min(1.0);
                let phase = std::f32::consts::TAU * hz * i as f32 / rate as f32;
                ramp * 0.4 * (phase.sin() + 0.5 * (2.0 * phase).sin()) + 0.0005 * noise.signed()
            }));
            truth.push((start, out.len()));
        }
        out.extend((0..rate as usize / 10).map(|_| 0.0005 * noise.signed()));
        (out, truth)
    }

    #[test]
    fn the_split_finds_each_burst_between_silences() {
        let rate = 48_000;
        let plan = [
            (0.1, 0.15, 180.0),
            (0.12, 0.2, 220.0),
            (0.1, 0.12, 260.0),
            (0.15, 0.18, 200.0),
            (0.1, 0.16, 240.0),
        ];
        let (audio, truth) = bursts(rate, &plan);
        for onsets in [false, true] {
            let found = syllables(
                &audio,
                1,
                rate,
                &Split {
                    onsets,
                    ..Split::default()
                },
            );
            assert_eq!(found.len(), truth.len(), "{onsets}: {found:?}");
            for (syllable, (start, end)) in found.iter().zip(&truth) {
                assert!(
                    syllable.start.abs_diff(*start) <= 960,
                    "{syllable:?} {start}"
                );
                assert!(syllable.end.abs_diff(*end) <= 960, "{syllable:?} {end}");
            }
        }
    }

    #[test]
    fn onsets_split_a_run_where_a_louder_syllable_starts_without_a_gap() {
        let rate = 48_000;
        let mut audio: Vec<f32> = vec![0.0; 4_800];
        audio.extend(
            (0..9_600).map(|i| 0.06 * (std::f32::consts::TAU * 200.0 * i as f32 / 48_000.0).sin()),
        );
        audio.extend(
            (0..9_600).map(|i| 0.5 * (std::f32::consts::TAU * 330.0 * i as f32 / 48_000.0).sin()),
        );
        audio.extend(vec![0.0; 4_800]);
        let plain = syllables(&audio, 1, rate, &Split::default());
        assert_eq!(plain.len(), 1, "{plain:?}");
        let split = syllables(
            &audio,
            1,
            rate,
            &Split {
                onsets: true,
                ..Split::default()
            },
        );
        assert_eq!(split.len(), 2, "{split:?}");
        assert!(split[1].start.abs_diff(14_400) <= 960, "{split:?}");
    }

    #[test]
    fn the_grain_helpers_keep_their_lengths_and_the_seed_fixes_the_order() {
        let grains: Vec<Vec<f32>> = (1..=6).map(|n| vec![n as f32; 2 * 1_000 * n]).collect();
        assert_eq!(reorder(&grains, 4), reorder(&grains, 4));
        assert_ne!(permutation(6, 4), permutation(6, 5));
        let mut sorted = permutation(6, 4);
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4, 5]);
        let total: usize = grains.iter().map(Vec::len).sum();
        let joined = join(&grains, 2, 48_000, 0.01);
        assert_eq!(joined.len(), total - 5 * 480 * 2);
        let ramp: Vec<f32> = (0..2_000).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let back = reverse(&ramp, 2);
        assert_eq!(&back[..2], &[1_999.0, -1_999.0]);
        for resample in [Resample::Sinc, Resample::Cubic, Resample::Linear] {
            assert_eq!(repitch(&ramp, 2, 2.0, resample).len(), 2_000);
            assert_eq!(repitch(&ramp, 2, 0.5, resample).len(), 8_000);
        }
        let tone: Vec<f32> = (0..24_000)
            .map(|i| (std::f32::consts::TAU * 300.0 * i as f32 / 48_000.0).sin())
            .collect();
        let up = repitch(&tone, 1, 1.5, Resample::Cubic);
        assert_eq!(up.len(), 16_000);
        assert!(crate::interp::tone_level(&up, 48_000.0, 450.0) > 0.95);
        let cut_grains = cut(&ramp, 2, &[Syllable { start: 10, end: 20 }]);
        assert_eq!(cut_grains[0].len(), 20);
        assert_eq!(cut_grains[0][0], 10.0);
    }
}
