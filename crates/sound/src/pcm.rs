use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::mix::lock;

pub const PCM_EDGE_SECONDS: f32 = 0.002;

struct Inner {
    samples: VecDeque<f32>,
    closed: bool,
}

pub(crate) struct PcmRing {
    inner: Mutex<Inner>,
    capacity: usize,
    channels: usize,
}

impl PcmRing {
    fn new(channels: usize, capacity_frames: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                samples: VecDeque::new(),
                closed: false,
            }),
            capacity: capacity_frames.max(1),
            channels: channels.max(1),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock(&self.inner)
    }
}

pub struct PcmSender {
    ring: Arc<PcmRing>,
    rate: u32,
}

impl PcmSender {
    pub fn push(&self, samples: &[f32]) -> usize {
        let channels = self.ring.channels;
        let mut inner = self.ring.lock();
        let queued = inner.samples.len() / channels;
        let room = self.ring.capacity.saturating_sub(queued);
        let frames = (samples.len() / channels).min(room);
        inner.samples.extend(&samples[..frames * channels]);
        frames
    }

    pub fn queued_frames(&self) -> usize {
        self.ring.lock().samples.len() / self.ring.channels
    }

    pub fn capacity_frames(&self) -> usize {
        self.ring.capacity
    }

    pub fn room_frames(&self) -> usize {
        self.ring.capacity.saturating_sub(self.queued_frames())
    }

    pub fn channels(&self) -> u16 {
        self.ring.channels as u16
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn is_connected(&self) -> bool {
        Arc::strong_count(&self.ring) > 1
    }

    pub fn finish(&self) {
        self.ring.lock().closed = true;
    }
}

impl Drop for PcmSender {
    fn drop(&mut self) {
        self.finish();
    }
}

pub(crate) fn pcm_pair(channels: u16, capacity_frames: usize, rate: u32) -> (PcmVoice, PcmSender) {
    let ring = PcmRing::new(usize::from(channels), capacity_frames);
    let edge = ((f64::from(PCM_EDGE_SECONDS) * f64::from(rate)).round() as u32).max(1);
    let voice = PcmVoice {
        ring: Arc::clone(&ring),
        local: VecDeque::new(),
        level: 0.0,
        step: 1.0 / edge as f32,
        last: (0.0, 0.0),
    };
    (voice, PcmSender { ring, rate })
}

pub(crate) struct PcmVoice {
    ring: Arc<PcmRing>,
    local: VecDeque<(f32, f32)>,
    level: f32,
    step: f32,
    last: (f32, f32),
}

impl PcmVoice {
    pub(crate) fn channels(&self) -> u16 {
        self.ring.channels.min(2) as u16
    }

    pub(crate) fn fill(&mut self, frames: usize) {
        if self.local.len() >= frames {
            return;
        }
        let channels = self.ring.channels;
        let mut inner = self.ring.lock();
        let take = (inner.samples.len() / channels).min(frames - self.local.len());
        for _ in 0..take {
            let left = inner.samples.pop_front().unwrap_or(0.0);
            let right = if channels > 1 {
                inner.samples.pop_front().unwrap_or(0.0)
            } else {
                left
            };
            for _ in 2..channels {
                inner.samples.pop_front();
            }
            self.local.push_back((left, right));
        }
    }

    pub(crate) fn next(&mut self) -> (f32, f32) {
        match self.local.pop_front() {
            Some(frame) => {
                self.last = frame;
                self.level = (self.level + self.step).min(1.0);
                (frame.0 * self.level, frame.1 * self.level)
            }
            None => {
                self.level = (self.level - self.step).max(0.0);
                (self.last.0 * self.level, self.last.1 * self.level)
            }
        }
    }

    pub(crate) fn finished(&self) -> bool {
        if self.level > 0.0 || !self.local.is_empty() {
            return false;
        }
        let inner = self.ring.lock();
        inner.closed && inner.samples.is_empty()
    }
}
