use std::collections::VecDeque;

use crate::FrameTimings;

const SMOOTHING: f64 = 0.25;
const PENDING: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    pub gpu_ms: f64,
    pub headroom: f64,
    pub min_scale: f32,
    pub max_scale: f32,
    pub step: f32,
    pub settle_frames: u32,
}

impl Budget {
    pub fn new(gpu_ms: f64) -> Self {
        Self {
            gpu_ms,
            headroom: 0.9,
            min_scale: 0.5,
            max_scale: 1.0,
            step: 0.05,
            settle_frames: 6,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(self.gpu_ms.is_finite() && self.gpu_ms > 0.0) {
            return Err("the GPU budget must be a positive number of milliseconds".into());
        }
        if !(self.headroom > 0.0 && self.headroom <= 1.0) {
            return Err("the headroom must be above 0 and at most 1".into());
        }
        if !(self.min_scale > 0.0 && self.min_scale <= self.max_scale && self.max_scale <= 2.0) {
            return Err("the scales must satisfy 0 < min <= max <= 2".into());
        }
        if !(self.step > 0.0 && self.step <= 0.5) {
            return Err("the step must be above 0 and at most 0.5".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct DynamicResolution {
    budget: Budget,
    scale: f32,
    cost: Option<f64>,
    pending: VecDeque<(u64, f32)>,
    since_change: u32,
    changes: u32,
}

impl DynamicResolution {
    pub fn new(budget: Budget) -> Result<Self, String> {
        budget.validate()?;
        Ok(Self {
            budget,
            scale: budget.max_scale,
            cost: None,
            pending: VecDeque::with_capacity(PENDING),
            since_change: 0,
            changes: 0,
        })
    }

    pub fn budget(&self) -> Budget {
        self.budget
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn changes(&self) -> u32 {
        self.changes
    }

    pub fn cost_ms_at_full(&self) -> Option<f64> {
        self.cost
    }

    pub fn begin(&mut self, frame: u64) -> f32 {
        self.begin_at(frame, self.scale)
    }

    pub fn begin_at(&mut self, frame: u64, rendered_scale: f32) -> f32 {
        if self.pending.len() == PENDING {
            self.pending.pop_front();
        }
        let rendered = if rendered_scale.is_finite() && rendered_scale > 0.0 {
            rendered_scale
        } else {
            self.scale
        };
        self.pending.push_back((frame, rendered));
        self.since_change = self.since_change.saturating_add(1);
        self.scale
    }

    pub fn render_size(&self, content: [u32; 2]) -> Option<[u32; 2]> {
        scaled(content, self.scale)
    }

    pub fn capacity(&self, content: [u32; 2]) -> Option<[u32; 2]> {
        scaled(content, self.budget.max_scale)
    }

    pub fn observe_timings(&mut self, timings: &FrameTimings) -> Option<f32> {
        timings
            .gpu_ms()
            .and_then(|gpu_ms| self.observe(timings.frame, gpu_ms))
    }

    pub fn observe(&mut self, frame: u64, gpu_ms: f64) -> Option<f32> {
        if !(gpu_ms.is_finite() && gpu_ms > 0.0) {
            return None;
        }
        let at = self
            .pending
            .iter()
            .position(|(pending, _)| *pending == frame)?;
        let (_, scale) = self.pending[at];
        self.pending.drain(..=at);
        let area = f64::from(scale) * f64::from(scale);
        let cost = gpu_ms / area;
        let over = gpu_ms > self.budget.gpu_ms && scale <= self.scale;
        let smoothed = match self.cost {
            Some(previous) if over => previous.max(cost),
            Some(previous) => previous + (cost - previous) * SMOOTHING,
            None => cost,
        };
        self.cost = Some(smoothed);
        let ideal = self.quantize((self.budget.gpu_ms * self.budget.headroom / smoothed).sqrt());
        let settled = self.since_change >= self.budget.settle_frames;
        let predicted = smoothed * f64::from(self.scale) * f64::from(self.scale);
        let lower = ideal < self.scale && (over || (settled && predicted > self.budget.gpu_ms));
        let higher = ideal >= self.scale + self.budget.step * 0.999 && settled;
        if !(lower || higher) {
            return None;
        }
        let next = ideal;
        self.scale = next;
        self.since_change = 0;
        self.changes += 1;
        Some(next)
    }

    fn quantize(&self, ideal: f64) -> f32 {
        let budget = self.budget;
        let steps = ((ideal - f64::from(budget.min_scale)) / f64::from(budget.step)).floor();
        let snapped = f64::from(budget.min_scale) + steps.max(0.0) * f64::from(budget.step);
        let snapped = (snapped * 1e4).round() / 1e4;
        (snapped as f32).clamp(budget.min_scale, budget.max_scale)
    }
}

fn scaled(content: [u32; 2], factor: f32) -> Option<[u32; 2]> {
    let size = crate::window::render_size(
        crate::window::Size {
            width: content[0],
            height: content[1],
        },
        crate::window::RenderScale::Factor(factor),
    )?;
    Some([size.width, size.height])
}
