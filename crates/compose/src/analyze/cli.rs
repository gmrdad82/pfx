use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::source::{self, Source};
use super::store::{self, Written};
use super::{Analysis, Piece, audio, scores};
use crate::doc::{Compose, find};

pub const USAGE: &str = "pfx compose analyze [OPTIONS] --into <INTO> --name <NAME> <SOURCE>";

#[derive(Clone, Debug, PartialEq)]
pub struct Analyze {
    pub source: PathBuf,
    pub into: PathBuf,
    pub name: String,
    pub score: Option<String>,
    pub json: bool,
}

pub trait Films {
    fn pieces(&self, script: &Path) -> Result<Vec<Piece>, String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub analysis: Analysis,
    pub written: Written,
    pub ms: f64,
}

fn name_ok(name: &str) -> Result<(), String> {
    let fine = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if fine {
        Ok(())
    } else {
        Err(format!(
            "--name `{name}` wants letters, digits, `_` and `-` only"
        ))
    }
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "audio".to_string())
}

fn document(path: &Path, only: Option<&str>) -> Result<Vec<Piece>, String> {
    let shown = path.display();
    let bytes = std::fs::read(path).map_err(|e| format!("{shown}: {e}"))?;
    let reader = pfx_file::Reader::from_bytes(&bytes).map_err(|e| format!("{shown}: {e}"))?;
    let found = find(&reader).map_err(|e| format!("{shown}: {e}"))?;
    let mut known: Vec<&str> = found
        .iter()
        .flat_map(|f| f.compose.scores.keys().map(String::as_str))
        .collect();
    known.sort_unstable();
    known.dedup();
    if known.is_empty() {
        return Err(format!(
            "{shown} holds no [compose.score.*]; compose one first or analyze a film or WAV"
        ));
    }
    if let Some(name) = only {
        let chunk = found
            .iter()
            .find(|f| f.compose.scores.contains_key(name))
            .ok_or_else(|| {
                format!(
                    "{shown}: no [compose.score.{name}]; the document has {}",
                    known.join(", ")
                )
            })?;
        return scores::pieces(&chunk.compose, Some(name)).map_err(|e| format!("{shown}: {e}"));
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut pieces = Vec::new();
    for chunk in found {
        let mut compose: Compose = chunk.compose;
        compose.scores.retain(|name, _| seen.insert(name.clone()));
        if compose.scores.is_empty() {
            continue;
        }
        pieces.extend(scores::pieces(&compose, None).map_err(|e| format!("{shown}: {e}"))?);
    }
    pieces.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pieces)
}

fn read(command: &Analyze, films: &dyn Films) -> Result<Vec<Piece>, String> {
    let found = source::detect(&command.source)?;
    if command.score.is_some() && !matches!(found, Source::Document(_)) {
        return Err(format!(
            "--score picks a composed score in a .pitofx; {} is not one",
            command.source.display()
        ));
    }
    match &found {
        Source::Document(path) => document(path, command.score.as_deref()),
        Source::Film(path) => films.pieces(path),
        Source::Audio(path) => {
            let stems = audio::read(path)?;
            Ok(vec![audio::piece(&stem(path), &stems)?])
        }
    }
}

pub fn run(
    command: &Analyze,
    films: &dyn Films,
    clock: &dyn Fn() -> f64,
) -> Result<Outcome, String> {
    name_ok(&command.name)?;
    let start = clock();
    let pieces = read(command, films)?;
    if pieces.is_empty() {
        return Err(format!(
            "{}: found no music to analyze",
            command.source.display()
        ));
    }
    let analysis = super::style(&pieces)?;
    let ms = (clock() - start).max(0.0);
    let written = store::write(&command.into, &command.name, &analysis.style)?;
    Ok(Outcome {
        analysis,
        written,
        ms,
    })
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

pub fn text(command: &Analyze, outcome: &Outcome) -> String {
    let report = &outcome.analysis.report;
    let written = &outcome.written;
    let action = if written.created {
        "created"
    } else if written.replaced {
        "replaced in"
    } else {
        "added to"
    };
    let [lo, hi] = report.bpm;
    let bpm = if lo == hi {
        format!("{lo}")
    } else {
        format!("{lo}-{hi}")
    };
    let correlation = report
        .keys
        .iter()
        .find(|k| k.key == report.key)
        .or(report.keys.first())
        .map_or(String::new(), |k| format!(" (r {:.2})", k.correlation));
    let shown: Vec<String> = report.numerals.iter().take(16).cloned().collect();
    let more = if report.numerals.len() > shown.len() {
        " …"
    } else {
        ""
    };
    let mut lines = vec![
        format!(
            "style {} {action} {} (chunk {})",
            command.name,
            command.into.display(),
            written.chunk
        ),
        format!(
            "from {} piece{}, {} notes, in {:.0} ms",
            report.pieces,
            if report.pieces == 1 { "" } else { "s" },
            report.notes,
            outcome.ms
        ),
        format!("bpm {bpm}, {} beats per bar", report.beats_per_bar),
        format!("key {}{correlation}", report.key),
        format!("chords {}: {}{more}", report.chords, shown.join(" ")),
        format!("roles {}", list(&report.roles)),
        format!("sections {}", list(&report.sections)),
    ];
    if report.coarse {
        lines.push(
            "coarse: derived from audio, so harmony, melody and patches are approximate"
                .to_string(),
        );
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}
