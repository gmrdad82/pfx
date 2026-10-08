use crate::ease::Ease;
use crate::motion::Sample;

const SNAP: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Count(u32),
    Forever,
}

impl Default for Repeat {
    fn default() -> Self {
        Repeat::Count(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween<T: Sample> {
    pub from: T,
    pub to: T,
    pub duration: f32,
    pub ease: Ease,
    pub delay: f32,
    pub repeat: Repeat,
    pub yoyo: bool,
    time: f64,
}

impl<T: Sample> Tween<T> {
    pub fn new(from: T, to: T, duration: f32) -> Self {
        Self {
            from,
            to,
            duration,
            ease: Ease::Linear,
            delay: 0.0,
            repeat: Repeat::default(),
            yoyo: false,
            time: 0.0,
        }
    }

    pub fn with_ease(mut self, ease: Ease) -> Self {
        self.ease = ease;
        self
    }

    pub fn with_delay(mut self, delay: f32) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_repeat(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }

    pub fn with_yoyo(mut self) -> Self {
        self.yoyo = true;
        self
    }

    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.advance(self.time + f64::from(dt));
    }

    pub fn seek(&mut self, seconds: f32) {
        if seconds.is_finite() {
            self.advance(f64::from(seconds).max(0.0));
        }
    }

    pub fn reset(&mut self) {
        self.time = 0.0;
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn value(&self) -> T {
        self.sample(self.time)
    }

    pub fn at(&self, seconds: f32) -> T {
        self.sample(f64::from(seconds))
    }

    pub fn finished(&self) -> bool {
        self.length().is_some_and(|total| self.time >= total)
    }

    pub fn length(&self) -> Option<f64> {
        match self.repeat {
            Repeat::Count(count) => {
                Some(self.delay_seconds() + f64::from(count.max(1)) * self.duration_seconds())
            }
            Repeat::Forever => None,
        }
    }

    fn delay_seconds(&self) -> f64 {
        f64::from(self.delay).max(0.0)
    }

    fn duration_seconds(&self) -> f64 {
        f64::from(self.duration).max(0.0)
    }

    fn advance(&mut self, to: f64) {
        self.time = match self.length() {
            Some(total) if to + SNAP >= total => total,
            _ => to,
        };
    }

    fn sample(&self, seconds: f64) -> T {
        let local = seconds - self.delay_seconds();
        let duration = self.duration_seconds();
        let cycles = match self.repeat {
            Repeat::Count(count) => Some(u64::from(count.max(1))),
            Repeat::Forever => None,
        };
        let done = match self.length() {
            Some(total) => seconds >= total,
            None => duration <= 0.0 && local > 0.0,
        };
        if !done && (local.is_nan() || local <= 0.0) {
            return self.from;
        }
        let (cycle, fraction) = if done {
            (cycles.map_or(0, |count| count - 1), 1.0)
        } else {
            let progress = local / duration;
            let whole = progress.floor();
            (whole as u64, progress - whole)
        };
        let mut x = fraction as f32;
        if self.yoyo && cycle % 2 == 1 {
            x = 1.0 - x;
        }
        self.from.lerp(self.to, self.ease.at(x))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence<T: Sample> {
    tweens: Vec<Tween<T>>,
    time: f64,
}

impl<T: Sample> Default for Sequence<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Sample> Sequence<T> {
    pub fn new() -> Self {
        Self {
            tweens: Vec::new(),
            time: 0.0,
        }
    }

    pub fn then(mut self, tween: Tween<T>) -> Self {
        self.tweens.push(tween);
        self
    }

    pub fn len(&self) -> usize {
        self.tweens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tweens.is_empty()
    }

    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.advance(self.time + f64::from(dt));
    }

    pub fn seek(&mut self, seconds: f32) {
        if seconds.is_finite() {
            self.advance(f64::from(seconds).max(0.0));
        }
    }

    pub fn reset(&mut self) {
        self.time = 0.0;
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn value(&self) -> T {
        self.sample(self.time)
    }

    pub fn at(&self, seconds: f32) -> T {
        self.sample(f64::from(seconds))
    }

    pub fn finished(&self) -> bool {
        self.length().is_some_and(|total| self.time >= total)
    }

    pub fn length(&self) -> Option<f64> {
        let mut total = 0.0;
        for tween in &self.tweens {
            total += tween.length()?;
        }
        Some(total)
    }

    pub fn current(&self) -> Option<usize> {
        let mut start = 0.0;
        for (index, tween) in self.tweens.iter().enumerate() {
            match tween.length() {
                Some(len) if self.time >= start + len => start += len,
                _ => return Some(index),
            }
        }
        None
    }

    fn advance(&mut self, to: f64) {
        self.time = match self.length() {
            Some(total) if to + SNAP >= total => total,
            _ => to,
        };
    }

    fn sample(&self, seconds: f64) -> T {
        let mut start = 0.0;
        let mut last = T::default();
        for tween in &self.tweens {
            match tween.length() {
                Some(len) => {
                    let end = start + len;
                    if seconds >= end {
                        last = tween.sample(len);
                        start = end;
                    } else {
                        return tween.sample(seconds - start);
                    }
                }
                None => return tween.sample(seconds - start),
            }
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn sixty_steps_of_a_sixtieth_land_exactly_on_the_end() {
        for ease in Ease::ALL {
            let mut tween = Tween::new(2.0f32, 5.0, 1.0).with_ease(ease);
            for _ in 0..59 {
                tween.step(DT);
                assert!(!tween.finished(), "{}", ease.name());
            }
            tween.step(DT);
            assert!(tween.finished(), "{}", ease.name());
            assert_eq!(tween.value().to_bits(), tween.at(1.0).to_bits());
            assert_eq!(tween.value(), 5.0);
            tween.step(DT);
            assert_eq!(tween.value(), 5.0);
        }
    }

    #[test]
    fn stepping_agrees_with_random_access_all_the_way() {
        for ease in Ease::ALL {
            let mut tween = Tween::new(-1.0f32, 3.0, 1.0)
                .with_ease(ease)
                .with_delay(0.25);
            let mut clock = 0.0f64;
            for _ in 0..90 {
                tween.step(DT);
                clock += f64::from(DT);
                assert!(
                    close(tween.value(), tween.sample(clock)),
                    "{} at {clock}",
                    ease.name()
                );
            }
            assert_eq!(tween.value(), 3.0);
        }
    }

    #[test]
    fn other_frame_rates_land_on_the_same_end() {
        for frames in [24u32, 30, 48, 60, 90, 120, 144, 240] {
            let dt = 1.0 / frames as f32;
            let mut tween = Tween::new(0.0f32, 1.0, 1.0).with_ease(Ease::CubicInOut);
            for _ in 0..frames {
                tween.step(dt);
            }
            assert!(tween.finished(), "{frames}");
            assert_eq!(tween.value(), 1.0, "{frames}");
        }
        let mut fine = Tween::new(0.0f32, 1.0, 0.7);
        let mut coarse = fine;
        for _ in 0..70 {
            fine.step(0.01);
        }
        for _ in 0..7 {
            coarse.step(0.1);
        }
        assert!(fine.finished() && coarse.finished());
        assert_eq!(fine.value(), coarse.value());
    }

    #[test]
    fn a_tween_holds_its_start_through_the_delay() {
        let tween = Tween::new(10.0f32, 20.0, 2.0).with_delay(1.0);
        assert_eq!(tween.at(-5.0), 10.0);
        assert_eq!(tween.at(0.0), 10.0);
        assert_eq!(tween.at(1.0), 10.0);
        assert_eq!(tween.at(2.0), 15.0);
        assert_eq!(tween.at(3.0), 20.0);
        assert_eq!(tween.at(99.0), 20.0);
        assert_eq!(tween.length(), Some(3.0));
        let mut stepped = tween;
        stepped.step(0.5);
        assert_eq!(stepped.value(), 10.0);
        assert!(!stepped.finished());
    }

    #[test]
    fn a_repeat_count_replays_from_the_start() {
        let tween = Tween::new(0.0f32, 1.0, 2.0).with_repeat(Repeat::Count(3));
        assert_eq!(tween.length(), Some(6.0));
        assert_eq!(tween.at(1.0), 0.5);
        assert_eq!(tween.at(2.0), 0.0);
        assert_eq!(tween.at(3.0), 0.5);
        assert_eq!(tween.at(5.0), 0.5);
        assert_eq!(tween.at(6.0), 1.0);
        assert_eq!(tween.at(60.0), 1.0);

        let mut stepped = tween;
        let mut previous = 0.0f32;
        let mut drops = 0;
        for _ in 0..360 {
            stepped.step(DT);
            let value = stepped.value();
            if value < previous {
                drops += 1;
            }
            previous = value;
        }
        assert_eq!(drops, 2);
        assert!(stepped.finished());
        assert_eq!(stepped.value(), 1.0);

        let once = Tween::new(0.0f32, 1.0, 2.0).with_repeat(Repeat::Count(0));
        assert_eq!(once.length(), Some(2.0));
        assert_eq!(once.at(2.0), 1.0);
    }

    #[test]
    fn yoyo_runs_every_other_cycle_backward() {
        let tween = Tween::new(0.0f32, 4.0, 1.0)
            .with_repeat(Repeat::Count(4))
            .with_yoyo();
        assert_eq!(tween.at(0.25), 1.0);
        assert_eq!(tween.at(1.0), 4.0);
        assert_eq!(tween.at(1.25), 3.0);
        assert_eq!(tween.at(2.0), 0.0);
        assert_eq!(tween.at(2.25), 1.0);
        assert_eq!(tween.at(3.5), 2.0);
        assert_eq!(tween.at(4.0), 0.0);
        assert_eq!(tween.at(40.0), 0.0);

        let odd = Tween::new(0.0f32, 4.0, 1.0)
            .with_repeat(Repeat::Count(3))
            .with_yoyo();
        assert_eq!(odd.at(3.0), 4.0);

        let mut stepped = tween;
        for _ in 0..240 {
            stepped.step(DT);
        }
        assert!(stepped.finished());
        assert_eq!(stepped.value(), 0.0);

        let single = Tween::new(0.0f32, 4.0, 1.0).with_yoyo();
        assert_eq!(single.at(1.0), 4.0);
    }

    #[test]
    fn yoyo_mirrors_an_eased_curve_in_time() {
        let tween = Tween::new(0.0f32, 1.0, 1.0)
            .with_ease(Ease::BackOut)
            .with_repeat(Repeat::Count(2))
            .with_yoyo();
        for i in 1..100 {
            let t = i as f32 / 100.0;
            let eased = Ease::BackOut.at(t);
            assert!(close(tween.at(t), eased), "{t}");
            assert!(close(tween.at(1.0 + t), Ease::BackOut.at(1.0 - t)), "{t}");
            assert!(close(tween.at(2.0 - t), eased), "{t}");
        }
    }

    #[test]
    fn forever_never_finishes() {
        let tween = Tween::new(0.0f32, 1.0, 1.0)
            .with_repeat(Repeat::Forever)
            .with_yoyo();
        assert_eq!(tween.length(), None);
        assert_eq!(tween.at(0.5), 0.5);
        assert_eq!(tween.at(1.5), 0.5);
        assert_eq!(tween.at(1001.25), 0.75);
        assert_eq!(tween.at(1000.25), 0.25);
        let mut stepped = tween;
        for _ in 0..6000 {
            stepped.step(DT);
        }
        assert!(!stepped.finished());
        assert_eq!(stepped.value(), stepped.sample(stepped.time()));

        let plain = Tween::new(0.0f32, 8.0, 2.0).with_repeat(Repeat::Forever);
        assert_eq!(plain.at(3.0), 4.0);
        assert_eq!(plain.at(201.0), 4.0);
    }

    #[test]
    fn a_zero_length_tween_jumps_to_its_end() {
        let tween = Tween::new(1.0f32, 2.0, 0.0);
        assert!(tween.finished());
        assert_eq!(tween.at(-0.1), 1.0);
        assert_eq!(tween.at(0.0), 2.0);
        assert_eq!(tween.value(), 2.0);
        let delayed = Tween::new(1.0f32, 2.0, 0.0).with_delay(1.0);
        assert_eq!(delayed.at(0.5), 1.0);
        assert_eq!(delayed.at(1.0), 2.0);
        assert_eq!(delayed.at(1.1), 2.0);
        let forever = Tween::new(1.0f32, 2.0, 0.0).with_repeat(Repeat::Forever);
        assert_eq!(forever.at(5.0), 2.0);
        assert_eq!(forever.at(0.0), 1.0);
    }

    #[test]
    fn bad_time_is_ignored() {
        let mut tween = Tween::new(0.0f32, 1.0, 1.0);
        tween.step(f32::NAN);
        tween.step(-1.0);
        tween.step(f32::INFINITY);
        tween.step(0.0);
        assert_eq!(tween.time(), 0.0);
        tween.seek(f32::NAN);
        assert_eq!(tween.time(), 0.0);
        tween.seek(0.25);
        assert_eq!(tween.value(), 0.25);
        tween.seek(-3.0);
        assert_eq!(tween.value(), 0.0);
        tween.seek(0.5);
        tween.reset();
        assert_eq!(tween.time(), 0.0);
        assert_eq!(tween.value(), 0.0);
    }

    #[test]
    fn a_tween_moves_a_vector_lane_by_lane() {
        let tween = Tween::new([0.0f32, 10.0, -4.0], [4.0, 0.0, 4.0], 2.0).with_ease(Ease::QuadIn);
        assert_eq!(tween.at(1.0), [1.0, 7.5, -2.0]);
        assert_eq!(tween.at(2.0), [4.0, 0.0, 4.0]);
        let mut stepped = tween;
        for _ in 0..120 {
            stepped.step(DT);
        }
        assert_eq!(stepped.value(), [4.0, 0.0, 4.0]);
    }

    #[test]
    fn a_sequence_chains_tweens_end_to_start() {
        let sequence = Sequence::new()
            .then(Tween::new(0.0f32, 1.0, 1.0))
            .then(Tween::new(5.0f32, 7.0, 2.0).with_delay(1.0))
            .then(Tween::new(-1.0f32, -2.0, 1.0).with_repeat(Repeat::Count(2)));
        assert_eq!(sequence.len(), 3);
        assert!(!sequence.is_empty());
        assert_eq!(sequence.length(), Some(6.0));
        assert_eq!(sequence.at(-1.0), 0.0);
        assert_eq!(sequence.at(0.5), 0.5);
        assert_eq!(sequence.at(1.0), 5.0);
        assert_eq!(sequence.at(1.5), 5.0);
        assert_eq!(sequence.at(2.0), 5.0);
        assert_eq!(sequence.at(3.0), 6.0);
        assert_eq!(sequence.at(4.0), -1.0);
        assert_eq!(sequence.at(4.5), -1.5);
        assert_eq!(sequence.at(5.0), -1.0);
        assert_eq!(sequence.at(6.0), -2.0);
        assert_eq!(sequence.at(100.0), -2.0);
    }

    #[test]
    fn a_sequence_steps_like_it_is_read_at_random() {
        let sequence = Sequence::new()
            .then(Tween::new(0.0f32, 1.0, 0.5).with_ease(Ease::ElasticOut))
            .then(Tween::new(1.0f32, 0.0, 0.5).with_ease(Ease::BounceOut))
            .then(
                Tween::new(0.0f32, 3.0, 0.25)
                    .with_repeat(Repeat::Count(2))
                    .with_yoyo(),
            );
        let mut stepped = sequence.clone();
        let mut clock = 0.0f64;
        for _ in 0..90 {
            stepped.step(DT);
            clock += f64::from(DT);
            assert!(close(stepped.value(), sequence.sample(clock)), "{clock}");
        }
        assert!(stepped.finished());
        assert_eq!(stepped.value().to_bits(), sequence.at(1.5).to_bits());
        assert_eq!(stepped.value(), 0.0);
    }

    #[test]
    fn sixty_steps_land_on_a_two_part_sequence() {
        let mut sequence = Sequence::new()
            .then(Tween::new(0.0f32, 1.0, 0.5).with_ease(Ease::SineInOut))
            .then(Tween::new(1.0f32, 9.0, 0.5).with_ease(Ease::CircOut));
        for _ in 0..59 {
            sequence.step(DT);
        }
        assert!(!sequence.finished());
        sequence.step(DT);
        assert!(sequence.finished());
        assert_eq!(sequence.value().to_bits(), sequence.at(1.0).to_bits());
        assert_eq!(sequence.value(), 9.0);
    }

    #[test]
    fn a_sequence_reports_where_it_is() {
        let mut sequence = Sequence::new()
            .then(Tween::new(0.0f32, 1.0, 1.0))
            .then(Tween::new(1.0f32, 2.0, 1.0));
        assert_eq!(sequence.current(), Some(0));
        sequence.seek(1.0);
        assert_eq!(sequence.current(), Some(1));
        sequence.seek(2.0);
        assert_eq!(sequence.current(), None);
        assert!(sequence.finished());
        sequence.reset();
        assert_eq!(sequence.current(), Some(0));
        assert_eq!(sequence.value(), 0.0);
    }

    #[test]
    fn a_forever_tween_ends_its_sequence() {
        let sequence = Sequence::new()
            .then(Tween::new(0.0f32, 1.0, 1.0))
            .then(Tween::new(0.0f32, 2.0, 1.0).with_repeat(Repeat::Forever))
            .then(Tween::new(9.0f32, 9.5, 1.0));
        assert_eq!(sequence.length(), None);
        assert_eq!(sequence.at(1.5), 1.0);
        assert_eq!(sequence.at(500.5), 1.0);
        let mut stepped = sequence.clone();
        for _ in 0..600 {
            stepped.step(DT);
        }
        assert!(!stepped.finished());
        assert_eq!(stepped.current(), Some(1));
    }

    #[test]
    fn an_empty_sequence_rests_at_the_default() {
        let mut sequence = Sequence::<f32>::new();
        assert!(sequence.is_empty());
        assert_eq!(sequence.length(), Some(0.0));
        assert!(sequence.finished());
        sequence.step(DT);
        assert_eq!(sequence.value(), 0.0);
        assert_eq!(sequence.current(), None);
    }
}
