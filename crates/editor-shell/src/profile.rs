use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use pfx_gpu::FrameTimings;

pub const PROFILE_VAR: &str = "PFX_PROFILE";
const FLUSH_EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Ui,
    Viewport,
    Tessellate,
    Acquire,
    Paint,
    Present,
}

impl Phase {
    pub const ALL: [Phase; 6] = [
        Phase::Ui,
        Phase::Viewport,
        Phase::Tessellate,
        Phase::Acquire,
        Phase::Paint,
        Phase::Present,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Phase::Ui => "ui",
            Phase::Viewport => "viewport",
            Phase::Tessellate => "tessellate",
            Phase::Acquire => "acquire",
            Phase::Paint => "paint",
            Phase::Present => "present",
        }
    }
}

pub struct Profile {
    out: Option<BufWriter<File>>,
    began: Instant,
    mark: Instant,
    spans: [Duration; 6],
    gpu_frame: Option<u64>,
    ui_frame: Option<u64>,
    flushed: Instant,
}

impl Profile {
    pub fn off() -> Self {
        Self::at(None)
    }

    pub fn from_env() -> Self {
        let out = std::env::var_os(PROFILE_VAR)
            .filter(|path| !path.is_empty())
            .and_then(|path| match File::create(Path::new(&path)) {
                Ok(file) => Some(BufWriter::with_capacity(1 << 16, file)),
                Err(error) => {
                    eprintln!("{PROFILE_VAR}: cannot write {}: {error}", path.display());
                    None
                }
            });
        Self::at(out)
    }

    fn at(out: Option<BufWriter<File>>) -> Self {
        let now = Instant::now();
        Self {
            out,
            began: now,
            mark: now,
            spans: [Duration::ZERO; 6],
            gpu_frame: None,
            ui_frame: None,
            flushed: now,
        }
    }

    pub fn on(&self) -> bool {
        self.out.is_some()
    }

    pub fn start(&mut self) {
        if self.on() {
            self.mark = Instant::now();
            self.spans = [Duration::ZERO; 6];
        }
    }

    pub fn span(&mut self, phase: Phase) {
        if self.on() {
            let now = Instant::now();
            self.spans[phase as usize] += now - self.mark;
            self.mark = now;
        }
    }

    pub fn frame(
        &mut self,
        setting: &str,
        scale: Option<f32>,
        gpu: Option<&FrameTimings>,
        ui_gpu: Option<&FrameTimings>,
    ) {
        let Some(out) = self.out.as_mut() else {
            return;
        };
        let line = line(
            self.began.elapsed(),
            setting,
            scale,
            &self.spans,
            fresh(gpu, &mut self.gpu_frame),
            fresh(ui_gpu, &mut self.ui_frame),
        );
        let due = self.flushed.elapsed() >= FLUSH_EVERY;
        if due {
            self.flushed = Instant::now();
        }
        if out.write_all(line.as_bytes()).is_err() || (due && out.flush().is_err()) {
            self.out = None;
        }
    }

    pub fn finish(&mut self) {
        if let Some(out) = self.out.as_mut() {
            let _ = out.flush();
        }
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        self.finish();
    }
}

fn fresh<'a>(
    timings: Option<&'a FrameTimings>,
    seen: &mut Option<u64>,
) -> Option<&'a FrameTimings> {
    let timings = timings.filter(|timings| seen.is_none_or(|seen| timings.frame > seen))?;
    *seen = Some(timings.frame);
    Some(timings)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn label(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn line(
    at: Duration,
    setting: &str,
    scale: Option<f32>,
    spans: &[Duration; 6],
    gpu: Option<&FrameTimings>,
    ui_gpu: Option<&FrameTimings>,
) -> String {
    let mut out = format!("{{\"t\":{:.3},\"setting\":", ms(at));
    label(&mut out, setting);
    match scale {
        Some(scale) => {
            let _ = write!(out, ",\"scale\":{scale:.3}");
        }
        None => out.push_str(",\"scale\":null"),
    }
    let total: Duration = spans.iter().sum();
    let _ = write!(out, ",\"cpu\":{:.3}", ms(total));
    for phase in Phase::ALL {
        let _ = write!(
            out,
            ",\"{}\":{:.3}",
            phase.name(),
            ms(spans[phase as usize])
        );
    }
    if let Some(timings) = gpu {
        let gpu_ms: f64 = timings.passes.iter().map(|pass| pass.milliseconds).sum();
        let _ = write!(
            out,
            ",\"gpu_frame\":{},\"gpu\":{gpu_ms:.3},\"passes\":{{",
            timings.frame
        );
        for (index, pass) in timings.passes.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            label(&mut out, &pass.label);
            let _ = write!(out, ":{:.3}", pass.milliseconds);
        }
        out.push('}');
    }
    if let Some(timings) = ui_gpu {
        let ms: f64 = timings.passes.iter().map(|pass| pass.milliseconds).sum();
        let _ = write!(out, ",\"ui_gpu\":{ms:.3}");
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_gpu::PassTiming;

    #[test]
    fn a_profile_line_is_one_json_object_per_frame() {
        let mut spans = [Duration::ZERO; 6];
        spans[Phase::Viewport as usize] = Duration::from_micros(2500);
        spans[Phase::Paint as usize] = Duration::from_micros(500);
        let timings = FrameTimings {
            frame: 7,
            passes: vec![
                PassTiming {
                    label: "opaque \"PBR\"".into(),
                    milliseconds: 1.25,
                },
                PassTiming {
                    label: "taa".into(),
                    milliseconds: 0.5,
                },
            ],
        };
        let text = line(
            Duration::from_millis(1500),
            "vrr on, uncapped",
            Some(0.75),
            &spans,
            Some(&timings),
            None,
        );
        assert!(text.ends_with('\n'));
        let value: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(value["t"], 1500.0);
        assert_eq!(value["scale"], 0.75);
        assert_eq!(value["cpu"], 3.0);
        assert_eq!(value["viewport"], 2.5);
        assert_eq!(value["gpu_frame"], 7);
        assert_eq!(value["gpu"], 1.75);
        assert_eq!(value["passes"]["opaque \"PBR\""], 1.25);
        let bare = line(Duration::ZERO, "x", None, &spans, None, Some(&timings));
        let value: serde_json::Value = serde_json::from_str(bare.trim_end()).unwrap();
        assert!(value["scale"].is_null());
        assert!(value.get("passes").is_none());
        assert_eq!(value["ui_gpu"], 1.75);
    }
}
