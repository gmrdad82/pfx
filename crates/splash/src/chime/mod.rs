use pfx_core::sim::pow;
use pfx_sound::tone::{Biquad, Fm, Noise, Osc, Partial, Pass, bell, fm, pan, smooth};
use pfx_sound::{Clip, Limiter, Room, reverb};

use crate::compose::{RULE_FROM, RULE_TO, ease_out};
use crate::timeline::LENGTH;

pub const RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
pub const CEILING_DB: f32 = LIMIT_DB + LEVEL_DB;
pub const LEVEL_DB: f32 = -14.0;

const LIMIT_DB: f32 = -1.5;

const GAIN: f32 = 0.978;
const FADE_FROM: f64 = 1.55;
const TUNE_EVERY: usize = 16;
const DETUNE: [f64; 5] = [-14.0, -6.0, 0.0, 6.0, 14.0];
const SWEEP_FROM: f64 = 146.83;
const TONE: f64 = 1174.66;

const ROOM: Room = Room {
    size: 0.88,
    damp: 0.3,
    wet: 0.4,
    width: 1.0,
};

const PURE: [Partial; 3] = [
    Partial {
        ratio: 1.0,
        amp: 1.0,
        decay: 0.65,
    },
    Partial {
        ratio: 2.0,
        amp: 0.18,
        decay: 0.3,
    },
    Partial {
        ratio: 3.0,
        amp: 0.05,
        decay: 0.15,
    },
];

const BODY: [Partial; 2] = [
    Partial {
        ratio: 1.0,
        amp: 1.0,
        decay: 0.4,
    },
    Partial {
        ratio: 2.0,
        amp: 0.2,
        decay: 0.2,
    },
];

const GLINT: Fm = Fm {
    hz: 2349.32,
    ratio: 3.5,
    index: 2.5,
    index_decay: 0.03,
    decay: 0.12,
};

pub fn chime() -> Clip {
    let frames = (LENGTH * f64::from(RATE)).round() as usize;
    let mut dry = vec![[0.0f32; 2]; frames];
    sweep(&mut dry);
    bell(&mut dry, frame(RULE_TO), TONE, &PURE, 0.4, 0.002, RATE);
    fm(&mut dry, frame(RULE_TO), GLINT, 0.08, RATE);
    bell(
        &mut dry,
        frame(RULE_TO),
        SWEEP_FROM,
        &BODY,
        0.28,
        0.012,
        RATE,
    );
    let wet = reverb(&dry, RATE, ROOM);
    let mut samples = Vec::with_capacity(frames * usize::from(CHANNELS));
    for (n, (d, w)) in dry.iter().zip(&wet).enumerate() {
        let t = seconds(n);
        let fade = (1.0 - smooth((t - FADE_FROM) / (LENGTH - FADE_FROM))) as f32;
        samples.push((d[0] + w[0]) * fade * GAIN);
        samples.push((d[1] + w[1]) * fade * GAIN);
    }
    Limiter::new(LIMIT_DB).master(&mut samples, usize::from(CHANNELS), RATE);
    let level = pow(10.0, f64::from(LEVEL_DB) / 20.0) as f32;
    for sample in &mut samples {
        *sample *= level;
    }
    Clip {
        rate: RATE,
        channels: CHANNELS,
        samples,
    }
}

fn seconds(n: usize) -> f64 {
    n as f64 / f64::from(RATE)
}

fn frame(t: f64) -> usize {
    (t * f64::from(RATE)).round() as usize
}

fn grow(t: f64) -> f64 {
    f64::from(ease_out((t - RULE_FROM) / (RULE_TO - RULE_FROM)))
}

fn sweep(dry: &mut [[f32; 2]]) {
    let rate = f64::from(RATE);
    let mut saws: [Osc; 5] = std::array::from_fn(|i| Osc::at(i as f64 * 0.19));
    let mut low = [Biquad::new(Pass::Low), Biquad::new(Pass::Low)];
    let mut noise = [Noise::new(0xc0e7), Noise::new(0xc0e8)];
    let mut band = [Biquad::new(Pass::Band), Biquad::new(Pass::Band)];
    for n in frame(RULE_FROM)..frame(RULE_TO + 0.08).min(dry.len()) {
        let t = seconds(n);
        let g = grow(t);
        let amp =
            (smooth((t - RULE_FROM) / 0.3) * (1.0 - smooth((t - RULE_TO + 0.01) / 0.07))) as f32;
        if n % TUNE_EVERY == 0 {
            for b in &mut low {
                b.tune(220.0 * pow(2.0, 5.2 * g), 2.2, rate);
            }
            for b in &mut band {
                b.tune(1800.0 * pow(2.0, 2.4 * g), 2.5, rate);
            }
        }
        let base = SWEEP_FROM * pow(2.0, g);
        let mut mix = [0.0f32; 2];
        for (i, (osc, cents)) in saws.iter_mut().zip(DETUNE).enumerate() {
            let hz = base * pow(2.0, cents / 1200.0);
            let spread = (i as f32 - 2.0) / 2.0 * g as f32 * 0.8;
            let [l, r] = pan(osc.saw(hz, rate) * 0.12, spread);
            mix[0] += l;
            mix[1] += r;
        }
        for c in 0..2 {
            let air = band[c].process(noise[c].sample()) * 0.4;
            dry[n][c] += (low[c].process(mix[c]) + air) * amp;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ebur128::{EbuR128, Mode};

    #[test]
    fn the_chime_is_level_bounded_and_silent_at_its_edges() {
        let clip = chime();
        assert_eq!(clip.samples, chime().samples);
        let channels = usize::from(CHANNELS);
        assert_eq!(clip.frames(), (LENGTH * f64::from(RATE)) as u64);
        let before = frame(RULE_FROM) * channels;
        assert!(clip.samples[..before].iter().all(|s| *s == 0.0));
        let tail = &clip.samples[clip.samples.len() - channels * 48..];
        assert!(tail.iter().all(|s| s.abs() < 1e-4));
        let mut meter = EbuR128::new(u32::from(CHANNELS), RATE, Mode::S | Mode::TRUE_PEAK).unwrap();
        let mut short = f64::NEG_INFINITY;
        let mut padded = clip.samples.clone();
        padded.extend(std::iter::repeat_n(0.0, RATE as usize * channels));
        for block in padded.chunks(RATE as usize / 10 * channels) {
            meter.add_frames_f32(block).unwrap();
            short = short.max(meter.loudness_shortterm().unwrap());
        }
        assert!((short + 30.0).abs() < 0.5, "short-term {short:.2} LUFS");
        let peak = (0..u32::from(CHANNELS))
            .map(|c| meter.true_peak(c).unwrap())
            .fold(0.0f64, f64::max);
        let peak_db = 20.0 * peak.log10();
        assert!(peak_db < -15.0, "true peak {peak_db:.2} dBTP");
    }
}
