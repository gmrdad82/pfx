use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pfx_compose::render::{
    self, Rendered,
    store::{self, Document},
};
use pfx_compose::{Compose as Composed, Request, Score, Style, StyleRef};
use pfx_core::cli::{self, Refusal};
use serde_json::{Value, json};

mod cache;

pub const USAGE: &str =
    "pfx compose [OPTIONS] --style <STYLE> --seed <SEED> --seconds <SECONDS> <DOC>";
const RENDER: &str = "pfx compose render [OPTIONS] --score <SCORE> <DOC>";
const LIST: &str = "pfx compose list [OPTIONS] <DOC>";
const PRINT: &str = "pfx compose print [OPTIONS] <DOC>";
const COMMANDS: &[&str] = &["render", "list", "print"];

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Compose(Compose),
    Render(Render),
    List(List),
    Print(Print),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Compose {
    pub doc: PathBuf,
    pub style: String,
    pub seed: u64,
    pub seconds: f64,
    pub name: Option<String>,
    pub hits: Vec<f64>,
    pub bpm: Option<u32>,
    pub key: Option<String>,
    pub music: bool,
    pub fresh: bool,
    pub wav: Option<PathBuf>,
    pub json: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Render {
    pub doc: PathBuf,
    pub score: String,
    pub music: bool,
    pub fresh: bool,
    pub wav: Option<PathBuf>,
    pub json: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct List {
    pub doc: PathBuf,
    pub json: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    All,
    Style(String),
    Score(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Print {
    pub doc: PathBuf,
    pub pick: Pick,
    pub json: bool,
}

struct Words {
    doc: Option<PathBuf>,
    values: BTreeMap<&'static str, String>,
    switches: BTreeSet<&'static str>,
}

fn words(
    args: &[String],
    values: &[&'static str],
    switches: &[&'static str],
    usage: &str,
) -> Result<Words, Refusal> {
    let refuse = |message: String| Refusal::new(message).under(usage);
    let mut found = Words {
        doc: None,
        values: BTreeMap::new(),
        switches: BTreeSet::new(),
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        if let Some(switch) = switches.iter().find(|s| **s == flag) {
            if inline.is_some() {
                return Err(refuse(cli::unexpected(arg)));
            }
            if !found.switches.insert(switch) {
                return Err(refuse(cli::repeated(flag)));
            }
            continue;
        }
        if let Some(name) = values.iter().find(|s| **s == flag) {
            let value = inline
                .or_else(|| rest.next().cloned())
                .filter(|v| !v.is_empty() && !v.starts_with("--"))
                .ok_or_else(|| Refusal::new(cli::missing_value(flag)))?;
            if found.values.insert(name, value).is_some() {
                return Err(refuse(cli::repeated(&cli::with_value(flag))));
            }
            continue;
        }
        if flag.starts_with('-') && flag.len() > 1 {
            return Err(refuse(cli::unexpected(arg)));
        }
        if found.doc.is_some() {
            return Err(refuse(cli::unexpected(arg)));
        }
        found.doc = Some(PathBuf::from(arg));
    }
    Ok(found)
}

fn missing(found: &Words, needed: &[&'static str], usage: &str) -> Result<(), Refusal> {
    let mut absent: Vec<String> = needed
        .iter()
        .filter(|flag| !found.values.contains_key(*flag))
        .map(|flag| cli::with_value(flag))
        .collect();
    if found.doc.is_none() {
        absent.push("<DOC>".to_string());
    }
    if absent.is_empty() {
        Ok(())
    } else {
        Err(Refusal::new(cli::required(&absent)).under(usage))
    }
}

fn parsed<T: std::str::FromStr>(
    value: &str,
    flag: &str,
    why: &str,
    ok: impl Fn(&T) -> bool,
) -> Result<T, Refusal> {
    value
        .parse::<T>()
        .ok()
        .filter(|parsed| ok(parsed))
        .ok_or_else(|| Refusal::new(cli::invalid(value, flag, why)))
}

fn hits(value: &str) -> Result<Vec<f64>, Refusal> {
    let mut out = Vec::new();
    for part in value.split(',') {
        let hit: f64 = parsed(
            part.trim(),
            "--hits",
            "expected seconds from 0 up, comma separated and rising",
            |h: &f64| h.is_finite() && *h >= 0.0,
        )
        .map_err(|_| {
            Refusal::new(cli::invalid(
                value,
                "--hits",
                "expected seconds from 0 up, comma separated and rising",
            ))
        })?;
        if out.last().is_some_and(|last| hit <= *last) {
            return Err(Refusal::new(cli::invalid(
                value,
                "--hits",
                "expected seconds from 0 up, comma separated and rising",
            )));
        }
        out.push(hit);
    }
    Ok(out)
}

fn name_ok(value: &str, flag: &str) -> Result<String, Refusal> {
    let fine = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if fine {
        Ok(value.to_string())
    } else {
        Err(Refusal::new(cli::invalid(
            value,
            flag,
            "expected letters, digits, '_' or '-'",
        )))
    }
}

fn wav_path(value: &str) -> Result<PathBuf, Refusal> {
    let path = PathBuf::from(value);
    let wav = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
    if wav {
        Ok(path)
    } else {
        Err(Refusal::new(cli::invalid(
            value,
            "--wav",
            "expected a path ending in .wav",
        )))
    }
}

pub fn plan(args: &[String]) -> Result<Command, Refusal> {
    match args.first().map(String::as_str) {
        Some("render") => plan_render(&args[1..]).map(Command::Render),
        Some("list") => {
            let found = words(&args[1..], &[], &["--json"], LIST)?;
            missing(&found, &[], LIST)?;
            Ok(Command::List(List {
                doc: found.doc.unwrap_or_default(),
                json: found.switches.contains("--json"),
            }))
        }
        Some("print") => plan_print(&args[1..]).map(Command::Print),
        Some(word)
            if !word.starts_with('-')
                && !word.contains(['.', '/', '\\'])
                && !COMMANDS.contains(&word) =>
        {
            Err(Refusal::new(cli::unrecognized(word)).under("pfx compose <COMMAND>"))
        }
        _ => plan_compose(args).map(Command::Compose),
    }
}

fn plan_compose(args: &[String]) -> Result<Compose, Refusal> {
    let found = words(
        args,
        &[
            "--style",
            "--seed",
            "--seconds",
            "--name",
            "--hits",
            "--bpm",
            "--key",
            "--wav",
        ],
        &["--music", "--fresh", "--json"],
        USAGE,
    )?;
    missing(&found, &["--style", "--seed", "--seconds"], USAGE)?;
    let value = |flag: &str| found.values.get(flag).map(String::as_str);
    let seed = parsed(
        value("--seed").unwrap_or_default(),
        "--seed",
        "expected a whole number",
        |_: &u64| true,
    )?;
    let seconds = parsed(
        value("--seconds").unwrap_or_default(),
        "--seconds",
        "expected seconds above 0, up to 600",
        |s: &f64| s.is_finite() && *s > 0.0 && *s <= 600.0,
    )?;
    let bpm = value("--bpm")
        .map(|v| {
            parsed(
                v,
                "--bpm",
                "expected a whole number from 20 to 400",
                |b: &u32| (20..=400).contains(b),
            )
        })
        .transpose()?;
    let hits = value("--hits").map(hits).transpose()?.unwrap_or_default();
    if let Some(last) = hits.last()
        && *last >= seconds
    {
        return Err(Refusal::new(cli::invalid(
            value("--hits").unwrap_or_default(),
            "--hits",
            "every hit must fall inside --seconds",
        )));
    }
    Ok(Compose {
        doc: found.doc.clone().unwrap_or_default(),
        style: name_ok(value("--style").unwrap_or_default(), "--style")?,
        seed,
        seconds,
        name: value("--name").map(|v| name_ok(v, "--name")).transpose()?,
        hits,
        bpm,
        key: value("--key").map(str::to_string),
        music: found.switches.contains("--music"),
        fresh: found.switches.contains("--fresh"),
        wav: value("--wav").map(wav_path).transpose()?,
        json: found.switches.contains("--json"),
    })
}

fn plan_render(args: &[String]) -> Result<Render, Refusal> {
    let found = words(
        args,
        &["--score", "--wav"],
        &["--music", "--fresh", "--json"],
        RENDER,
    )?;
    missing(&found, &["--score"], RENDER)?;
    Ok(Render {
        doc: found.doc.clone().unwrap_or_default(),
        score: name_ok(&found.values["--score"], "--score")?,
        music: found.switches.contains("--music"),
        fresh: found.switches.contains("--fresh"),
        wav: found.values.get("--wav").map(|v| wav_path(v)).transpose()?,
        json: found.switches.contains("--json"),
    })
}

fn plan_print(args: &[String]) -> Result<Print, Refusal> {
    let found = words(args, &["--style", "--score"], &["--json"], PRINT)?;
    missing(&found, &[], PRINT)?;
    let pick = match (found.values.get("--style"), found.values.get("--score")) {
        (Some(_), Some(_)) => {
            return Err(
                Refusal::new(cli::conflict("--style <STYLE>", "--score <SCORE>")).under(PRINT),
            );
        }
        (Some(style), None) => Pick::Style(name_ok(style, "--style")?),
        (None, Some(score)) => Pick::Score(name_ok(score, "--score")?),
        (None, None) => Pick::All,
    };
    Ok(Print {
        doc: found.doc.clone().unwrap_or_default(),
        pick,
        json: found.switches.contains("--json"),
    })
}

fn clock() -> impl Fn() -> f64 {
    let started = std::time::Instant::now();
    move || started.elapsed().as_secs_f64()
}

fn compose_table(text: &str) -> Result<toml::Table, String> {
    let doc: toml::Table = text.parse().map_err(|e| format!("{e}"))?;
    Ok(doc
        .get("compose")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default())
}

fn names(table: &toml::Table, kind: &str) -> Vec<String> {
    table
        .get(kind)
        .and_then(toml::Value::as_table)
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

fn unknown(shown: &str, kind: &str, name: &str, known: &[String]) -> String {
    if known.is_empty() {
        format!("{shown} holds no [compose.{kind}.*] section")
    } else {
        format!(
            "{shown} holds no [compose.{kind}.{name}]; it holds {}",
            known.join(", ")
        )
    }
}

const MIX: &str = "mix.wav";
const LEVELS: &str = "master.json";

#[derive(Clone, Debug, PartialEq)]
struct Levels {
    gain_db: f64,
    short_term_max: f64,
    integrated: f64,
    true_peak_db: f64,
    passes: usize,
}

struct Done {
    score: Score,
    stems: Vec<String>,
    levels: Levels,
    entry: PathBuf,
    cached: bool,
    replaced: bool,
    compose_ms: f64,
    render_ms: f64,
    master_ms: f64,
    name: String,
    style: String,
}

fn whole_wav(path: &Path, bytes: u64) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 12];
    let read = std::io::Read::read_exact(&mut &file, &mut head).is_ok();
    read && &head[..4] == b"RIFF"
        && &head[8..] == b"WAVE"
        && file.metadata().is_ok_and(|m| m.len() == bytes)
}

fn cached(root: &Path, key: &str, touch: bool) -> Option<(PathBuf, Levels)> {
    let entry = cache::get(root, key, &[MIX, LEVELS])?;
    let text = std::fs::read_to_string(entry.join(LEVELS)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let number = |key: &str| {
        let found = value.get(key)?;
        found
            .as_f64()
            .or(found.is_null().then_some(f64::NEG_INFINITY))
    };
    let levels = Levels {
        gain_db: number("gain_db")?,
        short_term_max: number("short_term_max_lufs")?,
        integrated: number("integrated_lufs")?,
        true_peak_db: number("true_peak_dbtp")?,
        passes: value.get("passes").and_then(Value::as_u64)? as usize,
    };
    let bytes = value.get("mix_bytes").and_then(Value::as_u64)?;
    if !whole_wav(&entry.join(MIX), bytes) {
        return None;
    }
    if touch {
        cache::touch(&entry);
    }
    Some((entry, levels))
}

fn levels_of(rendered: &Rendered) -> Levels {
    let master = &rendered.master;
    Levels {
        gain_db: master.gain_db,
        short_term_max: master.short_term_max,
        integrated: master.integrated,
        true_peak_db: master.true_peak_db,
        passes: master.passes,
    }
}

fn stored(
    root: &Path,
    key: &str,
    mix: Vec<f32>,
    levels: &Levels,
    fresh: bool,
) -> Result<PathBuf, String> {
    let values = [
        levels.gain_db,
        levels.short_term_max,
        levels.integrated,
        levels.true_peak_db,
    ];
    if values.iter().any(|v| v.is_nan()) {
        return Err(format!(
            "the master measured {values:?}; a level that isn't a number is a fault in the score or the render"
        ));
    }
    let mix = render::wav(mix);
    let json = json!({
        "gain_db": levels.gain_db,
        "short_term_max_lufs": levels.short_term_max,
        "integrated_lufs": levels.integrated,
        "true_peak_dbtp": levels.true_peak_db,
        "passes": levels.passes,
        "mix_bytes": mix.len(),
    })
    .to_string();
    let keep = |_: &Path| !fresh && cached(root, key, false).is_some();
    cache::put(root, key, &[(MIX, &mix), (LEVELS, json.as_bytes())], &keep)
}

struct Job<'a> {
    style: &'a Style,
    style_name: String,
    score: Score,
    name: String,
    samples: &'a render::Samples<'a>,
    compose_ms: f64,
    replaced: bool,
    fresh: bool,
}

fn produce(job: Job, root: &Path, now: &dyn Fn() -> f64) -> Result<Done, String> {
    let key = cache::key(&render::recipe(job.style, &job.score, job.samples)?);
    let hit = if job.fresh {
        None
    } else {
        cached(root, &key, true)
    };
    let (entry, levels, cached, render_ms, master_ms) = match hit {
        Some((entry, levels)) => (entry, levels, true, 0.0, 0.0),
        None => {
            let rendered = render::render(
                job.style,
                &job.score,
                job.samples,
                now,
                render::Target::Film,
            )?;
            let levels = levels_of(&rendered);
            let (render_ms, master_ms) = (rendered.render_ms, rendered.master_ms);
            let entry = stored(root, &key, rendered.master.mix, &levels, job.fresh)?;
            (entry, levels, false, render_ms, master_ms)
        }
    };
    Ok(Done {
        stems: render::stem_names(job.style),
        score: job.score,
        levels,
        entry,
        cached,
        replaced: job.replaced,
        compose_ms: job.compose_ms,
        render_ms,
        master_ms,
        name: job.name,
        style: job.style_name,
    })
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn refuse_wav(doc: &Path, wav: Option<&Path>) -> Result<(), String> {
    match wav {
        Some(path) if same_file(path, doc) => Err(format!(
            "--wav {} names the document itself; give the mix its own .wav path",
            path.display()
        )),
        _ => Ok(()),
    }
}

fn music_text(
    text: String,
    was: &str,
    style: &Style,
    score: &Score,
    name: &str,
    asked: bool,
) -> Result<(String, bool), String> {
    if !asked && !render::marks_music(was, name) {
        return Ok((text, false));
    }
    let table = render::music_table(style, score, name);
    Ok((store::set_table(&text, "music", table)?, true))
}

fn finish(
    doc: &Document,
    style: &Style,
    done: &Done,
    text: String,
    music: bool,
    drop: bool,
    wav: Option<&Path>,
) -> Result<bool, String> {
    let (text, music) = music_text(text, &doc.text, style, &done.score, &done.name, music)?;
    let written = if drop {
        render::written(style, &done.score, &done.name)
    } else {
        Vec::new()
    };
    if text != doc.text || doc.holds_audio(&written)? {
        doc.save(&text, &written)?;
    }
    if let Some(path) = wav {
        std::fs::copy(done.entry.join(MIX), path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(music)
}

fn report(doc: &Path, done: &Done, music: bool, json: bool, total_ms: f64) {
    let score = &done.score;
    let levels = &done.levels;
    let notes: usize = score.tracks.iter().map(|t| t.notes.len()).sum();
    let round = |v: f64, places: i32| {
        let k = 10f64.powi(places);
        (v * k).round() / k
    };
    let mix = done.entry.join(MIX).display().to_string();
    if json {
        let line = json!({
            "doc": doc.display().to_string(),
            "score": done.name,
            "style": done.style,
            "replaced": done.replaced,
            "seed": score.seed,
            "seconds": score.seconds,
            "bpm": score.bpm,
            "key": score.key,
            "chords": score.chords,
            "sections": score.sections.iter().map(|s| json!({
                "name": s.name, "from": s.from, "bars": s.bars, "at": round(s.at, 3)
            })).collect::<Vec<Value>>(),
            "notes": notes,
            "stems": done.stems,
            "mix": mix,
            "cached": done.cached,
            "music": music,
            "compose_ms": round(done.compose_ms, 3),
            "render_ms": round(done.render_ms, 1),
            "master_ms": round(done.master_ms, 1),
            "total_ms": round(total_ms, 1),
            "passes": levels.passes,
            "gain_db": round(levels.gain_db, 2),
            "short_term_max_lufs": round(levels.short_term_max, 2),
            "integrated_lufs": round(levels.integrated, 2),
            "true_peak_dbtp": round(levels.true_peak_db, 2),
        });
        println!("{line}");
        return;
    }
    println!(
        "{}: [compose.score.{}]{} {} at {} bpm, {} sections, {notes} notes, {} stems; {:.2} LUFS short-term max, {:.2} LUFS integrated, {:.2} dBTP; {:.0} ms{}{}",
        doc.display(),
        done.name,
        if done.replaced { " replaced," } else { "" },
        score.key,
        score.bpm,
        score.sections.len(),
        done.stems.len(),
        levels.short_term_max,
        levels.integrated,
        levels.true_peak_db,
        total_ms,
        if done.cached { ", cached" } else { "" },
        if music { ", [music] written" } else { "" },
    );
}

fn run_compose(command: &Compose) -> Result<(), String> {
    let now = clock();
    let shown = command.doc.display().to_string();
    let doc = Document::open(&command.doc)?;
    let parsed = Composed::parse(&doc.text).map_err(|e| format!("{shown}: {e}"))?;
    let style = parsed
        .style(&command.style)
        .map_err(|e| format!("{shown}: {e}"))?;
    let name = command
        .name
        .clone()
        .unwrap_or_else(|| command.style.clone());
    let request = Request {
        seed: command.seed,
        seconds: command.seconds,
        hits: command.hits.clone(),
        bpm: command.bpm,
        key: command.key.clone(),
    };
    let before = now();
    let score = pfx_compose::compose(&command.style, style, &request)
        .map_err(|e| format!("{shown}: {e}"))?;
    let compose_ms = (now() - before) * 1000.0;
    let text =
        pfx_compose::write_score(&doc.text, &name, &score).map_err(|e| format!("{shown}: {e}"))?;
    let kept = Composed::parse(&text).map_err(|e| format!("{shown}: {e}"))?;
    let score = kept
        .score(&name)
        .map_err(|e| format!("{shown}: {e}"))?
        .clone();
    refuse_wav(&command.doc, command.wav.as_deref())?;
    let samples = |file: &str| doc.sample(file);
    let done = produce(
        Job {
            style,
            style_name: command.style.clone(),
            score,
            name: name.clone(),
            samples: &samples,
            compose_ms,
            replaced: parsed.scores.contains_key(&name),
            fresh: command.fresh,
        },
        &cache::root()?,
        &now,
    )
    .map_err(|e| format!("{shown}: {e}"))?;
    let music = finish(
        &doc,
        style,
        &done,
        text,
        command.music,
        true,
        command.wav.as_deref(),
    )?;
    report(&command.doc, &done, music, command.json, now() * 1000.0);
    Ok(())
}

fn styled<'a>(doc: &'a Document, score: &Score) -> Result<Cow<'a, Document>, String> {
    match &score.style {
        StyleRef::Name(_) => Ok(Cow::Borrowed(doc)),
        StyleRef::File { file, .. } => {
            let folder = doc.path.parent().unwrap_or(Path::new(""));
            let path =
                store::beside(folder, file).map_err(|e| format!("{}: {e}", doc.path.display()))?;
            Document::open(&path).map(Cow::Owned)
        }
    }
}

fn run_render(command: &Render) -> Result<(), String> {
    let now = clock();
    let shown = command.doc.display().to_string();
    let doc = Document::open(&command.doc)?;
    let parsed = Composed::parse(&doc.text).map_err(|e| format!("{shown}: {e}"))?;
    let score = parsed
        .score(&command.score)
        .map_err(|e| format!("{shown}: {e}"))?
        .clone();
    let home = styled(&doc, &score)?;
    let held = Composed::parse(&home.text).map_err(|e| format!("{}: {e}", home.path.display()))?;
    let style = held
        .style(score.style.name())
        .map_err(|e| format!("{}: {e}", home.path.display()))?;
    refuse_wav(&command.doc, command.wav.as_deref())?;
    let samples = |file: &str| home.sample(file);
    let done = produce(
        Job {
            style,
            style_name: score.style.name().to_string(),
            score,
            name: command.score.clone(),
            samples: &samples,
            compose_ms: 0.0,
            replaced: false,
            fresh: command.fresh,
        },
        &cache::root()?,
        &now,
    )
    .map_err(|e| format!("{shown}: {e}"))?;
    let music = finish(
        &doc,
        style,
        &done,
        doc.text.clone(),
        command.music,
        false,
        command.wav.as_deref(),
    )?;
    report(&command.doc, &done, music, command.json, now() * 1000.0);
    Ok(())
}

fn is_cached(doc: &Document, parsed: &Composed, name: &str, root: &Path) -> Result<bool, String> {
    let score = parsed.score(name)?;
    let home = styled(doc, score)?;
    let other;
    let held = match &home {
        Cow::Borrowed(_) => parsed,
        Cow::Owned(other_doc) => {
            other = Composed::parse(&other_doc.text)?;
            &other
        }
    };
    let style = held.style(score.style.name())?;
    let recipe = render::recipe(style, score, &|file| home.sample(file))?;
    Ok(cached(root, &cache::key(&recipe), false).is_some())
}

fn run_list(command: &List) -> Result<(), String> {
    let shown = command.doc.display().to_string();
    let doc = Document::open(&command.doc)?;
    let table = compose_table(&doc.text).map_err(|e| format!("{shown}: {e}"))?;
    let parsed = Composed::parse(&doc.text).map_err(|e| format!("{shown}: {e}"))?;
    let root = cache::root();
    let styles = names(&table, "style");
    let mut scores = Vec::new();
    for score in names(&table, "score") {
        let state = root
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|root| is_cached(&doc, &parsed, &score, root));
        scores.push((score, state));
    }
    if command.json {
        let scores: Vec<Value> = scores
            .iter()
            .map(|(name, state)| match state {
                Ok(cached) => json!({ "name": name, "cached": cached }),
                Err(e) => json!({ "name": name, "cached": false, "error": e }),
            })
            .collect();
        println!("{}", json!({ "styles": styles, "scores": scores }));
        return Ok(());
    }
    for style in &styles {
        println!("style[{style}]");
    }
    for (score, state) in &scores {
        match state {
            Ok(true) => println!("score[{score}]  cached"),
            Ok(false) => println!("score[{score}]"),
            Err(e) => println!("score[{score}]  cannot render: {e}"),
        }
    }
    Ok(())
}

fn run_print(command: &Print) -> Result<(), String> {
    let shown = command.doc.display().to_string();
    let doc = Document::open(&command.doc)?;
    let table = compose_table(&doc.text).map_err(|e| format!("{shown}: {e}"))?;
    let picked = match &command.pick {
        Pick::All => table,
        Pick::Style(name) | Pick::Score(name) => {
            let kind = if matches!(command.pick, Pick::Style(_)) {
                "style"
            } else {
                "score"
            };
            let found = table
                .get(kind)
                .and_then(toml::Value::as_table)
                .and_then(|t| t.get(name))
                .cloned()
                .ok_or_else(|| unknown(&shown, kind, name, &names(&table, kind)))?;
            toml::Table::from_iter([(
                kind.to_string(),
                toml::Value::Table(toml::Table::from_iter([(name.clone(), found)])),
            )])
        }
    };
    let whole = toml::Table::from_iter([("compose".to_string(), toml::Value::Table(picked))]);
    if command.json {
        let value = serde_json::to_value(&whole).map_err(|e| e.to_string())?;
        println!("{value}");
    } else {
        print!("{}", toml::to_string(&whole).map_err(|e| e.to_string())?);
    }
    Ok(())
}

pub fn run(command: &Command) -> Result<(), String> {
    match command {
        Command::Compose(c) => run_compose(c),
        Command::Render(r) => run_render(r),
        Command::List(l) => run_list(l),
        Command::Print(p) => run_print(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: &str = r#"
[compose.style.small]
bpm = [100, 110]
tonics = [["C", 1]]
modes = [["major", 1]]

[compose.style.small.harmony]
beats = 4
start = ["I"]
cadence = ["V", "I"]
markov = { I = [["IV", 2]], IV = [["V", 2]], V = [["I", 2]] }

[compose.style.small.melody]
range = [62, 79]
steps = [[1, 3], [-1, 3]]
rhythm = [[0.5, 2], [1.0, 1]]
rest = 0.1
motif_bars = 1
repeat = 0.5

[[compose.style.small.role]]
name = "kick"
patch = "kick"
plays = "hit"
patterns = ["x...x...x...x..."]
gain = 0.5

[[compose.style.small.role]]
name = "pad"
patch = "pad"
plays = "chord"
octave = -1
gain = 0.05

[compose.style.small.patch.kick]
osc = [{ wave = "sine", hz = 150, to_hz = 55, glide_ms = 40 }]
env = [1, 300, 0.0, 0]

[compose.style.small.patch.pad]
osc = [{ wave = "saw", cents = -8, pan = -0.4 }, { wave = "triangle", cents = 8, pan = 0.4 }]
env = [30, 0, 0.8, 200]
lowpass = [900, 2400]
send = 0.2

[[compose.style.small.section]]
name = "calm"
bars = [2, 3]
roles = ["kick", "pad"]
intensity = [0.0, 0.4]

[compose.style.small.master]
lufs = -16
true_peak = -1.0
room = [0.6, 0.4, 0.15]
fade = 0.5
"#;

    #[test]
    fn composing_again_under_the_name_music_marks_rewrites_the_music_table() {
        let parsed = Composed::parse(STYLE).unwrap();
        let style = parsed.style("small").unwrap();
        let score = |seed: u64, seconds: f64| {
            let request = Request {
                seed,
                seconds,
                hits: Vec::new(),
                bpm: None,
                key: None,
            };
            pfx_compose::compose("small", style, &request).unwrap()
        };
        let table = |text: &str| text.parse::<toml::Table>().unwrap()["music"].clone();
        let first = score(5, 8.0);
        let text = pfx_compose::write_score(STYLE, "title", &first).unwrap();
        let (marked, wrote) =
            music_text(text.clone(), &text, style, &first, "title", true).unwrap();
        assert!(wrote);
        let second = score(6, 12.0);
        let again = pfx_compose::write_score(&marked, "title", &second).unwrap();
        let (again, wrote) = music_text(again, &marked, style, &second, "title", false).unwrap();
        assert!(wrote);
        assert_ne!(table(&again), table(&marked));
        assert_eq!(
            serde_json::to_value(table(&again)).unwrap(),
            serde_json::to_value(render::music_table(style, &second, "title")).unwrap()
        );
        let other = pfx_compose::write_score(&again, "other", &first).unwrap();
        let (kept, wrote) =
            music_text(other.clone(), &again, style, &first, "other", false).unwrap();
        assert!(!wrote);
        assert_eq!(kept, other);
        assert_eq!(table(&kept), table(&again));
    }

    #[test]
    fn a_cache_hit_hands_out_the_bytes_a_fresh_render_makes_and_a_broken_entry_heals() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/compose-tests/cache");
        let _ = std::fs::remove_dir_all(&root);
        let parsed = Composed::parse(STYLE).unwrap();
        let style = parsed.style("small").unwrap();
        let request = Request {
            seed: 5,
            seconds: 4.0,
            hits: Vec::new(),
            bpm: None,
            key: None,
        };
        let score = pfx_compose::compose("small", style, &request).unwrap();
        let none = |_: &str| Ok(None);
        let job = || Job {
            style,
            style_name: "small".into(),
            score: score.clone(),
            name: "small".into(),
            samples: &none,
            compose_ms: 0.0,
            replaced: false,
            fresh: false,
        };
        let clock = || 0.0;
        let cold = produce(job(), &root, &clock).unwrap();
        assert!(!cold.cached);
        let hit = produce(job(), &root, &clock).unwrap();
        assert!(hit.cached);
        assert_eq!(hit.levels.passes, cold.levels.passes);
        let near = |a: f64, b: f64| a == b || (a - b).abs() < 1e-9;
        assert!(near(hit.levels.gain_db, cold.levels.gain_db));
        assert!(near(hit.levels.short_term_max, cold.levels.short_term_max));
        assert!(near(hit.levels.integrated, cold.levels.integrated));
        assert!(near(hit.levels.true_peak_db, cold.levels.true_peak_db));
        let fresh = render::render(style, &score, &none, &clock, render::Target::Film).unwrap();
        let mix = hit.entry.join(MIX);
        assert!(std::fs::read(&mix).unwrap() == render::wav(fresh.master.mix));
        let whole = std::fs::read(&mix).unwrap();
        std::fs::write(&mix, &whole[..whole.len() / 2]).unwrap();
        let healed = produce(job(), &root, &clock).unwrap();
        assert!(!healed.cached);
        assert!(std::fs::read(&mix).unwrap() == whole);
        let mut other = score.clone();
        other.seed += 1;
        let key = |score: &Score| cache::key(&render::recipe(style, score, &none).unwrap());
        assert_ne!(key(&score), key(&other));
        let _ = std::fs::remove_dir_all(&root);
    }
}
