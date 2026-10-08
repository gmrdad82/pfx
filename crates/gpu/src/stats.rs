use std::collections::VecDeque;

use crate::PassTiming;

const DEFAULT_WINDOW: usize = 240;
const MAX_PASSES: usize = 16;
const MAX_PASS_NAME: usize = 24;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameStatsSnapshot {
    pub fps: f64,
    pub frame_ms: Percentiles,
    pub gpu_ms: Option<Percentiles>,
    pub interval_ms: Percentiles,
    pub size: Option<[u32; 2]>,
    pub renderer: Option<String>,
    pub gpu_pass_ms: Vec<(String, f64)>,
}

struct FrameSample {
    frame_ms: Option<f64>,
    interval_ms: Option<f64>,
    gpu_ms: Option<f64>,
    passes: Vec<(String, f64)>,
}

pub struct FrameStats {
    frames: VecDeque<FrameSample>,
    window: usize,
    size: Option<[u32; 2]>,
    renderer: Option<String>,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::with_window(DEFAULT_WINDOW)
    }
}

impl FrameStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_window(window: usize) -> Self {
        assert!(window > 0, "frame statistics window must be nonzero");
        Self {
            frames: VecDeque::with_capacity(window),
            window,
            size: None,
            renderer: None,
        }
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = Some([width, height]);
    }

    pub fn set_renderer(&mut self, renderer: &str) {
        self.renderer = match renderer {
            "live" | "trace" => Some(renderer.to_owned()),
            _ => None,
        };
    }

    pub fn record_frame(
        &mut self,
        frame_ms: f64,
        interval_ms: f64,
        gpu_ms: impl Into<Option<f64>>,
        passes: &[PassTiming],
    ) {
        let mut sample_passes: Vec<(String, f64)> = Vec::new();
        for pass in passes {
            let Some(milliseconds) = valid(pass.milliseconds) else {
                continue;
            };
            let name: String = pass.label.chars().take(MAX_PASS_NAME).collect();
            if name.is_empty() {
                continue;
            }
            if let Some((_, total)) = sample_passes.iter_mut().find(|(label, _)| *label == name) {
                if let Some(sum) = valid(*total + milliseconds) {
                    *total = sum;
                }
            } else {
                sample_passes.push((name, milliseconds));
            }
        }
        if self.frames.len() == self.window {
            self.frames.pop_front();
        }
        self.frames.push_back(FrameSample {
            frame_ms: valid(frame_ms),
            interval_ms: valid(interval_ms),
            gpu_ms: gpu_ms.into().and_then(valid),
            passes: sample_passes,
        });
    }

    pub fn snapshot(&self) -> FrameStatsSnapshot {
        let frame_ms = percentiles(self.frames.iter().filter_map(|frame| frame.frame_ms));
        let interval_ms = percentiles(self.frames.iter().filter_map(|frame| frame.interval_ms));
        let gpu_ms = {
            let values: Vec<_> = self
                .frames
                .iter()
                .filter_map(|frame| frame.gpu_ms)
                .collect();
            (!values.is_empty()).then(|| percentiles(values.into_iter()))
        };
        let mut interval_count = 0usize;
        let mut interval_total = 0.0;
        for interval in self
            .frames
            .iter()
            .filter_map(|frame| frame.interval_ms)
            .filter(|interval| *interval > 0.0)
        {
            interval_count += 1;
            interval_total += interval;
        }
        let fps = if interval_total > 0.0 {
            interval_count as f64 * 1000.0 / interval_total
        } else {
            0.0
        };
        let mut pass_means: Vec<(String, f64, usize)> = Vec::new();
        for frame in &self.frames {
            for (name, milliseconds) in &frame.passes {
                if let Some((_, mean, count)) =
                    pass_means.iter_mut().find(|(label, _, _)| label == name)
                {
                    *count += 1;
                    *mean += (milliseconds - *mean) / *count as f64;
                } else {
                    pass_means.push((name.clone(), *milliseconds, 1));
                }
            }
        }
        pass_means.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        pass_means.truncate(MAX_PASSES);
        FrameStatsSnapshot {
            fps,
            frame_ms,
            gpu_ms,
            interval_ms,
            size: self.size,
            renderer: self.renderer.clone(),
            gpu_pass_ms: pass_means
                .into_iter()
                .map(|(name, mean, _)| (name, mean))
                .collect(),
        }
    }
}

fn valid(value: f64) -> Option<f64> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn percentiles(values: impl Iterator<Item = f64>) -> Percentiles {
    let mut values: Vec<_> = values.collect();
    if values.is_empty() {
        return Percentiles::default();
    }
    values.sort_by(f64::total_cmp);
    let rank = |numerator: usize| (values.len() * numerator).div_ceil(100).saturating_sub(1);
    Percentiles {
        p50: values[rank(50)],
        p95: values[rank(95)],
        p99: values[rank(99)],
        max: *values.last().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(label: &str, milliseconds: f64) -> PassTiming {
        PassTiming {
            label: label.to_owned(),
            milliseconds,
        }
    }

    #[test]
    fn known_percentiles_and_fps() {
        let mut stats = FrameStats::new();
        stats.set_size(3840, 2160);
        stats.set_renderer("live");
        for number in 1..=100 {
            stats.record_frame(number as f64, 10.0, Some((101 - number) as f64), &[]);
        }
        let snapshot = stats.snapshot();
        let expected = Percentiles {
            p50: 50.0,
            p95: 95.0,
            p99: 99.0,
            max: 100.0,
        };
        assert_eq!(snapshot.frame_ms, expected);
        assert_eq!(snapshot.gpu_ms, Some(expected));
        assert_eq!(
            snapshot.interval_ms,
            Percentiles {
                p50: 10.0,
                p95: 10.0,
                p99: 10.0,
                max: 10.0,
            }
        );
        assert_eq!(snapshot.fps, 100.0);
        assert_eq!(snapshot.size, Some([3840, 2160]));
        assert_eq!(snapshot.renderer.as_deref(), Some("live"));
    }

    #[test]
    fn window_removes_old_samples_and_passes() {
        let mut stats = FrameStats::with_window(2);
        stats.record_frame(1.0, 10.0, 2.0, &[pass("old", 3.0)]);
        stats.record_frame(2.0, 20.0, 4.0, &[pass("new", 5.0)]);
        stats.record_frame(3.0, 30.0, 6.0, &[pass("new", 7.0)]);
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.frame_ms.max, 3.0);
        assert_eq!(snapshot.frame_ms.p50, 2.0);
        assert_eq!(snapshot.interval_ms.p50, 20.0);
        assert_eq!(snapshot.gpu_ms.unwrap().p50, 4.0);
        assert_eq!(snapshot.fps, 40.0);
        assert_eq!(snapshot.gpu_pass_ms, vec![("new".to_owned(), 6.0)]);
    }

    #[test]
    fn passes_are_capped_truncated_and_averaged() {
        let mut stats = FrameStats::new();
        let long_name = "abcdefghijklmnopqrstuvwx-extra";
        let mut passes = vec![pass(long_name, 2.0), pass(long_name, 3.0)];
        passes.extend((0..16).map(|index| pass(&format!("pass{index}"), index as f64)));
        stats.record_frame(1.0, 1.0, None, &passes);
        stats.record_frame(1.0, 1.0, None, &[pass("abcdefghijklmnopqrstuvwx", 7.0)]);
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.gpu_pass_ms.len(), 16);
        assert_eq!(snapshot.gpu_pass_ms[0], ("pass15".to_owned(), 15.0));
        assert_eq!(snapshot.gpu_pass_ms[15], ("pass1".to_owned(), 1.0));
        assert!(
            snapshot
                .gpu_pass_ms
                .contains(&("abcdefghijklmnopqrstuvwx".to_owned(), 6.0))
        );
        assert_eq!(snapshot.gpu_ms, None);
    }

    #[test]
    fn invalid_values_are_dropped_independently() {
        let mut stats = FrameStats::new();
        stats.record_frame(
            f64::NAN,
            f64::INFINITY,
            Some(-1.0),
            &[pass("bad", f64::NAN)],
        );
        stats.record_frame(4.0, 0.0, Some(5.0), &[pass("ok", 2.0)]);
        stats.record_frame(-1.0, 20.0, Some(f64::INFINITY), &[pass("ok", -1.0)]);
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.frame_ms.max, 4.0);
        assert_eq!(snapshot.gpu_ms.unwrap().max, 5.0);
        assert_eq!(snapshot.interval_ms.max, 20.0);
        assert_eq!(snapshot.fps, 50.0);
        assert_eq!(snapshot.gpu_pass_ms, vec![("ok".to_owned(), 2.0)]);
    }

    #[test]
    fn fps_uses_total_interval_not_average_instant_fps() {
        let mut stats = FrameStats::new();
        stats.record_frame(1.0, 10.0, None, &[]);
        stats.record_frame(1.0, 30.0, None, &[]);
        assert_eq!(stats.snapshot().fps, 50.0);
    }
}
