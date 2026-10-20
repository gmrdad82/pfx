use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::theory::Key;

fn four() -> u32 {
    4
}

fn one() -> f32 {
    1.0
}

fn unit() -> f64 {
    1.0
}

fn is_four(v: &u32) -> bool {
    *v == 4
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

fn is_unit(v: &f64) -> bool {
    *v == 1.0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    pub bpm: [u32; 2],
    #[serde(default = "four", skip_serializing_if = "is_four")]
    pub beats_per_bar: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub swing: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub humanize_ms: f64,
    pub tonics: Vec<(String, u32)>,
    pub modes: Vec<(String, u32)>,
    pub harmony: Harmony,
    #[serde(default)]
    pub melody: Melody,
    #[serde(rename = "role")]
    pub roles: Vec<Role>,
    #[serde(default)]
    pub patch: BTreeMap<String, Patch>,
    #[serde(rename = "section")]
    pub sections: Vec<Section>,
    #[serde(rename = "cue", default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<Cue>,
    pub master: Master,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harmony {
    pub beats: f64,
    pub start: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cadence: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sevenths: bool,
    pub markov: BTreeMap<String, Vec<(String, u32)>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Melody {
    pub range: [i32; 2],
    pub steps: Vec<(i32, u32)>,
    pub rhythm: Vec<(f64, u32)>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rest: f64,
    #[serde(default = "one_bar")]
    pub motif_bars: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub repeat: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub chord_tones: bool,
}

fn one_bar() -> u32 {
    1
}

impl Default for Melody {
    fn default() -> Melody {
        Melody {
            range: [60, 84],
            steps: vec![(1, 1), (-1, 1)],
            rhythm: vec![(1.0, 1)],
            rest: 0.0,
            motif_bars: 1,
            repeat: 0.0,
            chord_tones: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plays {
    Chord,
    Bass,
    Arp,
    Melody,
    Hit,
    Riser,
    Reverse,
    Roll,
    Impact,
}

impl Plays {
    pub fn patterned(self) -> bool {
        matches!(self, Plays::Bass | Plays::Arp | Plays::Hit)
    }

    pub fn event(self) -> bool {
        matches!(
            self,
            Plays::Riser | Plays::Reverse | Plays::Roll | Plays::Impact
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Voicing {
    #[default]
    Lead,
    Root,
    Top,
}

fn is_lead(v: &Voicing) -> bool {
    *v == Voicing::Lead
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub name: String,
    pub patch: String,
    pub plays: Plays,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub euclid: Vec<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rotate: Vec<u32>,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub octave: i32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub gain: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub hold: f64,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub accent: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tones: Vec<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub octaves: Vec<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub semis: Vec<i32>,
    #[serde(default, skip_serializing_if = "is_lead")]
    pub voicing: Voicing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[i32; 2]>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub ramp: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wave {
    Sine,
    Square,
    Saw,
    Triangle,
    Pulse,
    White,
    Pink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveCurve {
    Soft,
    Tanh,
    Hard,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Osc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wave: Option<Wave>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub cents: f32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub octave: i32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub gain: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hz: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_hz: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub glide_ms: f32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub bend: f32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub pan: f32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub at_ms: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub osc: Vec<Osc>,
    pub env: [f32; 4],
    #[serde(default, skip_serializing_if = "is_false")]
    pub linear: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowpass: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highpass: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive: Option<(DriveCurve, f32, f32)>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub send: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub gain: f32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub pan: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub name: String,
    pub bars: [u32; 2],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "is_flat")]
    pub intensity: [f32; 2],
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub round_bars: u32,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub start_roles: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub riser: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub roll: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub impact: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit: Option<usize>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tail: f64,
    #[serde(default = "unit", skip_serializing_if = "is_unit")]
    pub tempo: f64,
    #[serde(default = "unit", skip_serializing_if = "is_unit")]
    pub pitch: f64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub stutter: u32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub lift: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cue: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub gain_db: f32,
}

fn is_flat(v: &[f32; 2]) -> bool {
    *v == [0.0, 0.0]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Master {
    pub lufs: f64,
    pub true_peak: f32,
    #[serde(default, skip_serializing_if = "is_dry")]
    pub room: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain: Option<Sidechain>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade: f64,
}

fn is_dry(v: &[f32; 3]) -> bool {
    *v == [0.0; 3]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sidechain {
    pub keys: Vec<String>,
    pub beds: Vec<String>,
    pub depth: f32,
    pub release_ms: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CuePlays {
    Hit,
    Glide,
    Burst,
    Chord,
}

fn one_beat() -> f64 {
    1.0
}

fn is_one_beat(v: &f64) -> bool {
    *v == 1.0
}

fn one_count() -> u32 {
    1
}

fn is_one_count(v: &u32) -> bool {
    *v == 1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(rename = "voice")]
    pub voices: Vec<CueVoice>,
}

impl Cue {
    pub fn event(&self) -> &str {
        self.event.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueVoice {
    pub patch: String,
    pub plays: CuePlays,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub at: f64,
    #[serde(default = "one_beat", skip_serializing_if = "is_one_beat")]
    pub beats: f64,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub gain: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hz: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade: f64,
    #[serde(default = "one_count", skip_serializing_if = "is_one_count")]
    pub count: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub every: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vels: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeral: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub octave: i32,
}

pub const MAX_BPM: u32 = 400;
pub const MAX_BEATS_PER_BAR: u32 = 32;
pub const MAX_SECONDS: f64 = 600.0;
pub const MAX_BARS: u32 = 4096;
pub const MAX_SECTIONS: usize = 256;
pub const MIN_BEATS: f64 = 1.0 / 64.0;
pub const MAX_BEATS: f64 = 64.0;
pub const MAX_NOTE_BEATS: f64 = MAX_BEATS + MAX_BEATS_PER_BAR as f64;
pub const MAX_STEPS: u32 = 64;
pub const MAX_COUNT: u32 = 256;
pub const MAX_NOTES: usize = 200_000;
pub const MAX_CUE_BEATS: f64 = 32.0;
pub const MAX_CUE_SECONDS: f64 = 30.0;
pub const MAX_GAIN: f64 = 8.0;
pub const MAX_MS: f64 = 60_000.0;
pub const TEMPO_RATIO: [f64; 2] = [0.25, 4.0];
pub const MAX_ROLES: usize = 32;
pub const MAX_CUES: usize = 32;
pub const MAX_CUE_VOICES: usize = 8;
pub const MAX_OSCS: usize = 8;
pub const MAX_CENTS: f64 = 1200.0;
pub const MAX_BEND: f64 = 48.0;
pub const MAX_POLYPHONY: f64 = 256.0;

fn weights<T>(items: &[(T, u32)], what: &str) -> Result<(), String> {
    if items.is_empty() || items.iter().all(|(_, w)| *w == 0) {
        return Err(format!(
            "`{what}` needs at least one choice with a weight above 0"
        ));
    }
    Ok(())
}

pub(crate) fn within(what: &str, value: f64, lo: f64, hi: f64) -> Result<(), String> {
    if value.is_finite() && (lo..=hi).contains(&value) {
        Ok(())
    } else {
        Err(format!("`{what}` is {value}; it wants {lo} to {hi}"))
    }
}

fn whole(what: &str, values: &[i32], limit: i32) -> Result<(), String> {
    match values.iter().find(|v| !(-limit..=limit).contains(*v)) {
        Some(v) => Err(format!("`{what}` holds {v}; it wants -{limit} to {limit}")),
        None => Ok(()),
    }
}

fn hold(what: &str, value: f64) -> Result<(), String> {
    if value == 0.0 {
        Ok(())
    } else {
        within(what, value, MIN_BEATS, MAX_BEATS)
    }
}

fn each(what: &str, values: &[f32], lo: f64, hi: f64) -> Result<(), String> {
    values
        .iter()
        .try_for_each(|v| within(what, f64::from(*v), lo, hi))
}

fn env(values: &[f32; 4]) -> Result<(), String> {
    let [attack, decay, sustain, release] = *values;
    each("env", &[attack, decay, release], 0.0, MAX_MS)?;
    within("env", f64::from(sustain), 0.0, 1.0)
}

pub(crate) fn midi_of(hz: f64) -> f64 {
    69.0 + 12.0 * (pfx_core::sim::ln(hz / 440.0) / pfx_core::sim::ln(2.0))
}

impl Osc {
    fn check(&self) -> Result<(), String> {
        if self.wave.is_some() == self.sample.is_some() {
            return Err("wants either `wave` or `sample`".into());
        }
        if self.bend != 0.0 && self.to_hz.is_some() {
            return Err("has `bend` and `to_hz`; one oscillator takes one glide".into());
        }
        let wave_only = self.width.is_some()
            || self.hz.is_some()
            || self.to_hz.is_some()
            || self.glide_ms != 0.0
            || self.bend != 0.0
            || self.env.is_some();
        if self.sample.is_some() && wave_only {
            return Err("plays a `sample` once from the note's start, pitched from midi 60; `width`, `hz`, `to_hz`, `glide_ms`, `bend` and `env` are for waves".into());
        }
        if let Some(width) = self.width {
            within("width", f64::from(width), 0.01, 0.99)?;
        }
        for (what, hz) in [("hz", self.hz), ("to_hz", self.to_hz)] {
            if let Some(hz) = hz {
                within(what, f64::from(hz), 1.0, 24_000.0)?;
            }
        }
        whole("octave", &[self.octave], 8)?;
        within("cents", f64::from(self.cents), -MAX_CENTS, MAX_CENTS)?;
        within("bend", f64::from(self.bend), -MAX_BEND, MAX_BEND)?;
        within("gain", f64::from(self.gain), 0.0, MAX_GAIN)?;
        within("pan", f64::from(self.pan), -1.0, 1.0)?;
        each(
            "glide_ms or at_ms",
            &[self.glide_ms, self.at_ms],
            0.0,
            MAX_MS,
        )?;
        self.env.as_ref().map_or(Ok(()), env)
    }
}

impl Patch {
    pub fn tail_seconds(&self) -> f64 {
        self.osc
            .iter()
            .map(|o| {
                let [a, d, _, r] = o.env.unwrap_or(self.env);
                f64::from(o.at_ms.max(0.0) + a.max(0.0) + d.max(0.0) + r.max(0.0)) / 1000.0
            })
            .fold(0.0, f64::max)
    }

    fn check(&self) -> Result<(), String> {
        if self.osc.is_empty() || self.osc.len() > MAX_OSCS {
            return Err(format!("a patch wants 1 to {MAX_OSCS} `osc`"));
        }
        for (index, osc) in self.osc.iter().enumerate() {
            osc.check().map_err(|e| format!("osc {}: {e}", index + 1))?;
        }
        env(&self.env)?;
        if self.lowpass.is_some() && self.sweep.is_some() {
            return Err(
                "`lowpass` and `sweep` drive the same filter; give the patch one of them".into(),
            );
        }
        if let Some([lo, hi]) = self.lowpass {
            let (lo, hi) = (f64::from(lo), f64::from(hi));
            within("lowpass", lo, 1.0, 24_000.0)?;
            within("lowpass", hi, lo, 24_000.0)?;
        }
        if let Some(hz) = self.highpass {
            within("highpass", f64::from(hz), 1.0, 24_000.0)?;
        }
        if let Some([a, b]) = self.sweep {
            within("sweep", f64::from(a), 1.0, 24_000.0)?;
            within("sweep", f64::from(b), 1.0, 24_000.0)?;
        }
        if let Some((_, db, mix)) = self.drive {
            within("drive", f64::from(db), -60.0, 60.0)?;
            within("drive", f64::from(mix), 0.0, 1.0)?;
        }
        within("send", f64::from(self.send), 0.0, MAX_GAIN)?;
        each("sweep_ms", &[self.sweep_ms.unwrap_or(0.0)], 0.0, MAX_MS)?;
        within("gain", f64::from(self.gain), 0.0, MAX_GAIN)?;
        within("pan", f64::from(self.pan), -1.0, 1.0)
    }
}

impl CueVoice {
    fn check(&self) -> Result<(), String> {
        within("at", self.at, 0.0, MAX_BEATS)?;
        within("beats", self.beats, MIN_BEATS, MAX_BEATS)?;
        within("gain", f64::from(self.gain), 0.0, MAX_GAIN)?;
        if let Some(midi) = self.midi {
            within("midi", midi, 0.0, 127.0)?;
        }
        within("fade", self.fade, 0.0, MAX_CUE_SECONDS)?;
        for vel in &self.vels {
            within("vels", *vel, 0.0, MAX_GAIN)?;
        }
        whole("octave", &[self.octave], 8)?;
        let set = [
            ("midi", self.midi.is_some()),
            ("hz", self.hz.is_some()),
            ("fade", self.fade != 0.0),
            ("count", self.count != 1),
            ("every", self.every != 0.0),
            ("vels", !self.vels.is_empty()),
            ("numeral", self.numeral.is_some()),
            ("octave", self.octave != 0),
            ("beats", self.beats != 1.0),
        ];
        let uses: &[&str] = match self.plays {
            CuePlays::Hit => &["midi", "beats"],
            CuePlays::Glide => &["hz", "fade", "beats"],
            CuePlays::Burst => &["midi", "count", "every", "vels"],
            CuePlays::Chord => &["numeral", "octave", "beats"],
        };
        if let Some((name, _)) = set.iter().find(|(name, on)| *on && !uses.contains(name)) {
            return Err(format!(
                "`{name}` does nothing for a {}; it takes {}",
                self.plays.name(),
                uses.join(", ")
            ));
        }
        match self.plays {
            CuePlays::Glide => match self.hz {
                Some(hz)
                    if hz.iter().all(|h| {
                        h.is_finite() && *h > 0.0 && (0.0..=127.0).contains(&midi_of(*h))
                    }) => {}
                _ => {
                    return Err(
                        "a glide wants `hz = [from, to]` within 8.18 to 12543 Hz, midi 0 to 127"
                            .into(),
                    );
                }
            },
            CuePlays::Burst => {
                if !(1..=MAX_COUNT).contains(&self.count) {
                    return Err(format!("a burst wants `count` from 1 to {MAX_COUNT}"));
                }
                within("every", self.every, MIN_BEATS, MAX_BEATS)?;
            }
            CuePlays::Chord => {
                let numeral = self.numeral.as_deref().ok_or("a chord wants a `numeral`")?;
                Key::new("C", "major", 3)?.chord(numeral, false)?;
            }
            CuePlays::Hit => {}
        }
        let end = self.end();
        if end > MAX_CUE_BEATS {
            return Err(format!(
                "the voice ends at beat {end}; a cue is a short stinger of at most {MAX_CUE_BEATS} beats"
            ));
        }
        Ok(())
    }

    pub fn end(&self) -> f64 {
        match self.plays {
            CuePlays::Burst => self.at + f64::from(self.count) * self.every,
            _ => self.at + self.beats,
        }
    }
}

impl CuePlays {
    pub fn name(self) -> &'static str {
        match self {
            CuePlays::Hit => "hit",
            CuePlays::Glide => "glide",
            CuePlays::Burst => "burst",
            CuePlays::Chord => "chord",
        }
    }
}

impl Style {
    pub fn role(&self, name: &str) -> Option<&Role> {
        self.roles.iter().find(|r| r.name == name)
    }

    pub fn cue(&self, name: &str) -> Option<&Cue> {
        self.cues.iter().find(|c| c.name == name)
    }

    pub fn check(&self) -> Result<(), String> {
        let [lo, hi] = self.bpm;
        if lo == 0 || lo > hi || hi > MAX_BPM {
            return Err(format!(
                "`bpm` is [{lo}, {hi}]; it wants [low, high] from 1 to {MAX_BPM}"
            ));
        }
        if !(1..=MAX_BEATS_PER_BAR).contains(&self.beats_per_bar) {
            return Err(format!("`beats_per_bar` wants 1 to {MAX_BEATS_PER_BAR}"));
        }
        within("swing", self.swing, 0.0, 1.0)?;
        within("humanize_ms", self.humanize_ms, 0.0, 100.0)?;
        weights(&self.tonics, "tonics")?;
        weights(&self.modes, "modes")?;
        for (tonic, _) in &self.tonics {
            for (mode, _) in &self.modes {
                Key::new(tonic, mode, 3).map_err(|e| format!("`tonics`/`modes`: {e}"))?;
            }
        }
        self.check_harmony()?;
        self.check_melody()?;
        self.check_roles()?;
        for (name, patch) in &self.patch {
            patch.check().map_err(|e| format!("[patch.{name}]: {e}"))?;
        }
        self.check_sections()?;
        self.check_cues()?;
        self.check_master()
    }

    fn check_roles(&self) -> Result<(), String> {
        if self.roles.len() > MAX_ROLES {
            return Err(format!(
                "the style has {} [[role]]s; it wants at most {MAX_ROLES}",
                self.roles.len()
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for role in &self.roles {
            let at = format!("[[role]] `{}`", role.name);
            if !names.insert(role.name.as_str()) {
                return Err(format!("{at}: the name is used twice"));
            }
            if !self.patch.contains_key(&role.patch) {
                return Err(format!("{at}: no [patch.{}]", role.patch));
            }
            if role.plays.patterned() && role.patterns.is_empty() && role.euclid.is_empty() {
                return Err(format!(
                    "{at}: a bass, arp or hit role wants `patterns` or `euclid`"
                ));
            }
            for pattern in &role.patterns {
                if pattern.is_empty()
                    || pattern.len() > MAX_STEPS as usize
                    || pattern.chars().any(|c| c != 'x' && c != '.')
                {
                    return Err(format!(
                        "{at}: pattern `{pattern}` wants 1 to {MAX_STEPS} steps of x and ."
                    ));
                }
            }
            for [k, n] in &role.euclid {
                if *n == 0 || k > n || *n > MAX_STEPS {
                    return Err(format!(
                        "{at}: euclid [{k}, {n}] wants 0 <= hits <= steps, steps from 1 to {MAX_STEPS}"
                    ));
                }
            }
            if let Some([a, b]) = role.range
                && (a >= b || a < 0 || b > 127)
            {
                return Err(format!(
                    "{at}: `range` [{a}, {b}] wants low below high, within 0 to 127"
                ));
            }
            let fields = || -> Result<(), String> {
                hold("hold", role.hold)?;
                whole("octave", &[role.octave], 8)?;
                whole("tones", &role.tones, 48)?;
                whole("octaves", &role.octaves, 8)?;
                whole("semis", &role.semis, 48)?;
                within("gain", f64::from(role.gain), 0.0, MAX_GAIN)?;
                within("accent", f64::from(role.accent), 0.0, 1.0)?;
                within("ramp", f64::from(role.ramp), -1.0, 1.0)
            };
            fields().map_err(|e| format!("{at}: {e}"))?;
        }
        Ok(())
    }

    fn check_sections(&self) -> Result<(), String> {
        if self.sections.is_empty() || self.sections.len() > MAX_SECTIONS {
            return Err(format!("the form wants 1 to {MAX_SECTIONS} [[section]]s"));
        }
        let fixed = self.sections.iter().all(|s| s.bars[0] == s.bars[1]);
        let mut names = std::collections::BTreeSet::new();
        let mut last_hit = None;
        for (index, section) in self.sections.iter().enumerate() {
            let at = format!("[[section]] `{}`", section.name);
            if !names.insert(section.name.as_str()) {
                return Err(format!("{at}: the name is used twice"));
            }
            let [a, b] = section.bars;
            if a > b || b == 0 || b > MAX_BARS {
                return Err(format!(
                    "{at}: `bars` [{a}, {b}] wants [low, high] with high from 1 to {MAX_BARS}"
                ));
            }
            for role in &section.roles {
                if role != "*" && self.role(role).is_none() {
                    return Err(format!("{at}: no [[role]] `{role}`"));
                }
            }
            let [t0, t1] = TEMPO_RATIO;
            let fields = || -> Result<(), String> {
                within("tempo", section.tempo, t0, t1)?;
                within("pitch", section.pitch, t0, t1)?;
                within("riser", section.riser, 0.0, MAX_BEATS)?;
                within("roll", section.roll, 0.0, MAX_BEATS)?;
                within("tail", section.tail, 0.0, MAX_BEATS)?;
                whole("lift", &[section.lift], 24)?;
                each("intensity", &section.intensity, 0.0, 1.0)?;
                within("gain_db", f64::from(section.gain_db), -60.0, 12.0)
            };
            fields().map_err(|e| format!("{at}: {e}"))?;
            if section.stutter == 1 {
                return Err(format!(
                    "{at}: `stutter = 1` drops every note; it wants 0 or 2 and up"
                ));
            }
            if section.round_bars > 0 && section.start_roles == 0 {
                return Err(format!(
                    "{at}: `round_bars` wants `start_roles` of 1 or more, or the first round is silent"
                ));
            }
            if let Some(hit) = section.hit {
                if index == 0 {
                    return Err(format!(
                        "{at}: the first section always starts at 0 s; a `hit` can't place it"
                    ));
                }
                if fixed {
                    return Err(format!(
                        "{at}: every section has fixed `bars`, so a `hit` can't place one"
                    ));
                }
                if last_hit.is_some_and(|last| hit <= last) {
                    return Err(format!(
                        "{at}: `hit = {hit}` wants a later hit than the sections before it"
                    ));
                }
                last_hit = Some(hit);
            }
            if let Some(cue) = &section.cue {
                if self.cue(cue).is_none() {
                    return Err(format!("{at}: no [[cue]] `{cue}`"));
                }
                if section.hit.is_none() {
                    return Err(format!(
                        "{at}: cue `{cue}` fires only on a hit; give the section `hit`, or let the game fire it on its event"
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_master(&self) -> Result<(), String> {
        let m = &self.master;
        let fields = || -> Result<(), String> {
            within("lufs", m.lufs, -70.0, 0.0)?;
            within("true_peak", f64::from(m.true_peak), -20.0, 0.0)?;
            each("room", &m.room, 0.0, 1.0)?;
            within("fade", m.fade, 0.0, MAX_SECONDS)?;
            if let Some(sidechain) = &m.sidechain {
                within("sidechain.depth", f64::from(sidechain.depth), 0.0, 1.0)?;
                within(
                    "sidechain.release_ms",
                    f64::from(sidechain.release_ms),
                    0.0,
                    MAX_MS,
                )?;
            }
            Ok(())
        };
        fields().map_err(|e| format!("[master]: {e}"))?;
        if let Some(sidechain) = &m.sidechain {
            for role in sidechain.keys.iter().chain(&sidechain.beds) {
                if self.role(role).is_none() {
                    return Err(format!("[master.sidechain]: no [[role]] `{role}`"));
                }
            }
        }
        Ok(())
    }

    fn check_harmony(&self) -> Result<(), String> {
        let h = &self.harmony;
        within("beats", h.beats, 0.25, MAX_BEATS).map_err(|e| format!("[harmony]: {e}"))?;
        if h.start.is_empty() {
            return Err("[harmony]: `start` needs at least one numeral".into());
        }
        let key = Key::new("C", "major", 3)?;
        let numerals = h
            .start
            .iter()
            .chain(&h.cadence)
            .chain(h.markov.keys())
            .chain(h.markov.values().flatten().map(|(n, _)| n));
        for numeral in numerals {
            key.chord(numeral, h.sevenths)
                .map_err(|e| format!("[harmony]: {e}"))?;
        }
        for (from, next) in &h.markov {
            weights(next, &format!("markov.{from}")).map_err(|e| format!("[harmony]: {e}"))?;
        }
        let reached = h
            .start
            .iter()
            .chain(h.markov.values().flatten().map(|(n, _)| n));
        for numeral in reached {
            if !h.markov.contains_key(numeral) {
                return Err(format!(
                    "[harmony]: `{numeral}` has no `markov` row, so the walk would stop on it; give it one, such as `{numeral} = [[\"{numeral}\", 1]]` to hold it"
                ));
            }
        }
        Ok(())
    }

    pub fn cue_seconds(&self, beats: f64, bpm: u32, patches: &[&str]) -> f64 {
        let tail = patches
            .iter()
            .filter_map(|p| self.patch.get(*p))
            .map(Patch::tail_seconds)
            .fold(0.0, f64::max);
        beats * 60.0 / f64::from(bpm) + tail
    }

    fn check_cues(&self) -> Result<(), String> {
        if self.cues.len() > MAX_CUES {
            return Err(format!(
                "the style has {} [[cue]]s; it wants at most {MAX_CUES}",
                self.cues.len()
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for cue in &self.cues {
            let at = format!("[[cue]] `{}`", cue.name);
            if !names.insert(cue.name.as_str()) {
                return Err(format!("{at}: the name is used twice"));
            }
            if self.role(&cue.name).is_some() {
                return Err(format!(
                    "{at}: a [[role]] has the same name; a cue is no voice of the score"
                ));
            }
            if cue.voices.is_empty() || cue.voices.len() > MAX_CUE_VOICES {
                return Err(format!(
                    "{at}: a cue wants 1 to {MAX_CUE_VOICES} [[cue.voice]]s"
                ));
            }
            for (index, voice) in cue.voices.iter().enumerate() {
                let at = format!("{at} voice {}", index + 1);
                if !self.patch.contains_key(&voice.patch) {
                    return Err(format!("{at}: no [patch.{}]", voice.patch));
                }
                voice.check().map_err(|e| format!("{at}: {e}"))?;
            }
            let beats = cue.voices.iter().map(CueVoice::end).fold(0.0, f64::max);
            let patches: Vec<&str> = cue.voices.iter().map(|v| v.patch.as_str()).collect();
            let seconds = self.cue_seconds(beats, self.bpm[0], &patches);
            if seconds > MAX_CUE_SECONDS {
                return Err(format!(
                    "{at}: lasts {seconds:.1} s at {} bpm with its patches' release; a cue is a short stinger of at most {MAX_CUE_SECONDS} s",
                    self.bpm[0]
                ));
            }
        }
        Ok(())
    }

    fn check_melody(&self) -> Result<(), String> {
        let m = &self.melody;
        let fields = || -> Result<(), String> {
            weights(&m.steps, "steps")?;
            weights(&m.rhythm, "rhythm")?;
            for (beats, _) in &m.rhythm {
                within("rhythm", *beats, MIN_BEATS, MAX_BEATS)?;
            }
            let steps: Vec<i32> = m.steps.iter().map(|(s, _)| *s).collect();
            whole("steps", &steps, 24)?;
            within("rest", m.rest, 0.0, 1.0)?;
            within("repeat", m.repeat, 0.0, 1.0)?;
            if !(1..=16).contains(&m.motif_bars) {
                return Err("`motif_bars` wants 1 to 16".into());
            }
            let [lo, hi] = m.range;
            if lo >= hi || lo < 0 || hi > 127 {
                return Err("`range` wants low below high, within 0 to 127".into());
            }
            Ok(())
        };
        fields().map_err(|e| format!("[melody]: {e}"))
    }
}
