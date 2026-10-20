use toml::{Table, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub at: f64,
    pub intensity: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sting {
    pub name: String,
    pub event: String,
    pub file: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub name: String,
    pub file: String,
    pub plays: Vec<bool>,
}

pub fn table(
    bpm: f64,
    beats_per_bar: u32,
    seconds: f64,
    parts: &[Part],
    lines: &[Line],
    stings: &[Sting],
) -> Table {
    let bar = 60.0 / bpm * f64::from(beats_per_bar);
    let bar_of = |at: f64| (at / bar).round() as i64;
    let mut music = Table::new();
    music.insert("bpm".into(), number(bpm));
    music.insert(
        "beats_per_bar".into(),
        Value::Integer(i64::from(beats_per_bar)),
    );
    if let Some(first) = parts.first() {
        music.insert("start".into(), Value::String(first.name.clone()));
    }
    let stems: Vec<Value> = lines
        .iter()
        .map(|line| {
            let mut stem = Table::new();
            stem.insert("name".into(), Value::String(line.name.clone()));
            stem.insert("file".into(), Value::String(line.file.clone()));
            if let Some(keys) = gain(parts, &line.plays) {
                stem.insert("gain".into(), keys);
            }
            Value::Table(stem)
        })
        .collect();
    music.insert("stem".into(), Value::Array(stems));
    let mut names: Vec<&str> = Vec::new();
    for part in parts {
        if !names.contains(&part.name.as_str()) {
            names.push(&part.name);
        }
    }
    let sections: Vec<Value> = names
        .iter()
        .filter_map(|name| {
            let mut variants = Vec::new();
            for (i, part) in parts.iter().enumerate() {
                if part.name != *name {
                    continue;
                }
                let end = parts.get(i + 1).map_or(seconds, |next| next.at);
                let (from, to) = (bar_of(part.at), bar_of(end));
                if to > from {
                    variants.push(Value::Array(vec![Value::Integer(from), Value::Integer(to)]));
                }
            }
            let bars = match variants.len() {
                0 => return None,
                1 => variants.remove(0),
                _ => Value::Array(variants),
            };
            let mut section = Table::new();
            section.insert("name".into(), Value::String((*name).to_string()));
            section.insert("bars".into(), bars);
            Some(Value::Table(section))
        })
        .collect();
    if !sections.is_empty() {
        music.insert("section".into(), Value::Array(sections));
    }
    if !stings.is_empty() {
        let stingers = stings
            .iter()
            .map(|sting| {
                let mut table = Table::new();
                table.insert("name".into(), Value::String(sting.name.clone()));
                table.insert("event".into(), Value::String(sting.event.clone()));
                table.insert("file".into(), Value::String(sting.file.clone()));
                Value::Table(table)
            })
            .collect();
        music.insert("stinger".into(), Value::Array(stingers));
    }
    music
}

fn gain(parts: &[Part], plays: &[bool]) -> Option<Value> {
    let mut keys: Vec<(f32, f32)> = Vec::new();
    for (part, on) in parts.iter().zip(plays) {
        let at = (part.intensity[0] + part.intensity[1]) * 0.5;
        let level = if *on { 1.0 } else { 0.0 };
        match keys.iter_mut().find(|(k, _)| (*k - at).abs() < 1e-6) {
            Some(key) => key.1 = key.1.max(level),
            None => keys.push((at, level)),
        }
    }
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    if keys.is_empty() || keys.iter().all(|(_, g)| *g == 1.0) || keys.iter().all(|(_, g)| *g == 0.0)
    {
        return None;
    }
    Some(Value::Array(
        keys.iter()
            .map(|(at, g)| Value::Array(vec![number(f64::from(*at)), number(f64::from(*g))]))
            .collect(),
    ))
}

fn number(value: f64) -> Value {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
        Value::Integer(rounded as i64)
    } else {
        Value::Float(rounded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_score_becomes_a_music_table_the_game_side_loads() {
        let part = |name: &str, at: f64, intensity: [f32; 2]| Part {
            name: name.into(),
            at,
            intensity,
        };
        let parts = vec![
            part("calm", 0.0, [0.0, 0.2]),
            part("panic", 8.0, [0.8, 1.0]),
            part("calm", 16.0, [0.0, 0.2]),
        ];
        let lines = vec![
            Line {
                name: "pad".into(),
                file: "compose/a/pad.wav".into(),
                plays: vec![true, true, true],
            },
            Line {
                name: "drums".into(),
                file: "compose/a/drums.wav".into(),
                plays: vec![false, true, false],
            },
        ];
        let stings = vec![Sting {
            name: "game_over".into(),
            event: "broken".into(),
            file: "compose/a/cue/game_over.wav".into(),
        }];
        let music = table(120.0, 4, 20.0, &parts, &lines, &stings);
        let mut doc = Table::new();
        doc.insert("music".into(), Value::Table(music));
        let text = toml::to_string(&doc).unwrap();
        let manifest = pfx_sound::manifest::Manifest::parse(&text)
            .unwrap()
            .unwrap();
        assert_eq!(manifest.stems.len(), 2);
        assert_eq!(manifest.stems[0].gain.keys, vec![(0.0, 1.0)]);
        assert_eq!(manifest.stems[1].gain.keys, vec![(0.1, 0.0), (0.9, 1.0)]);
        let sections = &manifest.track.sections;
        assert_eq!(sections[0].name, "calm");
        assert_eq!(sections[0].variants, vec![(0, 4), (8, 10)]);
        assert_eq!(sections[1].variants, vec![(4, 8)]);
        assert_eq!(manifest.track.start.as_deref(), Some("calm"));
        assert_eq!(manifest.stingers.len(), 1);
        assert_eq!(manifest.stingers[0].event, "broken");
        assert_eq!(manifest.stingers[0].file, "compose/a/cue/game_over.wav");
    }
}
