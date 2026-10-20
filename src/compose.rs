use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pfx_compose::render::{
    self, Rendered,
    store::{self, Document},
};
use pfx_compose::{Compose as Composed, Request, Score, Style, StyleRef};
use pfx_core::cli::{self, Refusal};
use serde_json::{Value, json};

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
    pub wav: Option<PathBuf>,
    pub json: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Render {
    pub doc: PathBuf,
    pub score: String,
    pub music: bool,
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
        &["--music", "--json"],
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
        wav: value("--wav").map(PathBuf::from),
        json: found.switches.contains("--json"),
    })
}

fn plan_render(args: &[String]) -> Result<Render, Refusal> {
    let found = words(args, &["--score", "--wav"], &["--music", "--json"], RENDER)?;
    missing(&found, &["--score"], RENDER)?;
    Ok(Render {
        doc: found.doc.clone().unwrap_or_default(),
        score: name_ok(&found.values["--score"], "--score")?,
        music: found.switches.contains("--music"),
        wav: found.values.get("--wav").map(PathBuf::from),
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

struct Done {
    score: Score,
    rendered: Rendered,
    compose_ms: f64,
    name: String,
    style: String,
}

fn finish(
    doc: &Document,
    style: &Style,
    done: &Done,
    text: String,
    music: bool,
    wav: Option<&Path>,
) -> Result<(), String> {
    let mut text = text;
    if music {
        let table = render::music_table(style, &done.score, &done.name, &done.rendered);
        text = store::set_table(&text, "music", table)?;
    }
    let mut files: Vec<(String, Vec<u8>)> = done
        .rendered
        .stems
        .iter()
        .map(|s| {
            (
                store::stem_file(&done.name, &s.name),
                render::wav(&s.samples),
            )
        })
        .collect();
    files.extend(done.rendered.cues.iter().map(|c| {
        (
            render::cue_file(&done.name, &c.name),
            render::wav(&c.samples),
        )
    }));
    let mix = render::wav(&done.rendered.master.mix);
    files.push((store::mix_file(&done.name), mix.clone()));
    doc.save(&text, Some(&done.name), &files)?;
    if let Some(path) = wav {
        std::fs::write(path, &mix).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

fn report(doc: &Path, done: &Done, json: bool, total_ms: f64) {
    let score = &done.score;
    let master = &done.rendered.master;
    let notes: usize = score.tracks.iter().map(|t| t.notes.len()).sum();
    let round = |v: f64, places: i32| {
        let k = 10f64.powi(places);
        (v * k).round() / k
    };
    if json {
        let line = json!({
            "doc": doc.display().to_string(),
            "score": done.name,
            "style": done.style,
            "seed": score.seed,
            "seconds": score.seconds,
            "bpm": score.bpm,
            "key": score.key,
            "chords": score.chords,
            "sections": score.sections.iter().map(|s| json!({
                "name": s.name, "from": s.from, "bars": s.bars, "at": round(s.at, 3)
            })).collect::<Vec<Value>>(),
            "notes": notes,
            "stems": done.rendered.stems.iter().map(|s| store::stem_file(&done.name, &s.name)).collect::<Vec<_>>(),
            "mix": store::mix_file(&done.name),
            "compose_ms": round(done.compose_ms, 3),
            "render_ms": round(done.rendered.render_ms, 1),
            "master_ms": round(done.rendered.master_ms, 1),
            "total_ms": round(total_ms, 1),
            "gain_db": round(master.gain_db, 2),
            "short_term_max_lufs": round(master.short_term_max, 2),
            "integrated_lufs": round(master.integrated, 2),
            "true_peak_dbtp": round(master.true_peak_db, 2),
        });
        println!("{line}");
        return;
    }
    println!(
        "{}: [compose.score.{}] {} at {} bpm, {} sections, {notes} notes, {} stems; {:.2} LUFS short-term max, {:.2} LUFS integrated, {:.2} dBTP; {:.0} ms",
        doc.display(),
        done.name,
        score.key,
        score.bpm,
        score.sections.len(),
        done.rendered.stems.len(),
        master.short_term_max,
        master.integrated,
        master.true_peak_db,
        total_ms,
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
    let rendered = render::render(style, &score, &|file| doc.file(file), &now)
        .map_err(|e| format!("{shown}: {e}"))?;
    let text =
        pfx_compose::write_score(&doc.text, &name, &score).map_err(|e| format!("{shown}: {e}"))?;
    let done = Done {
        score,
        rendered,
        compose_ms,
        name,
        style: command.style.clone(),
    };
    finish(
        &doc,
        style,
        &done,
        text,
        command.music,
        command.wav.as_deref(),
    )?;
    report(&command.doc, &done, command.json, now() * 1000.0);
    Ok(())
}

fn beside(doc: &Document, file: &str) -> Result<Document, String> {
    let relative = Path::new(file);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(format!(
            "`{file}` must name a document in {}'s own folder",
            doc.path.display()
        ));
    }
    let folder = doc.path.parent().unwrap_or(Path::new(""));
    Document::open(&folder.join(relative))
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
    let (home, held) = match &score.style {
        StyleRef::Name(_) => (doc.clone(), None),
        StyleRef::File { file, .. } => {
            let other = beside(&doc, file)?;
            let parsed = Composed::parse(&other.text)
                .map_err(|e| format!("{}: {e}", other.path.display()))?;
            (other, Some(parsed))
        }
    };
    let styles = held.as_ref().unwrap_or(&parsed);
    let style = styles
        .style(score.style.name())
        .map_err(|e| format!("{}: {e}", home.path.display()))?;
    let rendered = render::render(style, &score, &|file| home.file(file), &now)
        .map_err(|e| format!("{shown}: {e}"))?;
    let done = Done {
        style: score.style.name().to_string(),
        score,
        rendered,
        compose_ms: 0.0,
        name: command.score.clone(),
    };
    finish(
        &doc,
        style,
        &done,
        doc.text.clone(),
        command.music,
        command.wav.as_deref(),
    )?;
    report(&command.doc, &done, command.json, now() * 1000.0);
    Ok(())
}

fn run_list(command: &List) -> Result<(), String> {
    let doc = Document::open(&command.doc)?;
    let table = compose_table(&doc.text).map_err(|e| format!("{}: {e}", command.doc.display()))?;
    let styles = names(&table, "style");
    let scores = names(&table, "score");
    if command.json {
        println!("{}", json!({ "styles": styles, "scores": scores }));
        return Ok(());
    }
    for style in &styles {
        println!("style[{style}]");
    }
    for score in &scores {
        let stems = doc.file(&store::mix_file(score))?.is_some();
        println!("score[{score}]{}", if stems { "  rendered" } else { "" });
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
