use std::collections::BTreeMap;

use crate::doc::Compose;
use crate::score::{Score, StyleRef, Tempo};
use crate::style::{Plays, Style};

use super::{Chords, Origin, Part, Piece, Span};

fn beat_at(tempo: &Tempo, seconds: f64) -> f64 {
    let mut i = 0;
    while i + 1 < tempo.marks.len() && tempo.marks[i + 1].at <= seconds {
        i += 1;
    }
    let m = tempo.marks[i];
    m.beat + (seconds - m.at) * tempo.bpm * m.ratio / 60.0
}

pub fn piece(name: &str, score: &Score, style: Option<&Style>) -> Result<Piece, String> {
    let at = format!("[compose.score.{name}]");
    score.check().map_err(|e| format!("{at} {e}"))?;
    if let Some(style) = style {
        score.check_style(style).map_err(|e| format!("{at} {e}"))?;
    }
    let bpb = f64::from(score.beats_per_bar);
    let beats = f64::from(score.bars) * bpb;
    let parts = score
        .tracks
        .iter()
        .map(|track| {
            let role = style.and_then(|s| s.role(&track.role));
            let plays = role.map(|r| r.plays);
            Part {
                name: track.role.clone(),
                patch: role.map(|r| r.patch.clone()),
                plays,
                pitched: !matches!(plays, Some(Plays::Hit) | Some(Plays::Roll)),
                gain: role.map_or(1.0, |r| r.gain),
                notes: track.notes.clone(),
            }
        })
        .collect();
    let spans = score
        .sections
        .iter()
        .map(|placed| {
            let spec = style.and_then(|s| s.sections.iter().find(|x| x.name == placed.name));
            Span {
                name: placed.name.clone(),
                from: f64::from(placed.from) * bpb,
                beats: f64::from(placed.bars) * bpb,
                tempo: placed.tempo,
                intensity: Some(placed.intensity),
                gain_db: spec.map_or(0.0, |s| s.gain_db),
                hit: spec.and_then(|s| s.hit),
            }
        })
        .collect();
    let tempo = score.tempo();
    let hits = score.hits.iter().map(|s| beat_at(&tempo, *s)).collect();
    let chords = (!score.chords.is_empty()).then(|| Chords {
        beats: match style {
            Some(s) => s.harmony.beats,
            None => beats / score.chords.len() as f64,
        },
        numerals: score.chords.clone(),
        sevenths: style.is_some_and(|s| s.harmony.sevenths),
    });
    Ok(Piece {
        origin: Origin::Score,
        name: name.to_string(),
        bpm: f64::from(score.bpm),
        beats_per_bar: score.beats_per_bar,
        beats,
        key: Some(score.key.clone()),
        chords,
        chroma: None,
        parts,
        spans,
        hits,
        patches: style.map(|s| s.patch.clone()).unwrap_or_default(),
        cues: style.map(|s| s.cues.clone()).unwrap_or_default(),
        master: style.map(|s| s.master.clone()),
    })
}

fn known(scores: &BTreeMap<String, Score>) -> String {
    if scores.is_empty() {
        "the document has no scores".to_string()
    } else {
        let names: Vec<&str> = scores.keys().map(String::as_str).collect();
        format!("the document has {}", names.join(", "))
    }
}

pub fn pieces(compose: &Compose, only: Option<&str>) -> Result<Vec<Piece>, String> {
    let chosen: Vec<(&String, &Score)> = match only {
        Some(name) => {
            let (key, score) = compose
                .scores
                .get_key_value(name)
                .ok_or_else(|| format!("no [compose.score.{name}]; {}", known(&compose.scores)))?;
            vec![(key, score)]
        }
        None => compose.scores.iter().collect(),
    };
    if chosen.is_empty() {
        return Err("the document has no [compose.score.*] to analyze".to_string());
    }
    chosen
        .into_iter()
        .map(|(name, score)| {
            let style = match &score.style {
                StyleRef::Name(style) => compose.styles.get(style),
                StyleRef::File { .. } => None,
            };
            piece(name, score, style)
        })
        .collect()
}
