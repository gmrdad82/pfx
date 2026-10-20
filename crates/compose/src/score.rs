use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::style::{
    MAX_BARS, MAX_BEATS_PER_BAR, MAX_BPM, MAX_CUE_BEATS, MAX_CUE_SECONDS, MAX_CUE_VOICES, MAX_CUES,
    MAX_GAIN, MAX_NOTE_BEATS, MAX_NOTES, MAX_POLYPHONY, MAX_ROLES, MAX_SECONDS, MAX_SECTIONS,
    Style, TEMPO_RATIO, within,
};

fn four() -> u32 {
    4
}

fn unit() -> f64 {
    1.0
}

fn is_four(v: &u32) -> bool {
    *v == 4
}

fn is_unit(v: &f64) -> bool {
    *v == 1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum StyleRef {
    Name(String),
    File { file: String, name: String },
}

impl StyleRef {
    pub fn name(&self) -> &str {
        match self {
            StyleRef::Name(name) | StyleRef::File { name, .. } => name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Score {
    pub style: StyleRef,
    pub seed: u64,
    pub seconds: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<f64>,
    pub bpm: u32,
    #[serde(default = "four", skip_serializing_if = "is_four")]
    pub beats_per_bar: u32,
    pub key: String,
    pub bars: u32,
    pub chords: Vec<String>,
    #[serde(rename = "section")]
    pub sections: Vec<Placed>,
    #[serde(rename = "track", default)]
    pub tracks: Vec<Track>,
    #[serde(rename = "cue", default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<ScoreCue>,
    #[serde(rename = "fire", default, skip_serializing_if = "Vec::is_empty")]
    pub fires: Vec<Fire>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreCue {
    pub name: String,
    pub event: String,
    pub beats: f64,
    #[serde(rename = "voice")]
    pub voices: Vec<CueTrack>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueTrack {
    pub patch: String,
    pub gain: f32,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fire {
    pub cue: String,
    pub at: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placed {
    pub name: String,
    pub from: u32,
    pub bars: u32,
    pub at: f64,
    #[serde(default = "unit", skip_serializing_if = "is_unit")]
    pub tempo: f64,
    pub intensity: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub role: String,
    pub notes: Vec<Note>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lowpass: Vec<Glide>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub beat: f64,
    pub midi: f64,
    pub beats: f64,
    pub vel: f64,
    pub bend: f64,
    pub fade: f64,
}

impl Note {
    pub fn new(beat: f64, midi: f64, beats: f64, vel: f64) -> Note {
        Note {
            beat,
            midi,
            beats,
            vel,
            bend: 0.0,
            fade: 0.0,
        }
    }

    pub fn row(&self) -> Vec<f64> {
        let mut row = vec![self.beat, self.midi, self.beats, self.vel];
        if self.bend != 0.0 || self.fade != 0.0 {
            row.push(self.bend);
        }
        if self.fade != 0.0 {
            row.push(self.fade);
        }
        row
    }
}

impl Serialize for Note {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let row = self.row();
        let mut seq = serializer.serialize_seq(Some(row.len()))?;
        for value in row {
            seq.serialize_element(&value)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Note {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Note, D::Error> {
        struct Row;
        impl<'de> Visitor<'de> for Row {
            type Value = Note;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a note [beat, midi, beats, velocity] with optional bend and fade")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Note, A::Error> {
                let mut values = Vec::with_capacity(6);
                while let Some(value) = seq.next_element::<f64>()? {
                    values.push(value);
                }
                if !(4..=6).contains(&values.len()) {
                    return Err(de::Error::invalid_length(values.len(), &self));
                }
                let mut note = Note::new(values[0], values[1], values[2], values[3]);
                note.bend = values.get(4).copied().unwrap_or(0.0);
                note.fade = values.get(5).copied().unwrap_or(0.0);
                Ok(note)
            }
        }
        deserializer.deserialize_seq(Row)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    pub beat: f64,
    pub hz: f64,
    pub beats: f64,
}

impl Serialize for Glide {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        [self.beat, self.hz, self.beats].serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Glide {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Glide, D::Error> {
        let [beat, hz, beats] = <[f64; 3]>::deserialize(deserializer)?;
        Ok(Glide { beat, hz, beats })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tempo {
    pub bpm: f64,
    pub marks: Vec<Mark>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub beat: f64,
    pub at: f64,
    pub ratio: f64,
}

impl Tempo {
    pub fn new(bpm: f64, beats_per_bar: u32, sections: &[Placed]) -> Tempo {
        let bpb = f64::from(beats_per_bar);
        let mut marks = Vec::with_capacity(sections.len());
        let mut at = 0.0;
        for s in sections {
            marks.push(Mark {
                beat: f64::from(s.from) * bpb,
                at,
                ratio: s.tempo,
            });
            at += f64::from(s.bars) * bpb * 60.0 / (bpm * s.tempo);
        }
        if marks.is_empty() {
            marks.push(Mark {
                beat: 0.0,
                at: 0.0,
                ratio: 1.0,
            });
        }
        Tempo { bpm, marks }
    }

    pub fn seconds(&self, beat: f64) -> f64 {
        let i = self
            .marks
            .partition_point(|m| m.beat <= beat)
            .saturating_sub(1);
        let m = self.marks[i];
        m.at + (beat - m.beat) * 60.0 / (self.bpm * m.ratio)
    }
}

fn notes_ok(what: &str, notes: &[Note], beat: f64, beats: f64) -> Result<(), String> {
    for note in notes {
        let bad = |why: &str| Err(format!("`{what}` has the note {:?}; {why}", note.row()));
        let row = [
            note.beat, note.midi, note.beats, note.vel, note.bend, note.fade,
        ];
        if row.iter().any(|v| !v.is_finite()) {
            return bad("every value must be a finite number");
        }
        if !(0.0..=beat).contains(&note.beat) {
            return bad(&format!("its beat wants 0 to {beat}"));
        }
        if !(0.0..=beats).contains(&note.beats) {
            return bad(&format!("its length wants 0 to {beats} beats"));
        }
        if !(0.0..=127.0).contains(&note.midi) || !(0.0..=127.0).contains(&(note.midi + note.bend))
        {
            return bad("its pitch and its pitch after the bend want midi 0 to 127");
        }
        if !(0.0..=MAX_GAIN).contains(&note.vel) {
            return bad(&format!("its velocity wants 0 to {MAX_GAIN}"));
        }
        if !(0.0..=MAX_CUE_SECONDS).contains(&note.fade) {
            return bad(&format!("its fade wants 0 to {MAX_CUE_SECONDS} seconds"));
        }
    }
    Ok(())
}

fn device(file: &str) -> bool {
    let stem = file
        .split('.')
        .next()
        .unwrap_or(file)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix)
            .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
    };
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
}

impl StyleRef {
    pub fn check(&self) -> Result<(), String> {
        let StyleRef::File { file, .. } = self else {
            return Ok(());
        };
        let plain = !file.is_empty()
            && !file.ends_with(['.', ' '])
            && !file.contains(['/', '\\', ':', '\0'])
            && !device(file);
        if plain {
            Ok(())
        } else {
            Err(format!(
                "`style.file` is `{file}`; it wants a file name in the document's own folder"
            ))
        }
    }
}

impl Score {
    pub fn check(&self) -> Result<(), String> {
        self.style.check()?;
        within("seconds", self.seconds, f64::MIN_POSITIVE, MAX_SECONDS)?;
        let mut last = None;
        for hit in &self.hits {
            if !hit.is_finite() || *hit < 0.0 || *hit >= self.seconds {
                return Err(format!(
                    "`hits` holds {hit}; a hit wants 0 up to the length, {} s",
                    self.seconds
                ));
            }
            if last.is_some_and(|last| hit <= last) {
                return Err("`hits` must rise".into());
            }
            last = Some(hit);
        }
        if !(1..=MAX_BPM).contains(&self.bpm) {
            return Err(format!("`bpm` is {}; it wants 1 to {MAX_BPM}", self.bpm));
        }
        if !(1..=MAX_BEATS_PER_BAR).contains(&self.beats_per_bar) {
            return Err(format!("`beats_per_bar` wants 1 to {MAX_BEATS_PER_BAR}"));
        }
        if self.sections.is_empty() || self.sections.len() > MAX_SECTIONS {
            return Err(format!("the score wants 1 to {MAX_SECTIONS} [[section]]s"));
        }
        for (what, count, limit) in [
            ("[[track]]s", self.tracks.len(), MAX_ROLES),
            ("[[cue]]s", self.cues.len(), MAX_CUES),
            ("[[fire]]s", self.fires.len(), MAX_SECTIONS),
        ] {
            if count > limit {
                return Err(format!(
                    "the score holds {count} {what}; it wants at most {limit}"
                ));
            }
        }
        if let Some(cue) = self.cues.iter().find(|c| c.voices.len() > MAX_CUE_VOICES) {
            return Err(format!(
                "[[cue]] `{}` holds {} voices; it wants at most {MAX_CUE_VOICES}",
                cue.name,
                cue.voices.len()
            ));
        }
        let notes = self.notes()
            + self
                .cues
                .iter()
                .flat_map(|c| &c.voices)
                .map(|v| v.notes.len())
                .sum::<usize>();
        if notes > MAX_NOTES {
            return Err(format!(
                "the score holds {notes} notes; it wants at most {MAX_NOTES}"
            ));
        }
        if self.chords.len() > MAX_NOTES {
            return Err(format!(
                "the score holds {} chords; it wants at most {MAX_NOTES}",
                self.chords.len()
            ));
        }
        let [t0, t1] = TEMPO_RATIO;
        let mut bar = 0u32;
        for s in &self.sections {
            let at = format!("[[section]] `{}`", s.name);
            within("tempo", s.tempo, t0, t1).map_err(|e| format!("{at}: {e}"))?;
            for i in s.intensity {
                within("intensity", f64::from(i), 0.0, 1.0).map_err(|e| format!("{at}: {e}"))?;
            }
            if s.from != bar {
                return Err(format!(
                    "{at}: `from = {}` leaves a gap or an overlap; the sections before it end at bar {bar}",
                    s.from
                ));
            }
            bar = bar
                .checked_add(s.bars)
                .filter(|b| *b <= MAX_BARS)
                .ok_or_else(|| format!("{at}: the form runs past {MAX_BARS} bars"))?;
        }
        if bar != self.bars {
            return Err(format!(
                "`bars = {}` but the sections add up to {bar}",
                self.bars
            ));
        }
        let tempo = self.tempo();
        let bpb = f64::from(self.beats_per_bar);
        for s in &self.sections {
            let start = tempo.seconds(f64::from(s.from) * bpb);
            if !s.at.is_finite() || (s.at - start).abs() > 1e-3 {
                return Err(format!(
                    "[[section]] `{}`: `at = {}` but the tempo map starts it at {start:.6} s; `at` follows `from`, `bars` and `tempo`",
                    s.name, s.at
                ));
            }
        }
        let total = f64::from(self.bars) * bpb;
        let mut roles = std::collections::BTreeSet::new();
        for track in &self.tracks {
            let at = format!("[[track]] `{}`", track.role);
            if !roles.insert(track.role.as_str()) {
                return Err(format!("{at}: the role has two tracks"));
            }
            notes_ok("notes", &track.notes, total + 1.0, MAX_NOTE_BEATS)
                .map_err(|e| format!("{at}: {e}"))?;
            for glide in &track.lowpass {
                within("lowpass", glide.hz, 1.0, 24_000.0)
                    .and_then(|_| within("lowpass", glide.beat, 0.0, total))
                    .and_then(|_| within("lowpass", glide.beats, 0.0, total))
                    .map_err(|e| format!("{at}: {e}"))?;
            }
        }
        let mut cues = std::collections::BTreeSet::new();
        for cue in &self.cues {
            let at = format!("[[cue]] `{}`", cue.name);
            if !cues.insert(cue.name.as_str()) {
                return Err(format!("{at}: the name is used twice"));
            }
            within("beats", cue.beats, 0.0, MAX_CUE_BEATS).map_err(|e| format!("{at}: {e}"))?;
            let mut end = cue.beats;
            for voice in &cue.voices {
                within("gain", f64::from(voice.gain), 0.0, MAX_GAIN)
                    .map_err(|e| format!("{at}: {e}"))?;
                notes_ok("notes", &voice.notes, MAX_CUE_BEATS, MAX_CUE_BEATS)
                    .map_err(|e| format!("{at}: {e}"))?;
                for note in &voice.notes {
                    end = end.max(note.beat + note.beats);
                }
            }
            if end > MAX_CUE_BEATS {
                return Err(format!(
                    "{at}: a note ends at beat {end}; a cue is a short stinger of at most {MAX_CUE_BEATS} beats"
                ));
            }
            let seconds = end * 60.0 / f64::from(self.bpm);
            if seconds > MAX_CUE_SECONDS {
                return Err(format!(
                    "{at}: lasts {seconds:.1} s at {} bpm; a cue is a short stinger of at most {MAX_CUE_SECONDS} s",
                    self.bpm
                ));
            }
        }
        for fire in &self.fires {
            if self.cue(&fire.cue).is_none() {
                return Err(format!("[[fire]]: no [[cue]] `{}`", fire.cue));
            }
            within("at", fire.at, 0.0, self.seconds).map_err(|e| format!("[[fire]]: {e}"))?;
        }
        Ok(())
    }

    pub fn check_style(&self, style: &Style) -> Result<(), String> {
        let sound = |patch: &str| {
            style
                .patch
                .get(patch)
                .map(|p| (p.osc.len() as f64, p.tail_seconds()))
        };
        let tempo = self.tempo();
        let mut work = 0.0;
        for track in &self.tracks {
            let at = format!("[[track]] `{}`", track.role);
            let role = style
                .role(&track.role)
                .ok_or_else(|| format!("{at}: the style has no [[role]] of that name"))?;
            let (oscs, tail) = sound(&role.patch)
                .ok_or_else(|| format!("{at}: the style has no [patch.{}]", role.patch))?;
            for note in &track.notes {
                let start = tempo.seconds(note.beat);
                if start < self.seconds {
                    let end = (tempo.seconds(note.beat + note.beats) + tail).min(self.seconds);
                    work += oscs * (end - start).max(0.0);
                }
            }
        }
        if work > MAX_POLYPHONY * self.seconds {
            return Err(format!(
                "the score keeps {:.0} oscillators sounding on average; a render wants at most {MAX_POLYPHONY}, so shorten the notes or the patches' release, or use fewer oscillators",
                work / self.seconds
            ));
        }
        let beat = 60.0 / f64::from(self.bpm);
        for cue in &self.cues {
            let at = format!("[[cue]] `{}`", cue.name);
            let mut work = 0.0;
            let mut end = cue.beats;
            for voice in &cue.voices {
                let (oscs, tail) = sound(&voice.patch)
                    .ok_or_else(|| format!("{at}: the style has no [patch.{}]", voice.patch))?;
                for note in &voice.notes {
                    work += oscs * (note.beats * beat + tail);
                    end = end.max(note.beat + note.beats);
                }
            }
            let patches: Vec<&str> = cue.voices.iter().map(|v| v.patch.as_str()).collect();
            let seconds = style.cue_seconds(end, self.bpm, &patches);
            if seconds > MAX_CUE_SECONDS {
                return Err(format!(
                    "{at}: lasts {seconds:.1} s at {} bpm with its patches' release; a cue is a short stinger of at most {MAX_CUE_SECONDS} s",
                    self.bpm
                ));
            }
            if work > MAX_POLYPHONY * MAX_CUE_SECONDS {
                return Err(format!(
                    "{at}: sounds {work:.0} oscillator-seconds; a cue wants at most {}",
                    MAX_POLYPHONY * MAX_CUE_SECONDS
                ));
            }
        }
        Ok(())
    }

    pub fn tempo(&self) -> Tempo {
        Tempo::new(f64::from(self.bpm), self.beats_per_bar, &self.sections)
    }

    pub fn cue(&self, name: &str) -> Option<&ScoreCue> {
        self.cues.iter().find(|c| c.name == name)
    }

    pub fn track(&self, role: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.role == role)
    }

    pub fn notes(&self) -> usize {
        self.tracks.iter().map(|t| t.notes.len()).sum()
    }
}
