use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use pfx_run::jobs::state::{self, Asked, Job, State};
use pfx_run::jobs::store::{self, Ask};
use pfx_run::jobs::{process, supervise};
use serde_json::{Value, json};

const RUNS: &[&str] = &["clip", "film", "render", "bake"];

#[derive(Parser)]
#[command(name = "pfx", arg_required_else_help = false)]
struct Line {
    #[command(subcommand)]
    verb: Top,
}

#[derive(Subcommand)]
enum Top {
    #[command(
        about = "Long renders run detached as named jobs that carry on from their last finished frame",
        subcommand_required = true,
        arg_required_else_help = false
    )]
    Job(Args),
}

#[derive(clap::Args)]
struct Args {
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    #[command(about = "Run a pfx clip, film, render or bake detached as a named job")]
    Start(Start),
    #[command(about = "List the jobs with their state and frames")]
    List(Json),
    #[command(about = "A job's state, frames, current frame, samples and ETA")]
    Status(Status),
    #[command(about = "Pause a job once the frame in flight is written")]
    Pause(Named),
    #[command(about = "Carry on a paused, stopped or interrupted job from its last finished frame")]
    Resume(Resume),
    #[command(about = "Stop a job once the frame in flight is written, or now")]
    Stop(Stop),
    #[command(about = "Print a job's log: its run's stderr and the supervisor's notes")]
    Log(Log),
    #[command(
        about = "Remove the folders of done and failed jobs; the frames stay where they were made"
    )]
    Clean(Json),
    #[command(hide = true)]
    Supervise(Supervise),
}

#[derive(clap::Args)]
struct Json {
    #[arg(long, help = "Print JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct Start {
    #[arg(
        value_name = "name",
        help = "The job's name: letters, digits, '-', '_' or '.'"
    )]
    name: String,
    #[arg(
        long,
        value_name = "A..B",
        value_parser = state::range,
        help = "Make only frames A to B of a clip, both kept, numbered and seeded as in a whole run"
    )]
    frames: Option<[u32; 2]>,
    #[arg(
        long,
        value_name = "every",
        help = "Save a traced clip's or render's samples this often, in minutes or as samples like 64spp, so a run stopped inside a frame resumes there"
    )]
    checkpoint: Option<String>,
    #[arg(long, help = "Print JSON")]
    json: bool,
    #[arg(
        last = true,
        required = true,
        value_name = "COMMAND",
        help = "The pfx command the job runs, after --: clip, film, render or bake with its arguments"
    )]
    command: Vec<String>,
}

#[derive(clap::Args)]
struct Named {
    #[arg(
        value_name = "name|pid",
        help = "The job's name, or its supervisor's pid"
    )]
    name: String,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct Status {
    #[arg(
        value_name = "name|pid",
        help = "The job's name, or its supervisor's pid"
    )]
    name: String,
    #[arg(
        long,
        help = "Print a line on every change until the job stops running"
    )]
    watch: bool,
    #[arg(long, help = "Print JSON, one line a change with --watch")]
    json: bool,
}

#[derive(clap::Args)]
struct Resume {
    #[arg(
        value_name = "name|pid",
        required_unless_present = "all",
        conflicts_with = "all",
        help = "The job's name, or its supervisor's pid"
    )]
    name: Option<String>,
    #[arg(
        long,
        help = "Carry on every job a crash or a reboot interrupted; paused and stopped ones wait for their name"
    )]
    all: bool,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct Stop {
    #[arg(
        value_name = "name|pid",
        help = "The job's name, or its supervisor's pid"
    )]
    name: String,
    #[arg(
        long,
        help = "End the run now; the frame in flight is made again on resume"
    )]
    now: bool,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct Log {
    #[arg(
        value_name = "name|pid",
        help = "The job's name, or its supervisor's pid"
    )]
    name: String,
    #[arg(long, value_name = "N", help = "Only the last N lines")]
    lines: Option<usize>,
    #[arg(long, help = "Print JSON")]
    json: bool,
}

#[derive(clap::Args)]
struct Supervise {
    name: String,
}

pub fn claims(args: &[String]) -> bool {
    args.first().is_some_and(|word| word == "job")
}

#[cfg(test)]
pub fn help_text(args: &[String]) -> Option<String> {
    let words = std::iter::once("pfx").chain(args.iter().map(String::as_str));
    match Line::try_parse_from(words) {
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            Some(error.render().to_string())
        }
        _ => None,
    }
}

pub fn main(args: &[String]) -> ExitCode {
    let words = std::iter::once("pfx").chain(args.iter().map(String::as_str));
    let Top::Job(parsed) = match Line::try_parse_from(words) {
        Ok(line) => line.verb,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };
    let ran = match parsed.verb {
        Verb::Start(start) => run_start(&start),
        Verb::List(json) => list(json.json),
        Verb::Status(status) => run_status(&status),
        Verb::Pause(named) => hold(&named.name, Ask::Pause, named.json),
        Verb::Resume(resume) => run_resume(&resume),
        Verb::Stop(stop) => hold(
            &stop.name,
            if stop.now { Ask::StopNow } else { Ask::Stop },
            stop.json,
        ),
        Verb::Log(log) => run_log(&log),
        Verb::Clean(json) => clean(json.json),
        Verb::Supervise(named) => return run_supervise(&named.name),
    };
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pfx job: {error}");
            ExitCode::FAILURE
        }
    }
}

fn exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("cannot find this pfx: {e}"))
}

fn now() -> pfx_run::jobs::state::NaiveDateTime {
    supervise::Clock::now(&supervise::Wall)
}

fn command_line(start: &Start) -> Result<Vec<String>, String> {
    let mut argv = start.command.clone();
    let run = argv.first().map(String::as_str).unwrap_or_default();
    if !RUNS.contains(&run) {
        return Err(format!(
            "{run:?}: a job runs one of pfx {}",
            RUNS.join(", ")
        ));
    }
    let given = |flag: &str| {
        argv.iter()
            .any(|arg| arg == flag || arg.starts_with(&format!("{flag}=")))
    };
    if let Some(every) = &start.checkpoint {
        if run != "clip" && run != "render" {
            return Err(format!(
                "--checkpoint {every}: only a traced clip or render keeps checkpoints"
            ));
        }
        if given("--checkpoint") {
            return Err("give the checkpoint once: on the job, or on the command".into());
        }
    }
    let json = run == "clip" && !given("--json");
    let checkpoint = start.checkpoint.clone();
    if let Some([first, last]) = start.frames {
        if run != "clip" {
            return Err(format!(
                "--frames {first}..{last}: only a clip renders a range of frames"
            ));
        }
        if argv
            .iter()
            .any(|arg| arg == "--range" || arg.starts_with("--range="))
        {
            return Err("give the range once: --frames on the job, or --range on the clip".into());
        }
        argv.extend(["--range".to_string(), format!("{first}..{last}")]);
    }
    if let Some(every) = checkpoint {
        argv.extend(["--checkpoint".to_string(), every]);
    }
    if json {
        argv.push("--json".into());
    }
    Ok(argv)
}

fn launch(dir: &Path, name: &str) -> Result<u32, String> {
    let mut command = Command::new(exe()?);
    command
        .args(["job", "supervise", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    process::detach(&mut command);
    let child = command
        .spawn()
        .map_err(|e| format!("cannot start the supervisor: {e}"))?;
    let pid = child.id();
    let asked = Instant::now();
    while asked.elapsed() < Duration::from_secs(10) {
        if store::read(dir)
            .ok()
            .and_then(|job| job.supervisor)
            .is_some_and(|me| me.pid == pid)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(pid)
}

fn run_start(start: &Start) -> Result<(), String> {
    store::check_name(&start.name)?;
    let argv = command_line(start)?;
    let root = store::root()?;
    let dir = root.join(&start.name);
    if let Ok(job) = store::read(&dir) {
        return Err(format!(
            "{}: a job of that name is {}; resume it, or clean it and start again",
            start.name,
            pfx_run::jobs::seen(&dir, &job).name()
        ));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let cwd = std::env::current_dir().map_err(|e| format!("the current folder: {e}"))?;
    let job = Job::new(&start.name, argv, cwd, start.frames, now());
    store::write(&dir, &job)?;
    let pid = launch(&dir, &start.name)?;
    if start.json {
        println!(
            "{}",
            json!({"name": start.name, "pid": pid, "dir": dir.display().to_string(), "state": "running"})
        );
    } else {
        println!(
            "pfx job: {} started, supervisor pid {pid}, in {}",
            start.name,
            dir.display()
        );
    }
    Ok(())
}

fn find(word: &str) -> Result<(PathBuf, Job), String> {
    let root = store::root()?;
    if let Ok(pid) = word.parse::<u32>() {
        return store::all(&root)
            .into_iter()
            .find_map(|(dir, job)| {
                let job = job.ok()?;
                let owns = [&job.supervisor, &job.child]
                    .iter()
                    .any(|proc| proc.as_ref().is_some_and(|proc| proc.pid == pid));
                owns.then_some((dir, job))
            })
            .ok_or_else(|| format!("no job has pid {pid}"));
    }
    store::check_name(word)?;
    let dir = root.join(word);
    let job =
        store::read(&dir).map_err(|_| format!("{word}: no such job in {}", root.display()))?;
    Ok((dir, job))
}

fn seen(dir: &Path, job: &Job) -> Value {
    let state = pfx_run::jobs::seen(dir, job);
    let mut value = serde_json::to_value(job).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.insert("state".into(), json!(state.name()));
        map.insert("stale".into(), json!(state != job.state));
        map.insert("dir".into(), json!(dir.display().to_string()));
    }
    value
}

fn frames(job: &Job) -> String {
    match job.frames.total {
        Some(total) => format!("{} of {total} frames", job.frames.done),
        None => format!("{} frames", job.frames.done),
    }
}

fn line(dir: &Path, job: &Job) -> String {
    let state = pfx_run::jobs::seen(dir, job);
    let mut text = format!("{}  {}  {}", job.name, state.name(), frames(job));
    if let (true, Some(frame)) = (state == State::Running, job.frame) {
        text.push_str(&format!(", frame {frame}"));
        if let Some(samples) = job.samples {
            text.push_str(&match samples.of {
                Some(of) => format!(" at {} of {of} samples", samples.done),
                None => format!(" at {} samples", samples.done),
            });
        }
    }
    if let (false, Some(eta)) = (state.finished(), job.eta_seconds) {
        text.push_str(&format!(", ETA {}", state::duration(eta)));
    }
    if let Some(reason) = job.reason.as_deref().filter(|_| !state.live()) {
        text.push_str(&format!(" ({reason})"));
    }
    if state != job.state {
        text.push_str(" (its supervisor is gone)");
    }
    text
}

fn list(as_json: bool) -> Result<(), String> {
    let root = store::root()?;
    let jobs = store::all(&root);
    if as_json {
        let values: Vec<Value> = jobs
            .iter()
            .map(|(dir, job)| match job {
                Ok(job) => seen(dir, job),
                Err(error) => json!({"dir": dir.display().to_string(), "error": error}),
            })
            .collect();
        println!("{}", Value::Array(values));
        return Ok(());
    }
    if jobs.is_empty() {
        println!("no jobs in {}", root.display());
    }
    for (dir, job) in &jobs {
        match job {
            Ok(job) => println!("{}", line(dir, job)),
            Err(error) => println!("{}: {error}", dir.display()),
        }
    }
    Ok(())
}

fn detail(dir: &Path, job: &Job) -> String {
    let mut text = line(dir, job);
    let mut more = |label: &str, value: String| text.push_str(&format!("\n  {label}: {value}"));
    more("runs", format!("pfx {}", job.argv.join(" ")));
    if let Some([first, last]) = job.range {
        more("range", format!("frames {first} to {last}"));
    }
    if let Some(median) = job.median_frame_seconds {
        more("median frame", state::duration(median));
    }
    if let Some(path) = &job.path {
        more("traced on", path.clone());
    }
    if let Some(resume) = &job.resume {
        more("resumed", resume.line());
    }
    if let Some(me) = &job.supervisor {
        more("supervisor", format!("pid {}", me.pid));
    }
    if let Some(child) = &job.child {
        more("render", format!("pid {}", child.pid));
    }
    more("tries", job.runs.to_string());
    if let Some(error) = &job.last_error {
        more("last error", error.clone());
    }
    if let Some(report) = &job.report {
        more("report", report.clone());
    }
    more("folder", dir.display().to_string());
    text
}

fn run_status(status: &Status) -> Result<(), String> {
    let (dir, job) = find(&status.name)?;
    if !status.watch {
        if status.json {
            println!("{}", seen(&dir, &job));
        } else {
            println!("{}", detail(&dir, &job));
        }
        return Ok(());
    }
    let mut last = String::new();
    loop {
        let job = store::read(&dir)?;
        let shown = if status.json {
            let mut value = seen(&dir, &job);
            if let Value::Object(map) = &mut value {
                map.remove("updated");
            }
            value.to_string()
        } else {
            line(&dir, &job)
        };
        if shown != last {
            if status.json {
                println!("{}", seen(&dir, &job));
            } else {
                println!("{shown}");
            }
            last = shown;
        }
        if !pfx_run::jobs::seen(&dir, &job).live() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn say(as_json: bool, name: &str, state: &str, text: String) {
    if as_json {
        println!("{}", json!({"name": name, "state": state, "said": text}));
    } else {
        println!("pfx job: {text}");
    }
}

fn hold(word: &str, ask: Ask, as_json: bool) -> Result<(), String> {
    let (dir, mut job) = find(word)?;
    let state = pfx_run::jobs::seen(&dir, &job);
    if state.finished() {
        return Err(format!("{} is {}", job.name, state.name()));
    }
    let (word, held) = match ask {
        Ask::Pause => ("pauses", State::Paused),
        _ => ("stops", State::Stopped),
    };
    if pfx_run::jobs::alive(&dir, &job) {
        store::ask(&dir, ask)?;
        let when = if ask == Ask::StopNow {
            "now".to_string()
        } else {
            "once the frame in flight is written".to_string()
        };
        say(
            as_json,
            &job.name,
            state.name(),
            format!("{} {word} {when}", job.name),
        );
        return Ok(());
    }
    job.asked = ask.asked();
    job.state = held;
    job.reason = Some(format!("{} when asked", held.name()));
    store::write(&dir, &job)?;
    say(
        as_json,
        &job.name,
        held.name(),
        format!("{} is {}", job.name, held.name()),
    );
    Ok(())
}

fn carry_on(dir: &Path, mut job: Job) -> Result<u32, String> {
    job.asked = Asked::Run;
    job.state = State::Waiting;
    job.reason = Some("resuming".into());
    job.fails.clear();
    store::write(dir, &job)?;
    launch(dir, &job.name)
}

fn run_resume(resume: &Resume) -> Result<(), String> {
    let Some(word) = &resume.name else {
        let mut resumed = Vec::new();
        for (dir, job) in store::all(&store::root()?) {
            let Ok(job) = job else { continue };
            let state = pfx_run::jobs::seen(&dir, &job);
            if state.finished() || pfx_run::jobs::alive(&dir, &job) || job.asked != Asked::Run {
                continue;
            }
            let name = job.name.clone();
            let pid = carry_on(&dir, job)?;
            if !resume.json {
                println!("pfx job: {name} resumed, supervisor pid {pid}");
            }
            resumed.push(json!({"name": name, "pid": pid}));
        }
        if resume.json {
            println!("{}", json!({"resumed": resumed}));
        } else if resumed.is_empty() {
            println!("pfx job: nothing to resume");
        }
        return Ok(());
    };
    let (dir, job) = find(word)?;
    let state = pfx_run::jobs::seen(&dir, &job);
    if state.finished() {
        return Err(format!(
            "{} is {}; nothing to resume",
            job.name,
            state.name()
        ));
    }
    if pfx_run::jobs::alive(&dir, &job) {
        store::ask(&dir, Ask::Run)?;
        say(
            resume.json,
            &job.name,
            state.name(),
            format!("{} carries on", job.name),
        );
        return Ok(());
    }
    let name = job.name.clone();
    let pid = carry_on(&dir, job)?;
    if resume.json {
        println!("{}", json!({"resumed": [{"name": name, "pid": pid}]}));
    } else {
        println!("pfx job: {name} resumed, supervisor pid {pid}");
    }
    Ok(())
}

fn run_log(log: &Log) -> Result<(), String> {
    let (dir, job) = find(&log.name)?;
    let mut lines = store::log_lines(&dir);
    if let Some(keep) = log.lines {
        let over = lines.len().saturating_sub(keep);
        lines.drain(..over);
    }
    if log.json {
        println!("{}", json!({"name": job.name, "lines": lines}));
    } else {
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}

fn clean(as_json: bool) -> Result<(), String> {
    let root = store::root()?;
    let mut removed = Vec::new();
    for (dir, job) in store::all(&root) {
        let Ok(job) = job else { continue };
        if !pfx_run::jobs::seen(&dir, &job).finished() || dir.parent() != Some(root.as_path()) {
            continue;
        }
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        removed.push(job.name);
    }
    if as_json {
        println!("{}", json!({"removed": removed}));
    } else if removed.is_empty() {
        println!("pfx job: no done or failed jobs to clean");
    } else {
        println!("pfx job: removed {}", removed.join(", "));
    }
    Ok(())
}

fn run_supervise(name: &str) -> ExitCode {
    process::leave_session();
    let ran = store::root().map(|root| root.join(name)).and_then(|dir| {
        let signals = supervise::Signals::install()?;
        supervise::supervise(&dir, &exe()?, &supervise::Wall, signals)
    });
    match ran {
        Ok(State::Failed) | Err(_) => ExitCode::FAILURE,
        Ok(_) => ExitCode::SUCCESS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Verb, String> {
        let words = std::iter::once("pfx").chain(line.split_whitespace());
        Line::try_parse_from(words)
            .map(|line| match line.verb {
                Top::Job(args) => args.verb,
            })
            .map_err(|e| e.to_string())
    }

    fn start(line: &str) -> Start {
        match parse(line).unwrap() {
            Verb::Start(start) => start,
            _ => panic!("not start"),
        }
    }

    #[test]
    fn a_job_runs_one_pfx_render_and_a_range_goes_to_the_clip() {
        let plain = start("job start film -- clip a.pitofx --camera main --out tmp/f");
        assert_eq!(plain.name, "film");
        assert_eq!(command_line(&plain).unwrap()[0], "clip");
        let ranged = start("job start film --frames 40..79 --json -- clip a.pitofx --out tmp/f");
        assert!(ranged.json);
        let argv = command_line(&ranged).unwrap();
        assert_eq!(argv[argv.len() - 3..], ["--range", "40..79", "--json"]);
        let kept = start("job start film --checkpoint 30 -- clip a.pitofx --out tmp/f --json");
        let argv = command_line(&kept).unwrap();
        assert_eq!(argv[argv.len() - 3..], ["--json", "--checkpoint", "30"]);
        let still = start("job start s --checkpoint 64spp -- render --recipe a.pitofx --out a.exr");
        assert_eq!(
            command_line(&still).unwrap()[2..],
            ["a.pitofx", "--out", "a.exr", "--checkpoint", "64spp"]
        );
        for bad in [
            "job start film --frames 2..9 -- film a.film.toml --out tmp/f",
            "job start film -- encode --manifest m.json",
            "job start film --frames 2..9 -- clip a.pitofx --range 0..3",
            "job start film --checkpoint 10 -- film a.film.toml --out tmp/f",
            "job start film --checkpoint 10 -- clip a.pitofx --checkpoint 5",
        ] {
            assert!(command_line(&start(bad)).is_err(), "{bad}");
        }
        for bad in [
            "job start film",
            "job start film clip a.pitofx",
            "job start film --frames 9..2 -- clip a",
            "job resume",
            "job resume film --all",
            "job",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(matches!(parse("job resume --all --json"), Ok(Verb::Resume(r)) if r.all && r.json));
        assert!(matches!(parse("job stop film --now"), Ok(Verb::Stop(s)) if s.now));
        assert!(matches!(parse("job status 4242 --watch"), Ok(Verb::Status(s)) if s.watch));
    }
}
