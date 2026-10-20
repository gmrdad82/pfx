use std::collections::BTreeMap;
use std::path::Path;

use pfx_sound::Clip;
use pfx_sound::manifest::{Manifest, Music, Stinger};

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
            &|f| doc.file(f),
            &|| 0.0,
            Target::Game,
        ),
        StyleRef::File { file, name: style } => {
            let folder = beside.ok_or_else(|| {
                format!("its style is in `{file}`, and the document was read with no folder")
            })?;
            let other = Document::open(&store::beside(folder, file)?)?;
            let parsed = Compose::parse(&other.text)
                .map_err(|e| format!("{}: {e}", other.path.display()))?;
            super::render(
                parsed.style(style)?,
                score,
                &|f| other.file(f),
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

pub fn music(bytes: &[u8], beside: Option<&Path>) -> Result<Music, String> {
    let doc = Document::from_bytes(Path::new("document"), bytes.to_vec())?;
    let manifest =
        Manifest::parse(&doc.text)?.ok_or("the text holding [compose] has no [music] table")?;
    let compose = Compose::parse(&doc.text)?;
    let mut scores: BTreeMap<String, Rendered> = BTreeMap::new();
    let mut clip = |file: &str| -> Result<Clip, String> {
        let bytes = match doc.file(file)? {
            Some(bytes) => bytes,
            None => {
                if let Some((score, part)) = part(file)
                    && compose.scores.contains_key(score)
                {
                    if !scores.contains_key(score) {
                        let done = rendered(&doc, &compose, score, beside)?;
                        scores.insert(score.to_string(), done);
                    }
                    let done = scores
                        .get_mut(score)
                        .ok_or("a rendered score went missing")?;
                    let found = match part.strip_prefix(CUE_PREFIX) {
                        Some(cue) => take(&mut done.cues, cue),
                        None => take(&mut done.stems, part),
                    };
                    return found.ok_or_else(|| {
                        format!("`{file}`: [compose.score.{score}] renders no such part, or [music] names it twice")
                    });
                }
                let folder = beside.ok_or_else(|| format!("`{file}` is not in the document"))?;
                let path = store::beside(folder, file)?;
                std::fs::read(&path).map_err(|e| format!("`{file}`: {e}"))?
            }
        };
        Clip::from_wav(&bytes).ok_or_else(|| format!("`{file}` is no WAV pfx reads"))
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
