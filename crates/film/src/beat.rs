use crate::script::{Kinds, MAX_SECONDS, Scene, cuts};
use pfx_core::rate::Rate;
use serde::Deserialize;
use toml::{Table, Value};

pub const LEAST_BPM: f64 = 10.0;
pub const MOST_BPM: f64 = 1000.0;
pub const MOST_PER_BAR: u32 = 32;
pub const MOST_SNAP: u32 = 64;
pub const MOST_CHANGES: usize = 256;
const EARLY: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub beat: f64,
    pub seconds: f64,
    pub bpm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub signature: [u32; 2],
    pub offset: f64,
    pub snap: Option<u32>,
    pub strict: bool,
    pub tempo: Vec<Change>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoPart {
    pub at: Value,
    pub bpm: f64,
}

fn bpm_ok(bpm: f64) -> Result<(), String> {
    if bpm.is_finite() && (LEAST_BPM..=MOST_BPM).contains(&bpm) {
        Ok(())
    } else {
        Err(format!("[film] bpm is in [{LEAST_BPM}, {MOST_BPM}]"))
    }
}

impl Grid {
    pub fn new(bpm: f64, signature: [u32; 2], offset: f64) -> Result<Grid, String> {
        bpm_ok(bpm)?;
        if !(1..=MOST_PER_BAR).contains(&signature[0])
            || !(signature[1].is_power_of_two() && signature[1] <= 32)
        {
            return Err(format!(
                "[film] signature is [beats, note]: beats in [1, {MOST_PER_BAR}], the note 1, 2, 4, 8, 16 or 32"
            ));
        }
        if !(offset.is_finite() && (0.0..MAX_SECONDS).contains(&offset)) {
            return Err(format!(
                "[film] offset is the seconds to beat 1, in [0, {MAX_SECONDS})"
            ));
        }
        Ok(Grid {
            signature,
            offset,
            snap: Some(4),
            strict: false,
            tempo: vec![Change {
                beat: 0.0,
                seconds: offset,
                bpm,
            }],
        })
    }

    pub fn beats_per_bar(&self) -> u32 {
        self.signature[0]
    }

    pub fn bpm(&self) -> f64 {
        self.tempo[0].bpm
    }

    fn by_beat(&self, beat: f64) -> usize {
        self.tempo.iter().rposition(|c| c.beat <= beat).unwrap_or(0)
    }

    fn by_second(&self, seconds: f64) -> usize {
        self.tempo
            .iter()
            .rposition(|c| c.seconds <= seconds)
            .unwrap_or(0)
    }

    pub fn seconds(&self, beat: f64) -> f64 {
        let c = self.tempo[self.by_beat(beat)];
        c.seconds + (beat - c.beat) * 60.0 / c.bpm
    }

    pub fn beat(&self, seconds: f64) -> f64 {
        let c = self.tempo[self.by_second(seconds)];
        c.beat + (seconds - c.seconds) * c.bpm / 60.0
    }

    pub fn span(&self, from: f64, beats: f64) -> f64 {
        let i = self.by_second(from);
        let end = self.beat(from) + beats;
        match self.tempo.get(i + 1) {
            Some(next) if end > next.beat => self.seconds(end) - from,
            _ => beats * 60.0 / self.tempo[i].bpm,
        }
    }

    pub fn change(&mut self, part: &TempoPart) -> Result<(), String> {
        bpm_ok(part.bpm).map_err(|e| e.replace("[film] bpm", "[[film.tempo]] bpm"))?;
        let (beat, seconds) = match &part.at {
            Value::String(text) => {
                let beat = count(text, self.beats_per_bar())
                    .map_err(|e| format!("[[film.tempo]] at: {e}"))?;
                (beat, self.seconds(beat))
            }
            Value::Integer(n) => (self.beat(*n as f64), *n as f64),
            Value::Float(s) if s.is_finite() => (self.beat(*s), *s),
            _ => return Err("[[film.tempo]] at is film seconds or a place like \"8bar\"".into()),
        };
        let last = self.tempo[self.tempo.len() - 1];
        if !(beat > last.beat + EARLY && seconds < MAX_SECONDS * 10.0) {
            return Err(format!(
                "[[film.tempo]] at {} comes after beat 1 and the change before it",
                shown(beat)
            ));
        }
        if self.tempo.len() >= MOST_CHANGES {
            return Err(format!("[film] at most {MOST_CHANGES} tempo changes"));
        }
        self.tempo.push(Change {
            beat,
            seconds,
            bpm: part.bpm,
        });
        Ok(())
    }

    pub fn near(&self, seconds: f64, d: u32) -> f64 {
        let b = self.beat(seconds) * f64::from(d);
        self.seconds(b.round() / f64::from(d))
    }
}

pub fn signature(text: &[u32]) -> Result<[u32; 2], String> {
    match text {
        [n, d] => Ok([*n, *d]),
        _ => Err("[film] signature is [beats, note], like [4, 4]".into()),
    }
}

pub fn snap(text: &str) -> Result<Option<u32>, String> {
    let bad = || {
        format!("[film] snap is \"1\", \"1/2\", \"1/4\" … \"1/{MOST_SNAP}\" of a beat, or \"off\"")
    };
    match text.trim() {
        "off" => Ok(None),
        "1" => Ok(Some(1)),
        t => {
            let d: u32 = t
                .strip_prefix("1/")
                .and_then(|d| d.trim().parse().ok())
                .ok_or_else(bad)?;
            if (1..=MOST_SNAP).contains(&d) {
                Ok(Some(d))
            } else {
                Err(bad())
            }
        }
    }
}

pub fn count(text: &str, per_bar: u32) -> Result<f64, String> {
    let bad = || {
        format!(
            "'{text}' is a time in beats and bars, like \"8b\", \"2bar\", \"2bar+1b\" or \"1.5b\""
        )
    };
    let mut beats = 0.0;
    for term in text.split('+') {
        let term = term.trim();
        let i = term
            .find(|c: char| c.is_ascii_alphabetic())
            .ok_or_else(bad)?;
        let (n, unit) = term.split_at(i);
        let n: f64 = n.trim().parse().map_err(|_| bad())?;
        if !(n.is_finite() && n >= 0.0) {
            return Err(bad());
        }
        beats += match unit {
            "b" | "beat" | "beats" => n,
            "bar" | "bars" => n * f64::from(per_bar),
            _ => return Err(bad()),
        };
    }
    Ok(beats)
}

pub fn convert(
    table: &mut Table,
    keys: &[&str],
    grid: Option<&Grid>,
    from: f64,
) -> Result<(), String> {
    for k in keys {
        let Some(Value::String(text)) = table.get(*k) else {
            continue;
        };
        let text = text.clone();
        let Some(g) = grid else {
            return Err(format!(
                "{k} = \"{text}\" is in beats, which needs [film] bpm"
            ));
        };
        let beats = count(&text, g.beats_per_bar()).map_err(|e| format!("{k}: {e}"))?;
        table.insert((*k).to_string(), Value::Float(g.span(from, beats)));
    }
    Ok(())
}

fn number(table: &Table, key: &str) -> f64 {
    match table.get(key) {
        Some(Value::Float(v)) => *v,
        Some(Value::Integer(v)) => *v as f64,
        _ => 0.0,
    }
}

pub const SCENE: [&str; 2] = ["seconds", "from"];
pub const STYLE: [&str; 2] = ["fade", "out"];
const PLACES: [&str; 2] = ["at", "until"];
pub const LISTS: [(&str, &[&str]); 7] = [
    ("key", &[]),
    ("text", &["fade", "out"]),
    ("art", &["fade", "out", "from"]),
    ("sound", &["every", "seconds"]),
    ("flash", &["seconds"]),
    ("shake", &[]),
    ("input", &[]),
];

pub fn scene(table: &mut Table, grid: Option<&Grid>, start: f64) -> Result<f64, String> {
    convert(table, &SCENE, grid, start)?;
    if let Some(Value::Table(t)) = table.get_mut("transition") {
        convert(t, &["seconds"], grid, start).map_err(|e| format!("[scene.transition] {e}"))?;
    }
    for (list, lengths) in LISTS {
        let Some(Value::Array(items)) = table.get_mut(list) else {
            continue;
        };
        for item in items {
            if let Value::Table(t) = item {
                let at = |e: String| format!("a {list}'s {e}");
                convert(t, &PLACES, grid, start).map_err(at)?;
                let from = start + number(t, "at");
                convert(t, lengths, grid, from).map_err(at)?;
            }
        }
    }
    Ok(number(table, "seconds"))
}

pub fn shown(beats: f64) -> String {
    format!("{}b", (beats * 1e4).round() / 1e4)
}

fn unit(d: u32) -> String {
    if d == 1 {
        "beat".into()
    } else {
        format!("1/{d} beat")
    }
}

fn grid_name(d: u32) -> String {
    if d == 1 {
        "beat grid".into()
    } else {
        format!("1/{d}-beat grid")
    }
}

pub fn misses<K: Kinds>(grid: &Grid, scenes: &[Scene<K>], rate: Rate) -> Vec<String> {
    let mut starts = Vec::with_capacity(scenes.len() + 1);
    let mut start = 0.0;
    starts.push(start);
    for s in scenes {
        start += s.seconds;
        starts.push(start);
    }
    let cuts = cuts(&starts, rate);
    let fps = rate.fps();
    let frame = rate.step() + EARLY;
    let half = 0.5 * rate.step() + EARLY;
    let lead = |t: f64| t + EARLY < grid.offset;
    let off = |t: f64, d: u32, most: f64| {
        let by = (t - grid.near(t, d)).abs();
        (by > most).then_some(by * fps)
    };
    let mut out = Vec::new();
    for (i, s) in scenes.iter().enumerate() {
        let at = |why: String| format!("scene '{}': {why}", s.name);
        let start = starts[i];
        let cut = rate.time(cuts[i]);
        if !lead(start) {
            let end = start + s.seconds;
            let (b0, b1) = (grid.beat(start), grid.beat(end));
            let d = f64::from(grid.snap.unwrap_or(1));
            let whole = grid.seconds(b0 + ((b1 - b0) * d).round() / d);
            if (whole - end).abs() > frame {
                out.push(at(format!(
                    "its length, {}, isn't a whole number of {}s",
                    shown(b1 - b0),
                    unit(grid.snap.unwrap_or(1))
                )));
            }
            if let Some(d) = grid.snap
                && let Some(by) = off(cut, d, half)
            {
                out.push(at(format!(
                    "the cut into it lands {} from beat 1, off the {} by {by:.1} frames",
                    shown(grid.beat(cut)),
                    grid_name(d)
                )));
            }
            if let Some(d) = grid.snap
                && let Some(t) = &s.transition
            {
                let end = rate.time(rate.frames(start + t.seconds).round() as u64);
                if let Some(by) = off(end, d, half) {
                    out.push(at(format!(
                        "the {} into it ends {} from beat 1, off the {} by {by:.1} frames",
                        t.name(),
                        shown(grid.beat(end)),
                        grid_name(d)
                    )));
                }
            }
        }
        if let Some(d) = grid.snap {
            let picture = s
                .key
                .iter()
                .map(|k| ("a key", k.at))
                .chain(s.flash.iter().map(|f| ("a flash", f.at)))
                .chain(s.shake.iter().map(|k| ("a shake", k.at)))
                .chain(s.text.iter().map(|t| ("a text", t.at)));
            let sound = s.sound.iter().map(|c| ("a sound", c.at));
            for (what, t) in picture.chain(sound) {
                let film = start + t;
                if lead(film) {
                    continue;
                }
                if let Some(by) = off(film, d, frame) {
                    out.push(at(format!(
                        "{what} at {} is off the {} by {by:.1} frames ({} from beat 1)",
                        shown(grid.beat(film) - grid.beat(start)),
                        grid_name(d),
                        shown(grid.beat(film))
                    )));
                }
            }
        }
    }
    out
}
