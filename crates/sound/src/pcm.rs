use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

pub const PCM_EDGE_SECONDS: f32 = 0.002;

const LOCAL_FRAMES: usize = 4_096;

pub(crate) struct PcmRing {
    slots: Box<[AtomicU32]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    closed: AtomicBool,
    capacity: usize,
    channels: usize,
}

impl PcmRing {
    fn new(channels: usize, capacity_frames: usize) -> Arc<Self> {
        let capacity = capacity_frames.max(1);
        let channels = channels.max(1);
        let slots = (0..capacity.saturating_mul(channels))
            .map(|_| AtomicU32::new(0))
            .collect();
        Arc::new(Self {
            slots,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            capacity,
            channels,
        })
    }

    fn queued(&self, head: usize, tail: usize) -> usize {
        let len = self.slots.len();
        if tail >= head {
            tail - head
        } else {
            tail + 2 * len - head
        }
    }

    fn advance(&self, position: usize, by: usize) -> usize {
        let wrap = 2 * self.slots.len();
        let next = position + by;
        if next >= wrap { next - wrap } else { next }
    }

    fn slot(&self, position: usize) -> usize {
        let len = self.slots.len();
        if position >= len {
            position - len
        } else {
            position
        }
    }

    fn queued_frames(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        self.queued(head, tail) / self.channels
    }
}

pub struct PcmSender {
    ring: Arc<PcmRing>,
    rate: u32,
}

impl PcmSender {
    pub fn push(&self, samples: &[f32]) -> usize {
        let ring = &*self.ring;
        let channels = ring.channels;
        let tail = ring.tail.load(Ordering::Acquire);
        let head = ring.head.load(Ordering::Acquire);
        let queued = ring.queued(head, tail) / channels;
        let room = ring.capacity.saturating_sub(queued);
        let frames = (samples.len() / channels).min(room);
        let count = frames * channels;
        let mut slot = ring.slot(tail);
        for sample in &samples[..count] {
            ring.slots[slot].store(sample.to_bits(), Ordering::Relaxed);
            slot += 1;
            if slot == ring.slots.len() {
                slot = 0;
            }
        }
        ring.tail
            .store(ring.advance(tail, count), Ordering::Release);
        frames
    }

    pub fn queued_frames(&self) -> usize {
        self.ring.queued_frames()
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
        self.ring.closed.store(true, Ordering::Release);
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
        local: VecDeque::with_capacity(LOCAL_FRAMES),
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
        let ring = &*self.ring;
        let channels = ring.channels;
        let head = ring.head.load(Ordering::Acquire);
        let tail = ring.tail.load(Ordering::Acquire);
        let take = (ring.queued(head, tail) / channels).min(frames - self.local.len());
        let mut slot = ring.slot(head);
        let mut read = || {
            let sample = f32::from_bits(ring.slots[slot].load(Ordering::Relaxed));
            slot += 1;
            if slot == ring.slots.len() {
                slot = 0;
            }
            sample
        };
        for _ in 0..take {
            let left = read();
            let right = if channels > 1 { read() } else { left };
            for _ in 2..channels {
                read();
            }
            self.local.push_back((left, right));
        }
        ring.head
            .store(ring.advance(head, take * channels), Ordering::Release);
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
        self.ring.closed.load(Ordering::Acquire) && self.ring.queued_frames() == 0
    }
}
