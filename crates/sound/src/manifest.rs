use std::collections::BTreeSet;
use std::path::Path;

use toml::{Table, Value};

use crate::adaptive::{AdaptiveTrack, Ease, Next, Section, Stem, StemGain};
use crate::clip::Clip;

const MUSIC_KEYS: &[&str] = &[
    "bpm",
    "beats_per_bar",
    "offset",
    "start",
    "intensity",
    "ease",
    "ease_power",
    "transition",
    "stem",
    "section",
    "stinger",
];
const STEM_KEYS: &[&str] = &["name", "file", "gain", "shape"];
const SECTION_KEYS: &[&str] = &["name", "bars", "next", "avoid_repeat"];
const STINGER_KEYS: &[&str] = &["name", "event", "file"];

#[derive(Clone, Debug, PartialEq)]
pub struct StemFile {
    pub name: String,
    pub file: String,
    pub gain: StemGain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StingerFile {
    pub name: String,
    pub event: String,
    pub file: String,
}

#[derive(Clone, Debug)]
pub struct Stinger {
    pub name: String,
    pub event: String,
    pub clip: Clip,
}

#[derive(Clone, Debug)]
pub struct Music {
    pub track: AdaptiveTrack,
    pub stingers: Vec<Stinger>,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub stems: Vec<StemFile>,
    pub stingers: Vec<StingerFile>,
    pub track: AdaptiveTrack,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Option<Manifest>, String> {
        let document: Table = toml::from_str(text).map_err(|e| e.to_string())?;
        match document.get("music") {
            None => Ok(None),
            Some(Value::Table(music)) => Manifest::from_table(music).map(Some),
            Some(_) => Err("`music` must be a table".into()),
        }
    }

    pub fn from_table(music: &Table) -> Result<Manifest, String> {
        let at = "[music]";
        known(music, MUSIC_KEYS, at)?;
        let bpm = number(music, "bpm", at)?.ok_or("[music]: `bpm` is required")?;
        let beats_per_bar = whole(music, "beats_per_bar", at)?.unwrap_or(4);
        let mut track = AdaptiveTrack::new(Vec::new(), bpm, beats_per_bar);
        track.offset = number(music, "offset", at)?.unwrap_or(0.0);
        track.start = text(music, "start", at)?;
        track.intensity = number(music, "intensity", at)?.unwrap_or(0.0) as f32;
        track.transition = number(music, "transition", at)?.unwrap_or(0.0) as f32;
        let power = whole(music, "ease_power", at)?.unwrap_or(2).clamp(1, 16) as i32;
        track.ease = match text(music, "ease", at)?.as_deref() {
            None | Some("linear") => Ease::Linear,
            Some("in") => Ease::In(power),
            Some("out") => Ease::Out(power),
            Some("in_out") => Ease::InOut(power),
            Some(other) => {
                return Err(format!(
                    "[music]: `ease` is `{other}`, not linear, in, out or in_out"
                ));
            }
        };
        let mut stems = Vec::new();
        for (index, table) in tables(music, "stem", at)?.into_iter().enumerate() {
            stems.push(stem_file(table, &format!("[[music.stem]] {}", index + 1))?);
        }
        if stems.is_empty() {
            return Err("[music]: at least one [[music.stem]] is required".into());
        }
        let mut stingers = Vec::new();
        for (index, table) in tables(music, "stinger", at)?.into_iter().enumerate() {
            stingers.push(stinger_file(
                table,
                &format!("[[music.stinger]] {}", index + 1),
            )?);
        }
        for (index, table) in tables(music, "section", at)?.into_iter().enumerate() {
            track
                .sections
                .push(section(table, &format!("[[music.section]] {}", index + 1))?);
        }
        Ok(Manifest {
            stems,
            stingers,
            track,
        })
    }

    pub fn track(
        &self,
        mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<AdaptiveTrack, String> {
        let mut track = self.track.clone();
        for stem in &self.stems {
            let bytes = read(&stem.file).map_err(|e| format!("stem `{}`: {e}", stem.name))?;
            let clip = Clip::from_wav(&bytes).ok_or_else(|| {
                format!("stem `{}`: `{}` is no WAV pfx reads", stem.name, stem.file)
            })?;
            track.stems.push(Stem {
                name: stem.name.clone(),
                clip,
                gain: stem.gain.clone(),
            });
        }
        Ok(track)
    }
}

pub fn load(path: &Path) -> Result<AdaptiveTrack, String> {
    load_music(path).map(|music| music.track)
}

pub fn load_music(path: &Path) -> Result<Music, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new(""));
    music_bytes(&bytes, Some(dir)).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_bytes(bytes: &[u8], beside: Option<&Path>) -> Result<AdaptiveTrack, String> {
    music_bytes(bytes, beside).map(|music| music.track)
}

pub fn music_bytes(bytes: &[u8], beside: Option<&Path>) -> Result<Music, String> {
    let reader = pfx_file::Reader::from_bytes(bytes).map_err(|e| e.to_string())?;
    let mut manifest = None;
    let mut texts = reader.chunks(pfx_file::SCNE).map_err(|e| e.to_string())?;
    texts.extend(reader.chunks(pfx_file::RCPE).map_err(|e| e.to_string())?);
    for (name, chunk) in texts {
        let text = std::str::from_utf8(&chunk).map_err(|_| format!("`{name}` is not UTF-8"))?;
        if let Some(found) = Manifest::parse(text).map_err(|e| format!("`{name}`: {e}"))? {
            manifest = Some(found);
            break;
        }
    }
    let manifest = manifest.ok_or("no scene in the file has a [music] table")?;
    let read = |file: &str| -> Result<Vec<u8>, String> {
        if let Some(chunk) = reader
            .named(pfx_file::FILE, file)
            .map_err(|e| e.to_string())?
        {
            return Ok(chunk.into_owned());
        }
        let Some(dir) = beside else {
            return Err(format!("`{file}` is not in the file"));
        };
        let relative = Path::new(file);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(format!("`{file}` must stay inside the file's folder"));
        }
        std::fs::read(dir.join(relative)).map_err(|e| format!("`{file}`: {e}"))
    };
    let track = manifest.track(&read)?;
    let mut stingers = Vec::new();
    for stinger in &manifest.stingers {
        let bytes = read(&stinger.file).map_err(|e| format!("stinger `{}`: {e}", stinger.name))?;
        let clip = Clip::from_wav(&bytes).ok_or_else(|| {
            format!(
                "stinger `{}`: `{}` is no WAV pfx reads",
                stinger.name, stinger.file
            )
        })?;
        stingers.push(Stinger {
            name: stinger.name.clone(),
            event: stinger.event.clone(),
            clip,
        });
    }
    Ok(Music { track, stingers })
}

fn stinger_file(table: &Table, at: &str) -> Result<StingerFile, String> {
    known(table, STINGER_KEYS, at)?;
    let name = text(table, "name", at)?.ok_or_else(|| format!("{at}: `name` is required"))?;
    let file = text(table, "file", at)?.ok_or_else(|| format!("{at}: `file` is required"))?;
    let event = text(table, "event", at)?.unwrap_or_else(|| name.clone());
    Ok(StingerFile { name, event, file })
}

fn stem_file(table: &Table, at: &str) -> Result<StemFile, String> {
    known(table, STEM_KEYS, at)?;
    let name = text(table, "name", at)?.ok_or_else(|| format!("{at}: `name` is required"))?;
    let file = text(table, "file", at)?.ok_or_else(|| format!("{at}: `file` is required"))?;
    let power = match text(table, "shape", at)?.as_deref() {
        None | Some("power") => true,
        Some("linear") => false,
        Some(other) => {
            return Err(format!("{at}: `shape` is `{other}`, not power or linear"));
        }
    };
    let keys = match table.get("gain") {
        None => vec![(0.0, 1.0)],
        Some(value) if as_number(value).is_some() => {
            vec![(0.0, as_number(value).unwrap_or(1.0) as f32)]
        }
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.as_array().map(Vec::as_slice) {
                Some([a, b]) => match (as_number(a), as_number(b)) {
                    (Some(a), Some(b)) => Ok((a as f32, b as f32)),
                    _ => Err(()),
                },
                _ => Err(()),
            })
            .collect::<Result<_, _>>()
            .map_err(|_| format!("{at}: `gain` keys are [intensity, gain] pairs"))?,
        Some(_) => {
            return Err(format!(
                "{at}: `gain` must be a number or [[intensity, gain], ...]"
            ));
        }
    };
    Ok(StemFile {
        name,
        file,
        gain: StemGain { keys, power },
    })
}

fn section(table: &Table, at: &str) -> Result<Section, String> {
    known(table, SECTION_KEYS, at)?;
    let name = text(table, "name", at)?.ok_or_else(|| format!("{at}: `name` is required"))?;
    if name == "stop" {
        return Err(format!("{at}: `stop` is kept for `next`"));
    }
    let bar = |value: &Value| value.as_integer().and_then(|n| u32::try_from(n).ok());
    let pair = |value: &Value| match value.as_array().map(Vec::as_slice) {
        Some([a, b]) => bar(a).zip(bar(b)),
        _ => None,
    };
    let want = || format!("{at}: `bars` is [from, to] or [[from, to], ...] in whole bars");
    let variants = match table.get("bars") {
        None => return Err(format!("{at}: `bars` is required")),
        Some(value) => match pair(value) {
            Some(one) => vec![one],
            None => value
                .as_array()
                .ok_or_else(want)?
                .iter()
                .map(|item| pair(item).ok_or_else(want))
                .collect::<Result<_, _>>()?,
        },
    };
    let next = match text(table, "next", at)?.as_deref() {
        None => Next::Repeat,
        Some("stop") => Next::Stop,
        Some(other) => Next::Section(other.to_string()),
    };
    let avoid_repeat = match table.get("avoid_repeat") {
        None => false,
        Some(Value::Boolean(flag)) => *flag,
        Some(_) => return Err(format!("{at}: `avoid_repeat` must be true or false")),
    };
    Ok(Section {
        name,
        variants,
        next,
        avoid_repeat,
    })
}

fn known(table: &Table, keys: &[&str], at: &str) -> Result<(), String> {
    let allowed: BTreeSet<&str> = keys.iter().copied().collect();
    match table.keys().find(|key| !allowed.contains(key.as_str())) {
        Some(key) => Err(format!(
            "{at}: unknown key `{key}` (known: {})",
            keys.join(", ")
        )),
        None => Ok(()),
    }
}

fn tables<'a>(table: &'a Table, key: &str, at: &str) -> Result<Vec<&'a Table>, String> {
    match table.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_table()
                    .ok_or_else(|| format!("{at}: `{key}` must be [[music.{key}]] tables"))
            })
            .collect(),
        Some(_) => Err(format!("{at}: `{key}` must be [[music.{key}]] tables")),
    }
}

fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Float(f) => Some(*f),
        Value::Integer(n) => Some(*n as f64),
        _ => None,
    }
}

fn number(table: &Table, key: &str, at: &str) -> Result<Option<f64>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => as_number(value)
            .map(Some)
            .ok_or_else(|| format!("{at}: `{key}` must be a number")),
    }
}

fn whole(table: &Table, key: &str, at: &str) -> Result<Option<u32>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(Value::Integer(n)) if *n >= 1 => u32::try_from(*n)
            .map(Some)
            .map_err(|_| format!("{at}: `{key}` is too large")),
        Some(_) => Err(format!("{at}: `{key}` must be a whole number of 1 or more")),
    }
}

fn text(table: &Table, key: &str, at: &str) -> Result<Option<String>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{at}: `{key}` must be a string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptive::Quantize;
    use crate::mix::{Level, Mixer};

    const SCENE: &str = r#"
[music]
bpm = 600
beats_per_bar = 4
start = "calm"
ease = "in_out"
transition = 0.0

[[music.stem]]
name = "pads"
file = "music/pads.wav"

[[music.stem]]
name = "drums"
file = "music/drums.wav"
gain = [[0.3, 0.0], [0.4, 1.0]]

[[music.section]]
name = "calm"
bars = [[0, 1], [1, 2]]
avoid_repeat = true

[[music.section]]
name = "fight"
bars = [2, 4]
next = "calm"
"#;

    fn wav(value: f32) -> Vec<u8> {
        Clip {
            rate: 1000,
            channels: 1,
            samples: vec![value; 1600],
        }
        .to_wav()
    }

    fn document(scene: &str) -> Vec<u8> {
        pfx_file::Writer::new()
            .named(
                pfx_file::SCNE,
                "main.toml",
                scene.as_bytes(),
                pfx_file::Codec::Auto,
            )
            .named(
                pfx_file::FILE,
                "music/pads.wav",
                wav(0.25),
                pfx_file::Codec::Auto,
            )
            .named(
                pfx_file::FILE,
                "music/drums.wav",
                wav(0.5),
                pfx_file::Codec::Auto,
            )
            .to_vec()
            .unwrap()
    }

    #[test]
    fn a_music_table_in_a_document_loads_and_plays() {
        let track = load_bytes(&document(SCENE), None).unwrap();
        assert_eq!(track.stems.len(), 2);
        assert_eq!(track.sections.len(), 2);
        assert_eq!(track.sections[0].variants, vec![(0, 1), (1, 2)]);
        assert_eq!(track.sections[1].next, Next::Section("calm".into()));
        assert_eq!(track.ease, Ease::InOut(2));
        assert!(track.stems[1].gain.power);
        let mut mixer = Mixer::new(1000, 1);
        mixer.play_adaptive(&track.prepare(1000).unwrap(), 0.0, Level::default());
        assert_eq!(mixer.render(10)[9], 0.25);
        mixer.set_music_intensity(1.0, 0.0);
        assert!(mixer.request_music_section("fight", Quantize::Bar));
        let out = mixer.render(500);
        assert_eq!(out[390], 0.75);
        assert_eq!(mixer.music_section(), Some("fight"));
    }

    #[test]
    fn a_wrong_music_table_names_its_table_and_key() {
        let cases = [
            ("[music]\nbeats_per_bar = 4", "`bpm` is required"),
            ("[music]\nbpm = 1\ntempo = 2", "unknown key `tempo`"),
            ("[music]\nbpm = 1", "at least one [[music.stem]]"),
            (
                "[music]\nbpm = 1\n[[music.stem]]\nname = \"a\"\nfile = \"a.wav\"\ngain = [1]",
                "[[music.stem]] 1: `gain` keys",
            ),
            (
                "[music]\nbpm = 1\n[[music.stem]]\nname = \"a\"\nfile = \"a.wav\"\n[[music.section]]\nname = \"x\"\nbars = [1.5, 2]",
                "[[music.section]] 1: `bars`",
            ),
        ];
        for (text, want) in cases {
            let got = match Manifest::parse(text) {
                Ok(None) => "no music".to_string(),
                Ok(Some(_)) => "ok".to_string(),
                Err(e) => e,
            };
            assert!(got.contains(want), "{text}: {got}");
        }
        assert!(Manifest::parse("bpm = 600").unwrap().is_none());
        let missing = SCENE.replace("music/drums.wav", "music/gone.wav");
        let err = load_bytes(&document(&missing), None).unwrap_err();
        assert!(
            err.contains("stem `drums`") && err.contains("gone.wav"),
            "{err}"
        );
        let escape = SCENE.replace("music/drums.wav", "../drums.wav");
        let err = load_bytes(&document(&escape), Some(Path::new("."))).unwrap_err();
        assert!(err.contains("inside the file's folder"), "{err}");
    }
}
