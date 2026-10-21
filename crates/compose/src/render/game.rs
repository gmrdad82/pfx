use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use pfx_sound::Clip;
use pfx_sound::manifest::{Manifest, Music, Stinger};

use super::bounds;
use super::master::Stem;
use super::store::{self, Document};
use super::{RATE, Rendered, Target};
use crate::{Compose, StyleRef};

const CUE_PREFIX: &str = "cue/";

fn part(file: &str) -> Option<(&str, &str)> {
    let rest = file.strip_prefix(store::FOLDER)?.strip_prefix('/')?;
    let (score, part) = rest.split_once('/')?;
    Some((score, part.strip_suffix(".wav")?))
}

fn rendered(
    doc: &Document,
    compose: &Compose,
    name: &str,
    beside: Option<&Path>,
) -> Result<Rendered, String> {
    let score = compose.score(name)?;
    let done = match &score.style {
        StyleRef::Name(style) => super::render(
            compose.style(style)?,
            score,
            &|f| doc.sample(f),
            &|| 0.0,
            Target::Game,
        ),
        StyleRef::File { file, name: style } => {
            let folder = beside.ok_or_else(|| {
                format!("its style is in `{file}`, and the document was read with no folder")
            })?;
            let other = store::linked(folder, file)?;
            let parsed = Compose::parse(&other.text)
                .map_err(|e| format!("{}: {e}", other.path.display()))?;
            super::render(
                parsed.style(style)?,
                score,
                &|f| other.sample(f),
                &|| 0.0,
                Target::Game,
            )
        }
    };
    done.map_err(|e| format!("[compose.score.{name}]: {e}"))
}

fn take(list: &mut Vec<Stem>, name: &str) -> Option<Clip> {
    let at = list.iter().position(|s| s.name == name)?;
    Some(Clip {
        rate: RATE,
        channels: 2,
        samples: list.swap_remove(at).samples,
    })
}

enum Source {
    Chunk,
    Beside(PathBuf),
    Score(String),
}

fn source(
    doc: &Document,
    compose: &Compose,
    marked: Option<&str>,
    beside: Option<&Path>,
    file: &str,
) -> Result<Source, String> {
    if doc.holds(file)? {
        return Ok(Source::Chunk);
    }
    if let Some(folder) = beside {
        let path = store::beside(folder, file)?;
        if path.is_file() {
            return Ok(Source::Beside(path));
        }
    }
    let missing = if beside.is_some() {
        "in neither the document nor its folder"
    } else {
        "not in the document"
    };
    match part(file) {
        Some((score, part)) if compose.scores.contains_key(score) => match marked {
            Some(name) if name == score => Ok(Source::Score(part.to_string())),
            Some(name) => Err(format!(
                "`{file}` is {missing}, and a game renders only [compose.score.{name}], the score [music] is marked with"
            )),
            None => Err(format!(
                "`{file}` is {missing}, and [music] is marked with no score to render"
            )),
        },
        _ => Err(format!("`{file}` is {missing}")),
    }
}

fn read(file: &str, source: &Source, doc: &Document) -> Result<Vec<u8>, String> {
    match source {
        Source::Beside(path) => {
            let size = std::fs::metadata(path)
                .map_err(|e| format!("`{file}`: {e}"))?
                .len();
            if size > bounds::MAX_PART_BYTES {
                return Err(format!(
                    "`{file}` holds {size} bytes; the render reads at most {}",
                    bounds::MAX_PART_BYTES
                ));
            }
            std::fs::read(path).map_err(|e| format!("`{file}`: {e}"))
        }
        _ => doc
            .file_within(file, bounds::MAX_PART_BYTES)?
            .ok_or_else(|| format!("`{file}` left the document")),
    }
}

pub fn music(bytes: &[u8], beside: Option<&Path>) -> Result<Music, String> {
    let doc = Document::from_bytes(Path::new("document"), bytes.to_vec())?;
    let manifest =
        Manifest::parse(&doc.text)?.ok_or("the text holding [compose] has no [music] table")?;
    if manifest.stems.len() > bounds::MAX_ROLES + 1 || manifest.stingers.len() > bounds::MAX_CUES {
        return Err(format!(
            "[music] names {} stems and {} stingers; a game loads at most {} and {}",
            manifest.stems.len(),
            manifest.stingers.len(),
            bounds::MAX_ROLES + 1,
            bounds::MAX_CUES
        ));
    }
    let compose = Compose::parse(&doc.text)?;
    let marked = manifest.score.as_deref();
    let files = manifest
        .stems
        .iter()
        .map(|s| s.file.as_str())
        .chain(manifest.stingers.iter().map(|s| s.file.as_str()));
    let sources = files
        .map(|file| source(&doc, &compose, marked, beside, file))
        .collect::<Result<Vec<Source>, String>>()?;
    let wanted: BTreeSet<&str> = sources
        .iter()
        .filter_map(|s| match s {
            Source::Score(part) => Some(part.as_str()),
            _ => None,
        })
        .collect();
    let mut stems = Vec::new();
    let mut cues = Vec::new();
    if let Some(score) = marked.filter(|_| !wanted.is_empty()) {
        let Rendered {
            stems: all,
            cues: fired,
            ..
        } = rendered(&doc, &compose, score, beside)?;
        stems = all
            .into_iter()
            .filter(|s| wanted.contains(s.name.as_str()))
            .collect();
        cues = fired
            .into_iter()
            .filter(|c| wanted.contains(format!("{CUE_PREFIX}{}", c.name).as_str()))
            .collect();
    }
    let mut held: u64 = stems
        .iter()
        .chain(&cues)
        .map(|s| (s.samples.len() as u64).saturating_mul(4))
        .fold(0, u64::saturating_add);
    let mut sources = sources.into_iter();
    let mut clip = |file: &str| -> Result<Clip, String> {
        let source = sources.next().ok_or("[music] changed while it loaded")?;
        let clip = match &source {
            Source::Score(part) => {
                let found = match part.strip_prefix(CUE_PREFIX) {
                    Some(cue) => take(&mut cues, cue),
                    None => take(&mut stems, part),
                };
                found.ok_or_else(|| {
                    format!(
                        "`{file}`: [compose.score.{}] renders no such part, or [music] names it twice",
                        marked.unwrap_or_default()
                    )
                })?
            }
            _ => {
                let bytes = read(file, &source, &doc)?;
                bounds::part(file, &bytes, &mut held)?;
                Clip::from_wav(&bytes).ok_or_else(|| format!("`{file}` is no WAV pfx reads"))?
            }
        };
        if clip.samples.iter().any(|v| !v.is_finite()) {
            return Err(format!(
                "`{file}` holds a sample that is not a finite number"
            ));
        }
        Ok(clip)
    };
    let mut track = manifest.track.clone();
    for stem in &manifest.stems {
        track.stems.push(pfx_sound::Stem {
            name: stem.name.clone(),
            clip: clip(&stem.file).map_err(|e| format!("stem `{}`: {e}", stem.name))?,
            gain: stem.gain.clone(),
        });
    }
    let mut stingers = Vec::with_capacity(manifest.stingers.len());
    for stinger in &manifest.stingers {
        stingers.push(Stinger {
            name: stinger.name.clone(),
            event: stinger.event.clone(),
            clip: clip(&stinger.file).map_err(|e| format!("stinger `{}`: {e}", stinger.name))?,
        });
    }
    Ok(Music { track, stingers })
}
