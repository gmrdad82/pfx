use std::time::Instant;

use super::*;
use pfx_editor_shell::Frames;
use pfx_live::hud::{self, Pace, Readout, Shown};

pub(super) const HINT: &str = "f3 shows the frame stats: fps, frame intervals, GPU time per pass";

#[derive(Clone, Debug, Default)]
pub(super) struct Stats {
    frames: Option<Frames>,
    drawn: Option<[u64; 2]>,
    hud_ms: Option<f64>,
    hud_shown: Option<f64>,
}

impl Stats {
    pub(super) fn clear(&mut self) {
        self.frames = None;
        self.hud_ms = None;
        self.hud_shown = None;
    }
}

fn rate(name: &str) -> String {
    match name {
        "cap60" => "60 fps".into(),
        "cap120" => "120 fps".into(),
        other => other.into(),
    }
}

pub fn readout(
    present: pfx_editor_shell::Present,
    frames: Option<&Frames>,
    drawn: Option<[u64; 2]>,
    hud_ms: Option<f64>,
) -> Readout {
    let shown = Shown {
        mode: frames.map_or_else(String::new, |frames| frames.present_mode.clone()),
        vrr: present.vrr,
        vrr_in_effect: frames.and_then(|frames| frames.vrr_in_effect),
        frame_rate: rate(present.frame_rate.name()),
    };
    let Some(frames) = frames else {
        return Readout {
            drawn,
            hud_ms,
            present: Some(shown),
            ..Readout::default()
        };
    };
    let snapshot = &frames.snapshot;
    let interval = snapshot.interval_ms;
    Readout {
        pace: (snapshot.fps > 0.0).then_some(Pace {
            fps: snapshot.fps,
            p50_ms: interval.p50,
            p99_ms: interval.p99,
            max_ms: interval.max,
        }),
        shown_pace: frames.shown.map(|(fps, spread)| Pace {
            fps,
            p50_ms: spread.p50,
            p99_ms: spread.p99,
            max_ms: spread.max,
        }),
        cpu_ms: Some(snapshot.frame_ms.p50),
        gpu: snapshot.gpu_ms.map(|gpu| (0, gpu.p50)),
        passes: snapshot.gpu_pass_ms.clone(),
        scale: frames.render_scale,
        present: Some(shown),
        drawn,
        hud_ms,
        ..Readout::default()
    }
}

impl Shell {
    pub fn stats(&self) -> bool {
        self.preferences.stats
    }

    pub fn set_stats(&mut self, on: bool) {
        self.set_preferences(pfx_settings::Preferences {
            stats: on,
            ..self.preferences
        });
    }

    pub(super) fn toggle_stats(&mut self) {
        self.set_stats(!self.stats());
    }

    pub fn set_frames(&mut self, frames: &Frames) {
        self.stats.frames = Some(frames.clone());
        self.stats.hud_shown = self.stats.hud_ms;
    }

    pub fn set_drawn(&mut self, drawn: Option<[u64; 2]>) {
        self.stats.drawn = drawn;
    }

    pub(super) fn stats_overlay(&mut self, ctx: &Context, image: Rect) {
        if !self.stats() || self.full {
            return;
        }
        let started = Instant::now();
        let shown = readout(
            self.present(),
            self.stats.frames.as_ref(),
            self.stats.drawn,
            self.stats.hud_shown,
        );
        hud::ui_at(
            ctx,
            &shown,
            image.right_top() + Vec2::new(-SECTION_GAP * 0.5, SECTION_GAP * 0.5),
            Align2::RIGHT_TOP,
        );
        self.stats.hud_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
    }
}
