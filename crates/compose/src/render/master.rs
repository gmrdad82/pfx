use pfx_core::sim;
use pfx_sound::{Limiter, Loudness, Room, decibels, reverb, true_peak};

use super::RATE;

pub const ROOM_STEM: &str = "room";
const SETTLE_DB: f64 = 0.25;
const PASSES: usize = 3;
const HOP: usize = (RATE / 10) as usize;

#[derive(Clone, Debug, PartialEq)]
pub struct Stem {
    pub name: String,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mastered {
    pub mix: Vec<f32>,
    pub gain_db: f64,
    pub short_term_max: f64,
    pub integrated: f64,
    pub true_peak_db: f64,
}

pub struct Chain<'a> {
    pub lufs: f64,
    pub true_peak: f32,
    pub room: [f32; 3],
    pub sidechain: Option<Sidechain<'a>>,
    pub fade: f64,
}

pub struct Sidechain<'a> {
    pub keys: &'a [String],
    pub beds: &'a [String],
    pub depth: f32,
    pub release_ms: f32,
}

pub fn decibels_to_gain(db: f64) -> f64 {
    sim::pow(10.0, db / 20.0)
}

pub fn duck(stems: &mut [Stem], sidechain: &Sidechain, frames: usize) {
    let mut key = vec![0.0f32; frames * 2];
    for stem in stems.iter().filter(|s| sidechain.keys.contains(&s.name)) {
        for (a, b) in key.iter_mut().zip(&stem.samples) {
            *a += b;
        }
    }
    let peak = key.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak <= 1e-6 {
        return;
    }
    let release = sim::expf(-1.0 / (sidechain.release_ms.max(1.0) / 1000.0 * RATE as f32));
    let mut env = 0.0f32;
    let gains: Vec<f32> = key
        .chunks_exact(2)
        .map(|f| {
            let level = f[0].abs().max(f[1].abs()) / peak;
            env = if level > env { level } else { env * release };
            1.0 - sidechain.depth * (env * 2.0).min(1.0)
        })
        .collect();
    for stem in stems
        .iter_mut()
        .filter(|s| sidechain.beds.contains(&s.name))
    {
        for (frame, g) in stem.samples.chunks_exact_mut(2).zip(&gains) {
            frame[0] *= g;
            frame[1] *= g;
        }
    }
}

pub fn room(stems: &[Stem], sends: &[f32], room: [f32; 3], frames: usize) -> Option<Stem> {
    let [size, damp, wet] = room;
    if wet <= 0.0 || sends.iter().all(|s| *s <= 0.0) {
        return None;
    }
    let mut dry = vec![[0.0f32; 2]; frames];
    for (stem, send) in stems.iter().zip(sends) {
        if *send <= 0.0 {
            continue;
        }
        for (d, f) in dry.iter_mut().zip(stem.samples.chunks_exact(2)) {
            d[0] += f[0] * send;
            d[1] += f[1] * send;
        }
    }
    let wet = reverb(
        &dry,
        RATE,
        Room {
            size,
            damp,
            wet,
            width: 1.0,
        },
    );
    Some(Stem {
        name: ROOM_STEM.to_string(),
        samples: wet.iter().flat_map(|f| [f[0], f[1]]).collect(),
    })
}

pub fn sum(stems: &[Stem], frames: usize) -> Vec<f32> {
    let mut mix = vec![0.0f32; frames * 2];
    for stem in stems {
        for (m, v) in mix.iter_mut().zip(&stem.samples) {
            *m += v;
        }
    }
    mix
}

pub fn fade_out(mix: &mut [f32], seconds: f64) {
    let frames = mix.len() / 2;
    let span = ((seconds * f64::from(RATE)) as usize).min(frames);
    if span == 0 {
        return;
    }
    let start = frames - span;
    for i in start..frames {
        let t = (i - start) as f64 / span as f64;
        let g = (1.0 - t * t * (3.0 - 2.0 * t)) as f32;
        mix[i * 2] *= g;
        mix[i * 2 + 1] *= g;
    }
}

pub fn measure(samples: &[f32]) -> (f64, f64) {
    let mut meter = Loudness::new(2, RATE);
    let mut most = f64::NEG_INFINITY;
    for block in samples.chunks(HOP * 2) {
        meter.add(block);
        most = most.max(meter.short_term());
    }
    (most, meter.integrated())
}

pub fn master(mix: &[f32], lufs: f64, ceiling: f32) -> Mastered {
    let limiter = Limiter::new(ceiling - 0.05);
    let (before, _) = measure(mix);
    let mut gain_db = if before.is_finite() {
        lufs - before
    } else {
        0.0
    };
    let mut pass = 1;
    loop {
        let g = decibels_to_gain(gain_db) as f32;
        let mut out: Vec<f32> = mix.iter().map(|v| v * g).collect();
        limiter.master(&mut out, 2, RATE);
        let (short_term_max, integrated) = measure(&out);
        let delta = lufs - short_term_max;
        if !short_term_max.is_finite() || delta.abs() < SETTLE_DB || pass == PASSES {
            let true_peak_db = f64::from(decibels(true_peak(&out, 2)));
            return Mastered {
                mix: out,
                gain_db,
                short_term_max,
                integrated,
                true_peak_db,
            };
        }
        gain_db += delta;
        pass += 1;
    }
}

pub fn chain(
    stems: &mut Vec<Stem>,
    sends: &[f32],
    fired: &[(u64, &Stem)],
    chain: &Chain,
    frames: usize,
) -> (Mastered, f32) {
    if let Some(sidechain) = &chain.sidechain {
        duck(stems, sidechain, frames);
    }
    if let Some(room) = room(stems, sends, chain.room, frames) {
        stems.push(room);
    }
    let mut mix = sum(stems, frames);
    fade_out(&mut mix, chain.fade);
    for (at, cue) in fired {
        let start = (*at as usize).min(frames) * 2;
        for (m, v) in mix[start..].iter_mut().zip(&cue.samples) {
            *m += v;
        }
    }
    let mastered = master(&mix, chain.lufs, chain.true_peak);
    let g = decibels_to_gain(mastered.gain_db) as f32;
    for stem in stems.iter_mut() {
        for v in &mut stem.samples {
            *v *= g;
        }
    }
    (mastered, g)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, level: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = level * sim::sinf(i as f32 * 0.0575);
                [v, v]
            })
            .collect()
    }

    #[test]
    fn the_master_lands_on_the_target_loudness_under_the_ceiling() {
        let frames = RATE as usize * 4;
        let mix = tone(frames, 0.05);
        let done = master(&mix, -16.0, -1.0);
        assert!(
            (done.short_term_max + 16.0).abs() < 0.2,
            "{}",
            done.short_term_max
        );
        assert!(done.true_peak_db <= -1.0 + 1e-3, "{}", done.true_peak_db);
        assert_eq!(done.mix, master(&mix, -16.0, -1.0).mix);
    }

    #[test]
    fn the_sidechain_ducks_only_the_beds_under_a_key() {
        let frames = 48_000;
        let mut key = vec![0.0f32; frames * 2];
        key[0] = 1.0;
        key[1] = 1.0;
        let keys = vec!["kick".to_string()];
        let beds = vec!["pad".to_string()];
        let mut stems = vec![
            Stem {
                name: "kick".into(),
                samples: key,
            },
            Stem {
                name: "pad".into(),
                samples: vec![0.5; frames * 2],
            },
            Stem {
                name: "lead".into(),
                samples: vec![0.5; frames * 2],
            },
        ];
        let sidechain = Sidechain {
            keys: &keys,
            beds: &beds,
            depth: 0.5,
            release_ms: 50.0,
        };
        duck(&mut stems, &sidechain, frames);
        assert!((stems[1].samples[0] - 0.25).abs() < 1e-6);
        assert!(stems[1].samples[frames * 2 - 1] > 0.49);
        assert!(stems[2].samples.iter().all(|v| *v == 0.5));
    }
}
