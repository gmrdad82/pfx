use std::collections::BTreeMap;
use std::path::PathBuf;

pub use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

pub const FORMAT: u32 = 1;
pub const TRIES: u32 = 3;
const KEPT_TIMES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Paused,
    Waiting,
    Done,
    Failed,
    Stopped,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Running => "running",
            State::Paused => "paused",
            State::Waiting => "waiting",
            State::Done => "done",
            State::Failed => "failed",
            State::Stopped => "stopped",
        }
    }

    pub fn finished(self) -> bool {
        matches!(self, State::Done | State::Failed)
    }

    pub fn live(self) -> bool {
        matches!(self, State::Running | State::Waiting)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Asked {
    #[default]
    Run,
    Pause,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proc {
    pub pid: u32,
    pub start: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Frames {
    pub done: u32,
    pub total: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Samples {
    pub done: u32,
    pub of: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub frame: u32,
    pub same: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Resumed {
    pub verified: u32,
    pub redone: Vec<u32>,
    pub check: Option<Check>,
}

impl Resumed {
    pub fn line(&self) -> String {
        let redone = match self.redone.as_slice() {
            [] => "none redone".to_string(),
            frames => format!(
                "redone {}",
                frames
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let check = match self.check {
            None => String::new(),
            Some(Check { frame, same: None }) => format!(", frame {frame} rendered again"),
            Some(Check {
                frame,
                same: Some(true),
            }) => format!(", frame {frame} rendered again to its stored bytes"),
            Some(Check {
                frame,
                same: Some(false),
            }) => format!(", frame {frame} rendered again differs"),
        };
        format!("{} frames verified, {redone}{check}", self.verified)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub format: u32,
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub state: State,
    #[serde(default)]
    pub asked: Asked,
    pub reason: Option<String>,
    pub supervisor: Option<Proc>,
    pub child: Option<Proc>,
    #[serde(default)]
    pub frames: Frames,
    pub frame: Option<u32>,
    pub samples: Option<Samples>,
    #[serde(default)]
    pub frame_seconds: Vec<f64>,
    pub median_frame_seconds: Option<f64>,
    pub eta_seconds: Option<f64>,
    pub range: Option<[u32; 2]>,
    #[serde(default)]
    pub resume: Option<Resumed>,
    #[serde(default)]
    pub fails: BTreeMap<String, u32>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub runs: u32,
    pub created: String,
    pub updated: String,
    pub finished: Option<String>,
    pub report: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    Render,
    End(State, String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Total {
        done: u32,
        total: u32,
    },
    Frame,
    Samples {
        frame: Option<u32>,
        done: u32,
        of: Option<u32>,
    },
    Checkpoint,
    Resumed {
        verified: u32,
        redone: Vec<u32>,
        check: Option<u32>,
    },
    Checked {
        frame: u32,
        same: bool,
    },
}

impl Job {
    pub fn new(
        name: &str,
        argv: Vec<String>,
        cwd: PathBuf,
        range: Option<[u32; 2]>,
        now: NaiveDateTime,
    ) -> Job {
        let stamp = stamp(now);
        Job {
            format: FORMAT,
            name: name.to_string(),
            argv,
            cwd,
            state: State::Waiting,
            asked: Asked::Run,
            reason: Some("starting".into()),
            supervisor: None,
            child: None,
            frames: Frames::default(),
            frame: None,
            samples: None,
            frame_seconds: Vec::new(),
            median_frame_seconds: None,
            eta_seconds: None,
            range,
            resume: None,
            fails: BTreeMap::new(),
            last_error: None,
            runs: 0,
            created: stamp.clone(),
            updated: stamp,
            finished: None,
            report: None,
        }
    }

    pub fn next(&self) -> Next {
        match self.asked {
            Asked::Stop => Next::End(State::Stopped, "stopped when asked".into()),
            Asked::Pause => Next::End(State::Paused, "paused when asked".into()),
            Asked::Run => Next::Render,
        }
    }

    pub fn left(&self) -> Option<u32> {
        self.frames
            .total
            .map(|total| total.saturating_sub(self.frames.done))
    }

    pub fn apply(&mut self, event: &Event, seconds: Option<f64>) -> bool {
        match *event {
            Event::Total { done, total } => {
                self.frames = Frames {
                    done: done.min(total),
                    total: Some(total),
                };
                self.frame = (done < total).then_some(done);
                self.estimate();
                false
            }
            Event::Frame => {
                if let Some(frame) = self.frame {
                    self.fails.remove(&frame.to_string());
                }
                self.frames.done = match self.frames.total {
                    Some(total) => (self.frames.done + 1).min(total),
                    None => self.frames.done + 1,
                };
                self.frame = match self.frames.total {
                    Some(total) if self.frames.done >= total => None,
                    _ => Some(self.frames.done),
                };
                self.samples = None;
                if let Some(seconds) = seconds.filter(|s| s.is_finite() && *s >= 0.0) {
                    self.frame_seconds.push(seconds);
                    let over = self.frame_seconds.len().saturating_sub(KEPT_TIMES);
                    self.frame_seconds.drain(..over);
                }
                self.estimate();
                true
            }
            Event::Samples { frame, done, of } => {
                if frame.is_some() {
                    self.frame = frame;
                }
                self.samples = Some(Samples { done, of });
                false
            }
            Event::Checkpoint => true,
            Event::Resumed {
                verified,
                ref redone,
                check,
            } => {
                self.resume = Some(Resumed {
                    verified,
                    redone: redone.clone(),
                    check: check.map(|frame| Check { frame, same: None }),
                });
                false
            }
            Event::Checked { frame, same } => {
                let resume = self.resume.get_or_insert_with(Resumed::default);
                resume.check = Some(Check {
                    frame,
                    same: Some(same),
                });
                false
            }
        }
    }

    fn estimate(&mut self) {
        self.median_frame_seconds = median(&self.frame_seconds);
        self.eta_seconds = match (self.median_frame_seconds, self.left()) {
            (Some(median), Some(left)) => Some(median * f64::from(left)),
            (_, Some(0)) => Some(0.0),
            _ => None,
        };
    }

    pub fn failed_once(&mut self, error: String) -> bool {
        let key = self
            .frame
            .map_or_else(|| "run".to_string(), |frame| frame.to_string());
        let count = self.fails.entry(key).or_insert(0);
        *count += 1;
        let out = *count >= TRIES;
        self.last_error = Some(error);
        out
    }

    pub fn summary(&self) -> String {
        let frames = match self.frames.total {
            Some(total) => format!("{} of {total} frames", self.frames.done),
            None => format!("{} frames", self.frames.done),
        };
        let median = self.median_frame_seconds.map_or(String::new(), |median| {
            format!(", median frame {}", duration(median))
        });
        let runs = match self.runs {
            1 => "1 run".to_string(),
            n => format!("{n} runs"),
        };
        format!("{frames}{median}, {runs}")
    }
}

pub fn stamp(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%S").to_string()
}

pub fn range(text: &str) -> Result<[u32; 2], String> {
    let shape = || format!("{text}: not a frame range A..B, both ends kept, as 0..119");
    let (first, last) = text.trim().split_once("..").ok_or_else(shape)?;
    let first: u32 = first.trim().parse().map_err(|_| shape())?;
    let last: u32 = last.trim().parse().map_err(|_| shape())?;
    if last < first {
        return Err(format!("{text}: the range ends before it starts"));
    }
    Ok([first, last])
}

pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    })
}

pub fn duration(seconds: f64) -> String {
    let whole = seconds.max(0.0).round() as u64;
    match whole {
        0..60 => format!("{:.1} s", seconds.max(0.0)),
        60..3600 => format!("{} min {} s", whole / 60, whole % 60),
        _ => format!("{} h {} min", whole / 3600, whole % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    fn job(range: Option<[u32; 2]>) -> Job {
        Job::new(
            "film",
            vec!["clip".into(), "a.pitofx".into()],
            PathBuf::from("/work"),
            range,
            at("2026-10-19 12:00"),
        )
    }

    #[test]
    fn job_json_keeps_every_field_through_a_round_trip_and_names_states_plainly() {
        let mut job = job(Some([40, 79]));
        job.apply(&Event::Total { done: 2, total: 10 }, None);
        job.apply(&Event::Frame, Some(30.0));
        job.supervisor = Some(Proc {
            pid: 42,
            start: Some("boot:9".into()),
        });
        let text = serde_json::to_string_pretty(&job).unwrap();
        let back: Job = serde_json::from_str(&text).unwrap();
        assert_eq!(back, job);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["state"], "waiting");
        assert_eq!(value["asked"], "run");
        assert_eq!(value["range"], serde_json::json!([40, 79]));
        assert_eq!(value["created"], "2026-10-19T12:00:00");
        assert_eq!(value["frames"]["done"], 3);
        assert_eq!(value["frames"]["total"], 10);
        assert_eq!(value["frame"], 3);
        assert_eq!(value["eta_seconds"], 210.0);
        assert_eq!(value["supervisor"]["pid"], 42);
        assert_eq!(value["supervisor"]["start"], "boot:9");
    }

    #[test]
    fn frames_count_up_the_eta_follows_the_median_and_a_frame_fails_the_job_on_its_third_try() {
        let mut job = job(None);
        job.apply(&Event::Total { done: 0, total: 4 }, None);
        assert_eq!(job.frame, Some(0));
        for seconds in [100.0, 10.0, 12.0] {
            assert!(job.apply(&Event::Frame, Some(seconds)));
        }
        assert_eq!(job.median_frame_seconds, Some(12.0));
        assert_eq!(job.eta_seconds, Some(12.0));
        assert!(!job.apply(
            &Event::Samples {
                frame: None,
                done: 8,
                of: Some(64)
            },
            None
        ));
        assert_eq!(
            job.samples,
            Some(Samples {
                done: 8,
                of: Some(64)
            })
        );
        assert!(!job.failed_once("lost the device".into()));
        assert!(!job.failed_once("lost the device".into()));
        assert_eq!(job.fails.get("3"), Some(&2));
        assert!(job.failed_once("lost the device".into()));
        job.fails.clear();
        assert!(!job.failed_once("x".into()));
        job.apply(&Event::Frame, Some(12.0));
        assert!(job.fails.is_empty());
        assert_eq!(job.frame, None);
        assert_eq!(job.left(), Some(0));
        assert_eq!(job.eta_seconds, Some(0.0));
        job.apply(&Event::Frame, None);
        assert_eq!(job.frames.done, 4);
        assert_eq!(median(&[1.0, 2.0, 3.0, 10.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn a_job_renders_until_asked_to_pause_or_stop() {
        let mut job = job(None);
        assert_eq!(job.next(), Next::Render);
        job.asked = Asked::Pause;
        assert!(matches!(job.next(), Next::End(State::Paused, _)));
        job.asked = Asked::Stop;
        assert!(matches!(job.next(), Next::End(State::Stopped, _)));
        assert!(State::Done.finished() && State::Failed.finished());
        assert!(State::Running.live() && State::Waiting.live() && !State::Paused.live());
    }

    #[test]
    fn a_frame_range_keeps_both_ends() {
        assert_eq!(range("0..119"), Ok([0, 119]));
        assert_eq!(range(" 40..40 "), Ok([40, 40]));
        for bad in ["40", "a..b", "10..2", "-1..4", "1...4", "..4"] {
            assert!(range(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_resume_records_what_it_verified_redid_and_rendered_again() {
        let mut job = job(None);
        let resumed = Event::Resumed {
            verified: 4,
            redone: vec![2],
            check: Some(3),
        };
        assert!(!job.apply(&resumed, None));
        assert_eq!(
            job.resume.as_ref().unwrap().line(),
            "4 frames verified, redone 2, frame 3 rendered again"
        );
        job.apply(
            &Event::Checked {
                frame: 3,
                same: false,
            },
            None,
        );
        assert_eq!(
            job.resume.as_ref().unwrap().line(),
            "4 frames verified, redone 2, frame 3 rendered again differs"
        );
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(
            value["resume"],
            serde_json::json!({"verified": 4, "redone": [2], "check": {"frame": 3, "same": false}})
        );
        let older: Job = serde_json::from_value({
            let mut value = value.clone();
            value.as_object_mut().unwrap().remove("resume");
            value
        })
        .unwrap();
        assert_eq!(older.resume, None);
    }
}
