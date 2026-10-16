use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

pub const BAR: usize = 24;
const EVERY: Duration = Duration::from_millis(100);

pub fn live() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    !set("NO_COLOR")
        && std::io::stderr().is_terminal()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

fn clock(seconds: f64) -> String {
    let whole = seconds.ceil() as u64;
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    match (h, m) {
        (0, 0) => format!("{s}s"),
        (0, _) => format!("{m}:{s:02}"),
        _ => format!("{h}:{m:02}:{s:02}"),
    }
}

pub fn line(label: &str, done: u64, total: u64, fps: f64) -> String {
    let done = done.min(total);
    let filled = if total == 0 {
        BAR
    } else {
        (done as u128 * BAR as u128 / total as u128) as usize
    };
    let bar = format!("{}{}", "#".repeat(filled), "-".repeat(BAR - filled));
    let eta = if done >= total {
        "done".to_string()
    } else if fps > 0.0 && fps.is_finite() {
        format!("ETA {}", clock((total - done) as f64 / fps))
    } else {
        "ETA --".to_string()
    };
    format!("{label} [{bar}] frame {done}/{total}  {fps:.1} fps  {eta}")
}

pub struct Progress {
    label: &'static str,
    total: u64,
    first: Option<(Instant, u64)>,
    drawn: Option<Instant>,
    width: usize,
    live: bool,
}

impl Progress {
    pub fn new(label: &'static str, total: u64, live: bool) -> Progress {
        Progress {
            label,
            total,
            first: None,
            drawn: None,
            width: 0,
            live,
        }
    }

    pub fn shows(&self) -> bool {
        self.live
    }

    pub fn frame(&mut self, done: u64) {
        if !self.live {
            return;
        }
        let now = Instant::now();
        if done < self.total && self.drawn.is_some_and(|drawn| now - drawn < EVERY) {
            return;
        }
        self.drawn = Some(now);
        let (started, base) = *self.first.get_or_insert((now, done));
        let seconds = (now - started).as_secs_f64();
        let fps = if seconds > 0.0 {
            done.saturating_sub(base) as f64 / seconds
        } else {
            0.0
        };
        let text = line(self.label, done, self.total, fps);
        let pad = self.width.saturating_sub(text.len());
        self.width = text.len();
        let mut err = std::io::stderr().lock();
        let _ = write!(err, "\r{text}{}", " ".repeat(pad));
        let _ = err.flush();
    }

    pub fn clear(&mut self) {
        if self.width == 0 {
            return;
        }
        let mut err = std::io::stderr().lock();
        let _ = write!(err, "\r{}\r", " ".repeat(self.width));
        let _ = err.flush();
        self.width = 0;
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_progress_line_shows_the_frame_the_rate_the_eta_and_a_bar() {
        assert_eq!(
            line("pfx shot", 0, 120, 0.0),
            "pfx shot [------------------------] frame 0/120  0.0 fps  ETA --"
        );
        assert_eq!(
            line("pfx shot", 30, 120, 30.0),
            "pfx shot [######------------------] frame 30/120  30.0 fps  ETA 3s"
        );
        assert_eq!(
            line("pfx film", 60, 7260, 12.4),
            "pfx film [------------------------] frame 60/7260  12.4 fps  ETA 9:41"
        );
        assert_eq!(
            line("pfx bench", 1, 90001, 2.0),
            "pfx bench [------------------------] frame 1/90001  2.0 fps  ETA 12:30:00"
        );
        assert_eq!(
            line("pfx shot", 120, 120, 41.66),
            "pfx shot [########################] frame 120/120  41.7 fps  done"
        );
        assert_eq!(
            line("pfx shot", 0, 0, 0.0),
            "pfx shot [########################] frame 0/0  0.0 fps  done"
        );
    }
}
