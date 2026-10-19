use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use fs2::FileExt;

use super::process;
use super::progress;
use super::state::{self, Asked, Event, Job, Next, State};
use super::store::{self, Ask, Log};

pub const RETRY: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(500);
const SAMPLES_EVERY: Duration = Duration::from_secs(2);

pub trait Clock {
    fn now(&self) -> NaiveDateTime;
    fn sleep(&self, length: Duration);
}

pub struct Wall;

impl Clock for Wall {
    fn now(&self) -> NaiveDateTime {
        chrono::Local::now().naive_local()
    }

    fn sleep(&self, length: Duration) {
        std::thread::sleep(length);
    }
}

#[derive(Clone, Default)]
pub struct Signals {
    pub stop_now: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

impl Signals {
    pub fn install() -> Result<Signals, String> {
        let signals = Signals::default();
        #[cfg(unix)]
        {
            use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM, SIGUSR1, SIGUSR2};
            let register = |signal: i32, flag: &Arc<AtomicBool>| {
                signal_hook::flag::register(signal, Arc::clone(flag))
                    .map(|_| ())
                    .map_err(|e| format!("signals: {e}"))
            };
            for signal in [SIGTERM, SIGINT, SIGHUP] {
                register(signal, &signals.stop_now)?;
            }
            register(SIGUSR1, &signals.pause)?;
            register(SIGUSR2, &signals.stop)?;
        }
        Ok(signals)
    }
}

pub fn held(dir: &Path) -> bool {
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(store::LOCK))
    else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            false
        }
        Err(_) => true,
    }
}

enum Line {
    Out(String),
    Err(String),
    Closed,
}

enum Outcome {
    Again,
    Ended(State),
}

struct Supervisor<'a> {
    dir: PathBuf,
    program: &'a Path,
    clock: &'a dyn Clock,
    signals: Signals,
    log: Log,
    job: Job,
    now_stop: bool,
}

pub fn supervise(
    dir: &Path,
    program: &Path,
    clock: &dyn Clock,
    signals: Signals,
) -> Result<State, String> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(store::LOCK))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    lock.try_lock_exclusive()
        .map_err(|_| format!("{}: another supervisor runs this job", dir.display()))?;
    let job = store::read(dir)?;
    let mut supervisor = Supervisor {
        dir: dir.to_path_buf(),
        program,
        clock,
        signals,
        log: Log::open(dir),
        job,
        now_stop: false,
    };
    let ended = supervisor.run();
    if let Err(error) = &ended {
        supervisor.job.last_error = Some(error.clone());
        let _ = supervisor.end(State::Failed, error.clone());
    }
    drop(lock);
    ended
}

impl Supervisor<'_> {
    fn note(&mut self, line: &str) {
        let stamp = state::stamp(self.clock.now());
        self.log
            .line(&format!("pfx job {}: {stamp} {line}", self.job.name));
    }

    fn save(&mut self) -> Result<(), String> {
        self.job.updated = state::stamp(self.clock.now());
        store::write(&self.dir, &self.job)
    }

    fn take(&mut self) {
        let mut got = store::take_ask(&self.dir);
        if self.signals.stop_now.swap(false, Ordering::SeqCst) {
            got = Some(Ask::StopNow);
        } else if self.signals.stop.swap(false, Ordering::SeqCst) {
            got = Some(Ask::Stop);
        } else if self.signals.pause.swap(false, Ordering::SeqCst) {
            got = Some(Ask::Pause);
        }
        if let Some(ask) = got {
            self.now_stop |= ask == Ask::StopNow;
            if self.job.asked != ask.asked() {
                self.job.asked = ask.asked();
                let line = match self.job.asked {
                    Asked::Run => "asked to carry on",
                    Asked::Pause => "asked to pause at the next frame",
                    Asked::Stop if self.now_stop => "asked to stop now",
                    Asked::Stop => "asked to stop at the next frame",
                };
                self.note(line);
                let _ = self.save();
            }
        }
    }

    fn run(&mut self) -> Result<State, String> {
        if let Some(child) = self.job.child.take()
            && process::reap(&child)
        {
            self.note(&format!("ended pid {} left from the last run", child.pid));
        }
        self.job.supervisor = Some(process::me());
        self.job.reason = None;
        self.take();
        self.save()?;
        self.note(&format!("supervising pid {}", std::process::id()));
        loop {
            self.take();
            match self.job.next() {
                Next::End(state, why) => return self.end(state, why),
                Next::Render => {
                    if let Outcome::Ended(state) = self.render()? {
                        return Ok(state);
                    }
                }
            }
        }
    }

    fn wait_until(&mut self, at: NaiveDateTime) {
        loop {
            self.take();
            let now = self.clock.now();
            if self.job.asked != Asked::Run || now >= at {
                return;
            }
            let left = (at - now).to_std().unwrap_or(Duration::ZERO);
            self.clock.sleep(left.min(Duration::from_secs(1)));
        }
    }

    fn end(&mut self, state: State, why: String) -> Result<State, String> {
        self.job.state = state;
        self.job.child = None;
        self.job.samples = None;
        if state == State::Done {
            let report = format!("done: {}", self.job.summary());
            self.job.report = Some(report.clone());
            self.note(&report);
        } else {
            self.note(&format!("{}: {why}", state.name()));
        }
        self.job.reason = Some(why);
        if state.finished() {
            self.job.finished = Some(state::stamp(self.clock.now()));
        }
        self.save()?;
        Ok(state)
    }

    fn pending(&self) -> bool {
        self.now_stop || !matches!(self.job.next(), Next::Render)
    }

    fn render(&mut self) -> Result<Outcome, String> {
        self.job.runs += 1;
        self.job.state = State::Running;
        self.job.reason = None;
        let mut command = Command::new(self.program);
        command
            .args(&self.job.argv)
            .current_dir(&self.job.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        process::own_group(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", self.program.display()))?;
        let pid = child.id();
        self.job.child = Some(process::of(pid));
        self.note(&format!(
            "run {} started, pid {pid}: {}",
            self.job.runs,
            self.job.argv.join(" ")
        ));
        self.save()?;
        let (send, lines) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            let send = send.clone();
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = send.send(Line::Out(line));
                }
                let _ = send.send(Line::Closed);
            });
        }
        if let Some(err) = child.stderr.take() {
            let send = send.clone();
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                    let _ = send.send(Line::Err(line));
                }
                let _ = send.send(Line::Closed);
            });
        }
        drop(send);
        let mut closed = 0;
        let mut stopping = false;
        let mut mark = Instant::now();
        let mut wrote_samples = Instant::now();
        let mut said: Option<String> = None;
        while closed < 2 {
            let event = match lines.recv_timeout(POLL) {
                Ok(Line::Out(line)) => progress::out_line(&line),
                Ok(Line::Err(line)) => {
                    self.log.line(&line);
                    if progress::error_line(&line) {
                        said = Some(line.trim().to_string());
                    }
                    progress::err_line(&line)
                }
                Ok(Line::Closed) => {
                    closed += 1;
                    None
                }
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            self.take();
            let mut boundary = false;
            if let Some(event) = event {
                let seconds = matches!(event, Event::Frame).then(|| mark.elapsed().as_secs_f64());
                if matches!(event, Event::Frame | Event::Total { .. }) {
                    mark = Instant::now();
                }
                boundary = self.job.apply(&event, seconds);
                let samples = matches!(event, Event::Samples { .. });
                if !samples || wrote_samples.elapsed() >= SAMPLES_EVERY {
                    wrote_samples = Instant::now();
                    self.save()?;
                }
            }
            if !stopping && (self.now_stop || boundary && self.pending()) {
                stopping = true;
                self.note(if self.now_stop {
                    "ending the run now"
                } else {
                    "ending the run at a frame boundary"
                });
                process::end_group(pid, false);
            }
        }
        let status = child.wait().map_err(|e| format!("pid {pid}: {e}"))?;
        self.job.child = None;
        self.job.samples = None;
        if stopping {
            self.now_stop = false;
            self.note(&format!("run {} ended ({status})", self.job.runs));
            self.save()?;
            return Ok(Outcome::Again);
        }
        let error = if status.success() {
            match self.job.left() {
                Some(left) if left > 0 => Some(format!("exited with {left} frames left")),
                _ => None,
            }
        } else {
            Some(said.unwrap_or_else(|| format!("exited with {status}")))
        };
        let Some(error) = error else {
            return self.end(State::Done, "finished".into()).map(Outcome::Ended);
        };
        self.note(&format!("run {} failed: {error}", self.job.runs));
        let at = self
            .job
            .frame
            .map_or("the run".to_string(), |frame| format!("frame {frame}"));
        if self.job.failed_once(error.clone()) {
            let why = format!("{at} failed {} times: {error}", super::state::TRIES);
            return self.end(State::Failed, why).map(Outcome::Ended);
        }
        self.job.state = State::Waiting;
        self.job.reason = Some(format!("{at} failed, trying again"));
        self.save()?;
        let after = self.clock.now() + chrono::Duration::from_std(RETRY).unwrap_or_default();
        self.wait_until(after);
        Ok(Outcome::Again)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake(Mutex<NaiveDateTime>);

    impl Clock for Fake {
        fn now(&self) -> NaiveDateTime {
            *self.0.lock().unwrap()
        }

        fn sleep(&self, length: Duration) {
            let mut now = self.0.lock().unwrap();
            *now += chrono::Duration::from_std(length).unwrap();
        }
    }

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    const CLIP: &str = r#"out=$1; pace=$2; total=4; todo=0
for i in 0 1 2 3; do [ -f "$out/frame-000$i.png" ] || todo=$((todo+1)); done
echo "pfx clip: $todo of $total frames to make, $((total-todo)) already finished in $out" >&2
for i in 0 1 2 3; do f="$out/frame-000$i.png"; [ -f "$f" ] && continue; sleep "$pace"; [ -f "$out/fail" ] && { echo "pfx clip: the device was lost" >&2; exit 1; }; touch "$f"; echo "$f"; done"#;

    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/jobs-supervise")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, out) = (base.join("job"), base.join("frames"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        (dir, out)
    }

    fn job(dir: &Path, out: &Path, pace: &str, now: NaiveDateTime) {
        let argv = vec![
            "-c".to_string(),
            CLIP.to_string(),
            "clip".to_string(),
            out.display().to_string(),
            pace.to_string(),
        ];
        let job = Job::new("t", argv, out.to_path_buf(), None, now);
        store::write(dir, &job).unwrap();
    }

    fn frames(out: &Path) -> usize {
        (0..4)
            .filter(|i| out.join(format!("frame-000{i}.png")).exists())
            .count()
    }

    #[test]
    fn a_job_renders_every_frame_reports_and_lets_go_of_its_lock() {
        let (dir, out) = scratch("all");
        let clock = Fake(Mutex::new(at("2026-10-19 12:00")));
        job(&dir, &out, "0", clock.now());
        let ended = supervise(&dir, Path::new("/bin/sh"), &clock, Signals::default()).unwrap();
        assert_eq!(ended, State::Done);
        let job = store::read(&dir).unwrap();
        assert_eq!(job.state, State::Done);
        assert_eq!(job.frames.done, 4);
        assert_eq!(job.frames.total, Some(4));
        assert_eq!(job.frame_seconds.len(), 4);
        assert!(
            job.report
                .as_deref()
                .is_some_and(|r| r.starts_with("done: 4 of 4 frames"))
        );
        assert!(job.finished.is_some() && job.child.is_none());
        assert_eq!(frames(&out), 4);
        let log = store::log_lines(&dir).join("\n");
        assert!(log.contains("pfx clip: 4 of 4 frames to make"), "{log}");
        assert!(!held(&dir));
    }

    #[test]
    fn a_pause_lands_on_a_frame_boundary_and_a_second_run_carries_on() {
        let (dir, out) = scratch("pause");
        let clock = Wall;
        job(&dir, &out, "0.3", clock.now());
        let asker = {
            let (dir, out) = (dir.clone(), out.clone());
            std::thread::spawn(move || {
                while frames(&out) == 0 {
                    std::thread::sleep(Duration::from_millis(20));
                }
                store::ask(&dir, Ask::Pause).unwrap();
            })
        };
        let ended = supervise(&dir, Path::new("/bin/sh"), &clock, Signals::default()).unwrap();
        asker.join().unwrap();
        assert_eq!(ended, State::Paused);
        let paused = store::read(&dir).unwrap();
        let made = frames(&out);
        assert!((1..4).contains(&made), "{made}");
        assert_eq!(paused.frames.done as usize, made);
        assert_eq!(paused.asked, Asked::Pause);
        let mut resumed = paused;
        resumed.asked = Asked::Run;
        store::write(&dir, &resumed).unwrap();
        assert_eq!(
            supervise(&dir, Path::new("/bin/sh"), &clock, Signals::default()).unwrap(),
            State::Done
        );
        assert_eq!(frames(&out), 4);
        assert_eq!(store::read(&dir).unwrap().runs, 2);
    }

    #[test]
    fn a_frame_that_fails_three_times_fails_the_job() {
        let (dir, out) = scratch("fail");
        std::fs::write(out.join("fail"), "").unwrap();
        let clock = Fake(Mutex::new(at("2026-10-19 12:00")));
        job(&dir, &out, "0", clock.now());
        let ended = supervise(&dir, Path::new("/bin/sh"), &clock, Signals::default()).unwrap();
        assert_eq!(ended, State::Failed);
        let job = store::read(&dir).unwrap();
        assert_eq!(job.runs, 3);
        assert_eq!(job.fails.get("0"), Some(&3));
        assert_eq!(
            job.last_error.as_deref(),
            Some("pfx clip: the device was lost")
        );
        assert_eq!(
            job.reason.as_deref(),
            Some("frame 0 failed 3 times: pfx clip: the device was lost")
        );
    }

    #[test]
    fn stop_now_ends_the_run_mid_frame_and_one_supervisor_holds_a_job() {
        let (dir, out) = scratch("now");
        let clock = Wall;
        job(&dir, &out, "30", clock.now());
        let signals = Signals::default();
        let flag = Arc::clone(&signals.stop_now);
        let watcher = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let asked = Instant::now();
                while store::read(&dir).map_or(true, |job| job.child.is_none()) {
                    std::thread::sleep(Duration::from_millis(20));
                    assert!(asked.elapsed() < Duration::from_secs(20));
                }
                assert!(held(&dir));
                assert!(supervise(&dir, Path::new("/bin/sh"), &Wall, Signals::default()).is_err());
                flag.store(true, Ordering::SeqCst);
            })
        };
        let asked = Instant::now();
        let ended = supervise(&dir, Path::new("/bin/sh"), &clock, signals).unwrap();
        watcher.join().unwrap();
        assert_eq!(ended, State::Stopped);
        assert!(asked.elapsed() < Duration::from_secs(20));
        assert_eq!(frames(&out), 0);
        assert_eq!(store::read(&dir).unwrap().asked, Asked::Stop);
    }
}
