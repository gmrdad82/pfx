use std::sync::atomic::{AtomicU64, Ordering, fence};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Bounds {
    Once(u64),
    Loop(u64, u64),
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MusicPlace {
    pub(crate) played: f64,
    pub(crate) ratio: f64,
    pub(crate) bounds: Bounds,
}

impl MusicPlace {
    pub(crate) fn frame_after(&self, frames: f64) -> f64 {
        let played = (self.played + frames * self.ratio).max(0.0);
        match self.bounds {
            Bounds::Once(total) => played.min(total as f64),
            Bounds::Loop(start, end) if played >= end as f64 => {
                start as f64 + (played - end as f64) % (end - start) as f64
            }
            Bounds::Loop(..) | Bounds::Open => played,
        }
    }
}

pub(crate) struct Heard {
    origin: Instant,
    rate: f64,
    seq: AtomicU64,
    kind: AtomicU64,
    played: AtomicU64,
    ratio: AtomicU64,
    first: AtomicU64,
    second: AtomicU64,
    at: AtomicU64,
    span: AtomicU64,
}

struct Snapshot {
    place: Option<MusicPlace>,
    at: u64,
    span: u64,
}

impl Heard {
    pub(crate) fn new(rate: u32) -> Self {
        Self {
            origin: Instant::now(),
            rate: f64::from(rate.max(1)),
            seq: AtomicU64::new(0),
            kind: AtomicU64::new(0),
            played: AtomicU64::new(0),
            ratio: AtomicU64::new(0),
            first: AtomicU64::new(0),
            second: AtomicU64::new(0),
            at: AtomicU64::new(0),
            span: AtomicU64::new(0),
        }
    }

    pub(crate) fn publish(&self, place: Option<MusicPlace>, heard_at: Instant, block: Duration) {
        let (kind, first, second) = match place.map(|place| place.bounds) {
            None => (0, 0, 0),
            Some(Bounds::Once(total)) => (1, total, 0),
            Some(Bounds::Loop(start, end)) => (2, start, end),
            Some(Bounds::Open) => (3, 0, 0),
        };
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.kind.store(kind, Ordering::Relaxed);
        if let Some(place) = place {
            self.played.store(place.played.to_bits(), Ordering::Relaxed);
            self.ratio.store(place.ratio.to_bits(), Ordering::Relaxed);
        }
        self.first.store(first, Ordering::Relaxed);
        self.second.store(second, Ordering::Relaxed);
        self.at.store(self.nanos(heard_at), Ordering::Relaxed);
        self.span.store(block.as_nanos() as u64, Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    pub(crate) fn seconds(&self, now: Instant) -> Option<f64> {
        let snapshot = self.snapshot();
        let place = snapshot.place?;
        let now = self.nanos(now) as f64;
        let since = (now - snapshot.at as f64).min(snapshot.span as f64) / 1e9;
        Some(place.frame_after(since * self.rate) / self.rate)
    }

    fn snapshot(&self) -> Snapshot {
        loop {
            let before = self.seq.load(Ordering::Acquire);
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let kind = self.kind.load(Ordering::Relaxed);
            let played = f64::from_bits(self.played.load(Ordering::Relaxed));
            let ratio = f64::from_bits(self.ratio.load(Ordering::Relaxed));
            let first = self.first.load(Ordering::Relaxed);
            let second = self.second.load(Ordering::Relaxed);
            let at = self.at.load(Ordering::Relaxed);
            let span = self.span.load(Ordering::Relaxed);
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) != before {
                continue;
            }
            let bounds = match kind {
                1 => Bounds::Once(first),
                2 => Bounds::Loop(first, second),
                3 => Bounds::Open,
                _ => {
                    return Snapshot {
                        place: None,
                        at,
                        span,
                    };
                }
            };
            return Snapshot {
                place: Some(MusicPlace {
                    played,
                    ratio,
                    bounds,
                }),
                at,
                span,
            };
        }
    }

    fn nanos(&self, instant: Instant) -> u64 {
        instant.saturating_duration_since(self.origin).as_nanos() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn heard_with(place: MusicPlace, latency_ms: u64) -> (Heard, Instant) {
        let heard = Heard::new(RATE);
        let callback = heard.origin + Duration::from_secs(1);
        heard.publish(
            Some(place),
            callback + Duration::from_millis(latency_ms),
            Duration::from_millis(10),
        );
        (heard, callback)
    }

    #[test]
    fn the_heard_frame_trails_the_rendered_one_by_the_output_latency() {
        let place = MusicPlace {
            played: 96_000.0,
            ratio: 1.0,
            bounds: Bounds::Once(480_000),
        };
        let (heard, callback) = heard_with(place, 40);
        let at_callback = heard.seconds(callback).unwrap();
        assert!((at_callback - 1.96).abs() < 1e-6, "{at_callback}");
        let later = heard.seconds(callback + Duration::from_millis(45)).unwrap();
        assert!((later - 2.005).abs() < 1e-6, "{later}");
        let stalled = heard.seconds(callback + Duration::from_secs(5)).unwrap();
        assert!((stalled - 2.01).abs() < 1e-6, "{stalled}");
    }

    #[test]
    fn a_loop_just_wrapped_is_heard_before_its_end() {
        let place = MusicPlace {
            played: 48_000.0 + 480.0,
            ratio: 1.0,
            bounds: Bounds::Loop(4_800, 48_000),
        };
        let (heard, callback) = heard_with(place, 20);
        let seconds = heard.seconds(callback).unwrap();
        assert!((seconds - 0.99).abs() < 1e-6, "{seconds}");
        let wrapped = heard.seconds(callback + Duration::from_millis(30)).unwrap();
        assert!((wrapped - 0.12).abs() < 1e-6, "{wrapped}");
    }

    #[test]
    fn no_music_reads_none_and_a_start_not_yet_heard_reads_zero() {
        let heard = Heard::new(RATE);
        assert_eq!(heard.seconds(Instant::now()), None);
        let place = MusicPlace {
            played: 0.0,
            ratio: 1.0,
            bounds: Bounds::Open,
        };
        let (heard, callback) = heard_with(place, 30);
        assert_eq!(heard.seconds(callback), Some(0.0));
        heard.publish(None, callback, Duration::from_millis(10));
        assert_eq!(heard.seconds(callback), None);
    }
}
