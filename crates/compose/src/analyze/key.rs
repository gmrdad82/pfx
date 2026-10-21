use serde::Serialize;

use crate::score::Note;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Krumhansl,
    Temperley,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    pub tonic: String,
    pub mode: String,
    pub correlation: f64,
}

impl Found {
    pub fn name(&self) -> String {
        format!("{} {}", self.tonic, self.mode)
    }
}

const KRUMHANSL_MAJOR: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const KRUMHANSL_MINOR: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];
const TEMPERLEY_MAJOR: [f64; 12] = [
    0.748, 0.060, 0.488, 0.082, 0.670, 0.460, 0.096, 0.715, 0.104, 0.366, 0.057, 0.400,
];
const TEMPERLEY_MINOR: [f64; 12] = [
    0.712, 0.084, 0.474, 0.618, 0.049, 0.460, 0.105, 0.747, 0.404, 0.067, 0.133, 0.330,
];

const MAJOR_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
];
const MINOR_NAMES: [&str; 12] = [
    "C", "C#", "D", "Eb", "E", "F", "F#", "G", "G#", "A", "Bb", "B",
];

pub(crate) const MODES: [&str; 2] = ["major", "minor"];

fn weights(profile: Profile, mode: &str) -> &'static [f64; 12] {
    match (profile, mode) {
        (Profile::Krumhansl, "major") => &KRUMHANSL_MAJOR,
        (Profile::Krumhansl, _) => &KRUMHANSL_MINOR,
        (Profile::Temperley, "major") => &TEMPERLEY_MAJOR,
        (Profile::Temperley, _) => &TEMPERLEY_MINOR,
    }
}

pub(crate) fn tonic_name(tonic: usize, mode: &str) -> &'static str {
    if mode == "major" {
        MAJOR_NAMES[tonic % 12]
    } else {
        MINOR_NAMES[tonic % 12]
    }
}

pub(crate) fn pitch_class(midi: f64) -> Option<usize> {
    midi.is_finite()
        .then(|| (midi.round() as i64).rem_euclid(12) as usize)
}

pub fn histogram(notes: &[Note]) -> [f64; 12] {
    let mut out = [0.0; 12];
    for note in notes {
        let Some(pc) = pitch_class(note.midi) else {
            continue;
        };
        if note.beats.is_finite() && note.beats > 0.0 {
            out[pc] += note.beats;
        }
    }
    out
}

pub(crate) fn correlation(histogram: &[f64; 12], profile: &[f64; 12], tonic: usize) -> f64 {
    let profile_mean = profile.iter().sum::<f64>() / 12.0;
    let histogram_mean = histogram.iter().sum::<f64>() / 12.0;
    let mut top = 0.0;
    let mut right = 0.0;
    let mut left = 0.0;
    for (j, h) in histogram.iter().enumerate() {
        let p = profile[(j + 12 - tonic) % 12] - profile_mean;
        let d = h - histogram_mean;
        top += p * d;
        right += p * p;
        left += d * d;
    }
    let bottom = (right * left).sqrt();
    if bottom > 0.0 && bottom.is_finite() {
        top / bottom
    } else {
        0.0
    }
}

pub fn rank(histogram: &[f64; 12], profile: Profile) -> Vec<Found> {
    let mut out = Vec::with_capacity(24);
    for mode in MODES {
        let table = weights(profile, mode);
        for tonic in 0..12 {
            out.push(Found {
                tonic: tonic_name(tonic, mode).to_string(),
                mode: mode.to_string(),
                correlation: correlation(histogram, table, tonic),
            });
        }
    }
    out.sort_by(|a, b| b.correlation.total_cmp(&a.correlation));
    out
}

pub(crate) fn best(histogram: &[f64; 12]) -> Vec<Found> {
    let mut temperley = rank(histogram, Profile::Temperley);
    let krumhansl = rank(histogram, Profile::Krumhansl);
    let tie = |found: &Found| {
        krumhansl
            .iter()
            .find(|k| k.tonic == found.tonic && k.mode == found.mode)
            .map_or(0.0, |k| k.correlation)
    };
    temperley.sort_by(|a, b| {
        b.correlation
            .total_cmp(&a.correlation)
            .then_with(|| tie(b).total_cmp(&tie(a)))
    });
    temperley
}
