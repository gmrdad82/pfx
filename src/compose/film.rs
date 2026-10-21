use std::collections::BTreeMap;
use std::path::Path;

use pfx_compose::analyze::cli::Films;
use pfx_compose::analyze::{Origin, Part, Piece, Span};
use pfx_compose::{DriveCurve, Master, Note, Osc, Patch, Wave};
use pfx_core::sim::ln;
use pfx_film::Grid;
use pfx_film::reel::Reel;
use pfx_film::score::{Curve, Kind, Layer, Recipe, Table};
use pfx_film::{Known, Script};

const LUFS: f64 = -16.0;
const START: f64 = 0.05;
const MIDDLE_C: f64 = 60.0;
const MOST_MS: f32 = 60_000.0;
const MOST_GAIN: f64 = 8.0;

pub struct Film;

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn marks(raw: &toml::Table) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let scenes = raw.get("scene").and_then(toml::Value::as_array);
    for scene in scenes.into_iter().flatten() {
        let arts = scene.get("art").and_then(toml::Value::as_array);
        for art in arts.into_iter().flatten() {
            let mark = art.get("kind").and_then(toml::Value::as_str) == Some("mark");
            if let (true, Some(file)) = (mark, art.get("file").and_then(toml::Value::as_str))
                && !out.iter().any(|f| f == file)
            {
                out.push(file.to_string());
            }
        }
    }
    out
}

fn score_text(raw: &toml::Table, base: &Path) -> Result<String, String> {
    let named = raw
        .get("film")
        .and_then(|f| f.get("score"))
        .and_then(toml::Value::as_str)
        .ok_or("analyze needs [film] score, the recipes its cues play")?;
    read(&base.join(named))
}

fn wave(kind: Kind) -> Wave {
    match kind {
        Kind::Sine => Wave::Sine,
        Kind::Square => Wave::Square,
        Kind::Saw => Wave::Saw,
        Kind::White => Wave::White,
        Kind::Pink => Wave::Pink,
    }
}

fn noise(kind: Kind) -> bool {
    matches!(kind, Kind::White | Kind::Pink)
}

fn midi_of(hz: f32) -> f64 {
    69.0 + 12.0 * (ln(f64::from(hz) / 440.0) / ln(2.0))
}

fn tone(layer: &Layer) -> Option<f64> {
    let wave = layer.wave?;
    if noise(wave) {
        return None;
    }
    let base = layer
        .note
        .map(f64::from)
        .or(layer.hz.filter(|hz| *hz > 0.0).map(midi_of))?;
    Some(base + f64::from(layer.pitch) + f64::from(layer.cents) / 100.0)
}

fn root(recipe: &Recipe) -> Option<f64> {
    recipe.layers.iter().find_map(tone)
}

fn ms(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, MOST_MS)
    } else {
        0.0
    }
}

fn envelope(layer: &Layer) -> [f32; 4] {
    if layer.sustain > 0.0 {
        [
            ms(layer.attack),
            ms(layer.decay),
            layer.sustain.clamp(0.0, 1.0),
            ms(layer.release),
        ]
    } else {
        [ms(layer.attack), ms(layer.decay), 0.0, ms(layer.release)]
    }
}

fn osc(layer: &Layer, root: Option<f64>, env: [f32; 4]) -> Option<Osc> {
    let kind = layer.wave?;
    let offset = match (tone(layer), root) {
        (Some(pitch), Some(root)) => (pitch - root) * 100.0,
        _ => 0.0,
    };
    let octave = (offset / 1200.0).round().clamp(-8.0, 8.0);
    let cents = (offset - octave * 1200.0).clamp(-1200.0, 1200.0);
    let own = envelope(layer);
    Some(Osc {
        wave: Some(wave(kind)),
        sample: None,
        width: None,
        cents: cents as f32,
        octave: octave as i32,
        gain: layer.volume.clamp(0.0, MOST_GAIN as f32),
        hz: None,
        to_hz: None,
        glide_ms: ms(layer.glide),
        bend: layer.bend.clamp(-48.0, 48.0),
        pan: layer.pan.clamp(-1.0, 1.0),
        at_ms: ms(layer.at),
        env: (own != env).then_some(own),
    })
}

fn cutoff(hz: f32) -> f32 {
    hz.clamp(1.0, 24_000.0)
}

fn patch(recipe: &Recipe) -> Option<Patch> {
    let root = root(recipe);
    let first = recipe.layers.iter().find(|l| l.wave.is_some())?;
    let env = envelope(first);
    let osc: Vec<Osc> = recipe
        .layers
        .iter()
        .filter_map(|l| osc(l, root, env))
        .take(8)
        .collect();
    let (lowpass, sweep, sweep_ms) = match recipe.lowpass.as_slice() {
        [] => (None, None, None),
        [[_, hz]] => (Some([cutoff(*hz); 2]), None, None),
        [a, .., b] => (
            None,
            Some([cutoff(a[1]), cutoff(b[1])]),
            Some(ms(b[0] - a[0])),
        ),
    };
    let drive = recipe.drive.as_ref().map(|d| {
        let curve = match d.curve {
            Curve::Soft => DriveCurve::Soft,
            Curve::Tanh => DriveCurve::Tanh,
            Curve::Hard => DriveCurve::Hard,
        };
        (
            curve,
            d.stage().drive_db.clamp(-60.0, 60.0),
            d.mix.clamp(0.0, 1.0),
        )
    });
    Some(Patch {
        osc,
        env,
        linear: first.linear,
        lowpass,
        highpass: recipe.highpass.first().map(|p| cutoff(p[1])),
        sweep,
        sweep_ms,
        drive,
        send: 0.0,
        gain: 1.0,
        pan: 0.0,
    })
}

fn pitched(recipe: &Recipe) -> bool {
    root(recipe).is_some()
}

fn bpm_at(grid: &Grid, beat: f64) -> f64 {
    let at = grid.tempo.iter().rposition(|c| c.beat <= beat).unwrap_or(0);
    grid.tempo[at].bpm
}

fn piece(name: &str, script: &Script<Reel>, table: &Table) -> Result<Piece, String> {
    let grid = script
        .film
        .beat
        .as_ref()
        .ok_or("analyze needs [film] bpm, the beat grid its cues sit on")?;
    let bpm = grid.bpm();
    let bpb = grid.beats_per_bar();
    let per_bar = f64::from(bpb.max(1));
    let origin = (grid.beat(0.0) / per_bar).floor().min(0.0) * per_bar;
    let beat = |seconds: f64| grid.beat(seconds) - origin;
    let starts = script.starts();
    let mut parts: BTreeMap<String, Part> = BTreeMap::new();
    let mut spans = Vec::with_capacity(script.scenes.len());
    let mut hits = Vec::new();
    let mut patches = BTreeMap::new();
    for (scene, &start) in script.scenes.iter().zip(&starts) {
        let from = beat(start);
        let end = beat(start + scene.seconds);
        let mut hit = None;
        for sound in &scene.sound {
            let recipe = table
                .recipes
                .get(&sound.sound)
                .ok_or_else(|| format!("the score has no sound '{}'", sound.sound))?;
            let shape = patch(recipe);
            let pitched = pitched(recipe);
            let midi = root(recipe).unwrap_or(MIDDLE_C) + f64::from(sound.pitch);
            let length = sound
                .seconds
                .map_or(f64::from(recipe.length) / 1000.0, f64::from);
            let part = parts.entry(sound.sound.clone()).or_insert_with(|| Part {
                name: sound.sound.clone(),
                patch: shape.as_ref().map(|_| sound.sound.clone()),
                plays: None,
                pitched,
                gain: 1.0,
                notes: Vec::new(),
            });
            if let Some(shape) = shape {
                patches.entry(sound.sound.clone()).or_insert(shape);
            }
            for at in sound.times(scene.seconds) {
                let seconds = start + at;
                let on = beat(seconds);
                let off = beat(seconds + length);
                part.notes.push(Note::new(
                    on,
                    midi.clamp(0.0, 127.0),
                    (off - on).max(0.0),
                    f64::from(sound.gain).clamp(0.0, MOST_GAIN),
                ));
                if hit.is_none() && !pitched && at.abs() <= START {
                    hits.push(on);
                    hit = Some(hits.len() - 1);
                }
            }
        }
        spans.push(Span {
            name: scene.name.clone(),
            from,
            beats: (end - from).max(0.0),
            tempo: bpm_at(grid, grid.beat(start)) / bpm,
            intensity: None,
            gain_db: scene.score_db,
            hit,
        });
    }
    let mut parts: Vec<Part> = parts.into_values().collect();
    for part in &mut parts {
        part.notes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    }
    let beats = beat(script.seconds());
    Ok(Piece {
        origin: Origin::Film,
        name: name.to_string(),
        bpm,
        beats_per_bar: bpb,
        beats,
        key: None,
        chords: None,
        chroma: None,
        parts,
        spans,
        hits,
        patches,
        cues: Vec::new(),
        master: Some(Master {
            lufs: LUFS,
            true_peak: script.film.true_peak,
            room: [0.0; 3],
            sidechain: None,
            fade: 0.0,
        }),
    })
}

impl Films for Film {
    fn pieces(&self, script: &Path) -> Result<Vec<Piece>, String> {
        let at = |e: String| format!("{}: {e}", script.display());
        let text = read(script)?;
        let base = script
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let raw: toml::Table = toml::from_str(&text).map_err(|e| at(e.to_string()))?;
        let table = Table::parse(&score_text(&raw, base).map_err(at)?, &[]).map_err(at)?;
        let sounds = |name: &str| table.recipes.contains_key(name);
        let marks = marks(&raw);
        let names: Vec<&str> = marks.iter().map(String::as_str).collect();
        let known = Known {
            sounds: &sounds,
            marks: &names,
        };
        let parsed: Script<Reel> = Script::parse(&text, &known).map_err(at)?;
        let name = script
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .map(|s| s.strip_suffix(".film").map_or(s.clone(), str::to_string))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "film".to_string());
        Ok(vec![piece(&name, &parsed, &table).map_err(at)?])
    }
}
