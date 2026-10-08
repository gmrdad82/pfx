pub const RATE: f32 = 240.0;
pub const STEP: f32 = 1.0 / 240.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rest {
    pub position: f32,
    pub velocity: f32,
}

impl Rest {
    pub const COARSE: Self = Self {
        position: 1e-4,
        velocity: 1e-3,
    };
    pub const FINE: Self = Self {
        position: 1e-5,
        velocity: 1e-4,
    };
}

impl Default for Rest {
    fn default() -> Self {
        Self::COARSE
    }
}

pub trait Sample: Copy + Default {
    fn integrate(
        &mut self,
        velocity: &mut Self,
        target: Self,
        stiffness: f32,
        damping: f32,
        h: f32,
    );
    fn settled(self, target: Self, velocity: Self, rest: Rest) -> bool;
    fn lerp(self, to: Self, t: f32) -> Self;
}

impl Sample for f32 {
    fn integrate(
        &mut self,
        velocity: &mut Self,
        target: Self,
        stiffness: f32,
        damping: f32,
        h: f32,
    ) {
        let force = -stiffness * (*self - target) - damping * *velocity;
        *velocity += force * h;
        *self += *velocity * h;
    }

    fn settled(self, target: Self, velocity: Self, rest: Rest) -> bool {
        (self - target).abs() < rest.position && velocity.abs() < rest.velocity
    }

    fn lerp(self, to: Self, t: f32) -> Self {
        self * (1.0 - t) + to * t
    }
}

impl Sample for [f32; 3] {
    fn integrate(
        &mut self,
        velocity: &mut Self,
        target: Self,
        stiffness: f32,
        damping: f32,
        h: f32,
    ) {
        self[0].integrate(&mut velocity[0], target[0], stiffness, damping, h);
        self[1].integrate(&mut velocity[1], target[1], stiffness, damping, h);
        self[2].integrate(&mut velocity[2], target[2], stiffness, damping, h);
    }

    fn settled(self, target: Self, velocity: Self, rest: Rest) -> bool {
        self[0].settled(target[0], velocity[0], rest)
            && self[1].settled(target[1], velocity[1], rest)
            && self[2].settled(target[2], velocity[2], rest)
    }

    fn lerp(self, to: Self, t: f32) -> Self {
        [
            self[0].lerp(to[0], t),
            self[1].lerp(to[1], t),
            self[2].lerp(to[2], t),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring<T: Sample> {
    pub value: T,
    pub velocity: T,
    pub target: T,
    pub stiffness: f32,
    pub damping_ratio: f32,
    pub rest: Rest,
}

impl<T: Sample> Spring<T> {
    pub fn new(stiffness: f32) -> Self {
        Self {
            value: T::default(),
            velocity: T::default(),
            target: T::default(),
            stiffness,
            damping_ratio: 1.0,
            rest: Rest::default(),
        }
    }

    pub fn with_rest(mut self, rest: Rest) -> Self {
        self.rest = rest;
        self
    }

    pub fn at(stiffness: f32, value: T) -> Self {
        let mut spring = Self::new(stiffness);
        spring.value = value;
        spring.target = value;
        spring
    }

    pub fn step(&mut self, dt: f32) {
        let steps = (dt / STEP).ceil().max(1.0) as u32;
        let h = dt / steps as f32;
        let damping = 2.0 * self.stiffness.sqrt() * self.damping_ratio;
        for _ in 0..steps {
            self.value
                .integrate(&mut self.velocity, self.target, self.stiffness, damping, h);
        }
        if self.settled() {
            self.value = self.target;
            self.velocity = T::default();
        }
    }

    pub fn settled(&self) -> bool {
        self.value.settled(self.target, self.velocity, self.rest)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cascade<T: Sample> {
    pub lead: Spring<T>,
    pub follow: Spring<T>,
}

impl<T: Sample> Cascade<T> {
    pub fn new(stiffness: f32, value: T) -> Self {
        Self {
            lead: Spring::at(stiffness, value),
            follow: Spring::at(stiffness, value),
        }
    }

    pub fn with_rest(mut self, rest: Rest) -> Self {
        self.lead.rest = rest;
        self.follow.rest = rest;
        self
    }

    pub fn step(&mut self, dt: f32) {
        self.lead.step(dt);
        self.follow.target = self.lead.value;
        self.follow.step(dt);
    }

    pub fn value(&self) -> T {
        self.follow.value
    }

    pub fn settled(&self) -> bool {
        self.lead.settled() && self.follow.settled()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stepper {
    time: f64,
    ticks: u64,
}

impl Default for Stepper {
    fn default() -> Self {
        Self::new()
    }
}

impl Stepper {
    pub fn new() -> Self {
        Self {
            time: 0.0,
            ticks: 0,
        }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn advance(&mut self, dt: f32) -> u32 {
        if !dt.is_finite() || dt <= 0.0 {
            return 0;
        }
        self.advance_f64(self.time + f64::from(dt))
    }

    pub fn advance_to(&mut self, to: f32) -> u32 {
        self.advance_f64(f64::from(to))
    }

    fn advance_f64(&mut self, to: f64) -> u32 {
        if !to.is_finite() || to < self.time {
            return 0;
        }
        let due = (to * f64::from(RATE)).floor().max(0.0) as u64;
        let n = due.saturating_sub(self.ticks);
        self.ticks = due;
        self.time = to;
        u32::try_from(n).unwrap_or(u32::MAX)
    }
}

pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn smootherstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let u = if t <= 0.5 { t } else { 1.0 - t };
    let y = u * u * u * (u * (u * 6.0 - 15.0) + 10.0);
    if t <= 0.5 { y } else { 1.0 - y }
}

pub fn quad_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t
}

pub fn quad_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let u = 1.0 - t;
    1.0 - u * u
}

pub fn cubic_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

pub fn cubic_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let u = 1.0 - t;
    1.0 - u * u * u
}

pub fn cubic_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let u = -2.0 * t + 2.0;
        1.0 - u * u * u / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fine_reference(
        mut value: f32,
        mut velocity: f32,
        target: f32,
        stiffness: f32,
        dt: f32,
    ) -> (f32, f32) {
        let steps = (dt / (1.0 / 240.0)).ceil().max(1.0) as u32;
        let h = dt / steps as f32;
        let damping = 2.0 * stiffness.sqrt();
        for _ in 0..steps {
            let force = -stiffness * (value - target) - damping * velocity;
            velocity += force * h;
            value += velocity * h;
        }
        if (value - target).abs() < 1e-5 && velocity.abs() < 1e-4 {
            value = target;
            velocity = 0.0;
        }
        (value, velocity)
    }

    #[test]
    fn a_fine_spring_follows_small_moves_that_a_coarse_one_snaps() {
        let mut fine = Spring::at(40.0, 0.0f32).with_rest(Rest::FINE);
        let mut coarse = Spring::at(40.0, 0.0f32);
        fine.target = 5e-5;
        coarse.target = 5e-5;
        fine.step(1.0 / 240.0);
        coarse.step(1.0 / 240.0);
        assert_eq!(coarse.value, 5e-5);
        assert!(fine.value < 5e-5);
        assert!(!fine.settled());
    }

    #[test]
    fn a_fine_spring_matches_its_reference_step_for_step() {
        let mut spring = Spring::at(37.0, 0.0f32).with_rest(Rest::FINE);
        let (mut value, mut velocity) = (0.0f32, 0.0f32);
        let frames = [1.0 / 60.0, 1.0 / 144.0, 0.02, 1.0 / 240.0, 0.05, 1.0 / 30.0];
        for (index, target) in [0.004f32, -0.0003, 0.00002, 0.0].iter().enumerate() {
            spring.target = *target;
            for dt in frames.iter().cycle().skip(index).take(200) {
                spring.step(*dt);
                (value, velocity) = fine_reference(value, velocity, *target, 37.0, *dt);
                assert_eq!(spring.value.to_bits(), value.to_bits());
                assert_eq!(spring.velocity.to_bits(), velocity.to_bits());
            }
        }
    }

    #[test]
    fn a_cascade_carries_its_rest_to_both_springs() {
        let cascade = Cascade::new(10.0, 1.0f32).with_rest(Rest::FINE);
        assert_eq!(cascade.lead.rest, Rest::FINE);
        assert_eq!(cascade.follow.rest, Rest::FINE);
        assert_eq!(Cascade::new(10.0, 1.0f32).lead.rest, Rest::COARSE);
    }

    fn coarse_reference(
        mut value: f32,
        mut velocity: f32,
        target: f32,
        stiffness: f32,
        ratio: f32,
        dt: f32,
    ) -> (f32, f32) {
        let steps = (dt / (1.0 / 240.0)).ceil().max(1.0) as u32;
        let h = dt / steps as f32;
        let damping = 2.0 * stiffness.sqrt() * ratio;
        for _ in 0..steps {
            let force = -stiffness * (value - target) - damping * velocity;
            velocity += force * h;
            value += velocity * h;
        }
        if (value - target).abs() < 1e-4 && velocity.abs() < 1e-3 {
            value = target;
            velocity = 0.0;
        }
        (value, velocity)
    }

    fn drive(spring: &mut Spring<f32>, frames: &[f32]) -> u32 {
        let mut clock = Stepper::new();
        let mut ticks = 0;
        for &dt in frames {
            let n = clock.advance(dt);
            ticks += n;
            for _ in 0..n {
                spring.step(STEP);
            }
        }
        ticks
    }

    #[test]
    fn settling_time() {
        let mut spring = Spring::new(100.0);
        spring.target = 1.0;
        let dt = 1.0 / 60.0;
        let mut time = 0.0;
        let mut previous = 0.0;
        while !spring.settled() {
            spring.step(dt);
            assert!(spring.value >= previous);
            assert!(spring.value <= 1.0);
            previous = spring.value;
            time += dt;
            assert!(time < 3.0, "still moving at {time}");
        }
        assert!(time > 1.0 && time < 1.5, "settled at {time}");
        assert_eq!(spring.value, 1.0);
        assert_eq!(spring.velocity, 0.0);
        spring.step(dt);
        assert_eq!(spring.value, 1.0);
        assert_eq!(spring.velocity, 0.0);
    }

    #[test]
    fn frame_patterns_agree() {
        let patterns: [Vec<f32>; 4] = [
            vec![1.0 / 60.0; 30],
            vec![1.0 / 240.0; 120],
            vec![1.0 / 120.0; 60],
            vec![1.0 / 30.0; 15],
        ];
        let mut states = Vec::new();
        for frames in &patterns {
            let mut spring = Spring::new(44.0);
            spring.target = 1.0;
            let ticks = drive(&mut spring, frames);
            assert_eq!(ticks, 120);
            states.push(spring);
        }
        let mut jumped = Spring::new(44.0);
        jumped.target = 1.0;
        let mut clock = Stepper::new();
        let ticks = clock.advance_to(0.5);
        assert_eq!(ticks, 120);
        for _ in 0..ticks {
            jumped.step(STEP);
        }
        for spring in states {
            assert!((spring.value - jumped.value).abs() < 1e-6);
            assert!((spring.velocity - jumped.velocity).abs() < 1e-6);
        }

        let mut odd = Spring::new(44.0);
        odd.target = 1.0;
        let left = drive(&mut odd, &[1.0 / 144.0; 72]);
        let mut even = Spring::new(44.0);
        even.target = 1.0;
        let right = drive(&mut even, &[1.0 / 60.0; 30]);
        assert_eq!(left, right);
        assert!((odd.value - even.value).abs() < 1e-6);
        assert!((odd.velocity - even.velocity).abs() < 1e-6);

        let mut once = Spring::new(44.0);
        once.target = [0.2, -0.4, 0.8];
        let mut other = once;
        let mut a = Stepper::new();
        let mut b = Stepper::new();
        for _ in 0..40 {
            for _ in 0..a.advance(1.0 / 60.0) {
                once.step(STEP);
            }
            for _ in 0..4 {
                for _ in 0..b.advance(1.0 / 240.0) {
                    other.step(STEP);
                }
            }
        }
        for ((left, left_v), (right, right_v)) in once
            .value
            .into_iter()
            .zip(once.velocity)
            .zip(other.value.into_iter().zip(other.velocity))
        {
            assert!((left - right).abs() < 1e-6);
            assert!((left_v - right_v).abs() < 1e-6);
        }
    }

    #[test]
    fn easing_endpoints_and_monotonicity() {
        let curves: [fn(f32) -> f32; 7] = [
            smoothstep,
            smootherstep,
            quad_in,
            quad_out,
            cubic_in,
            cubic_out,
            cubic_in_out,
        ];
        for curve in curves {
            assert_eq!(curve(0.0), 0.0);
            assert_eq!(curve(1.0), 1.0);
            assert_eq!(curve(-0.4), 0.0);
            assert_eq!(curve(1.7), 1.0);
            let mut previous = 0.0;
            for i in 0..=4000 {
                let y = curve(i as f32 / 4000.0);
                assert!((0.0..=1.0).contains(&y));
                assert!(y >= previous);
                previous = y;
            }
        }
        assert_eq!(smoothstep(0.5), 0.5);
        assert_eq!(smootherstep(0.5), 0.5);
        assert_eq!(quad_in(0.5), 0.25);
        assert_eq!(quad_out(0.5), 0.75);
        assert_eq!(cubic_in(0.5), 0.125);
        assert_eq!(cubic_out(0.5), 0.875);
        assert_eq!(cubic_in_out(0.5), 0.5);
        assert_eq!(smoothstep(0.25), 0.15625);
    }

    #[test]
    fn a_coarse_spring_matches_its_reference_step_for_step() {
        let mut spring = Spring::new(900.0);
        spring.target = 1.0;
        for _ in 0..8 {
            spring.step(1.0 / 60.0);
        }
        assert_eq!(spring.value.to_bits(), 0x3f68_168c);
        assert_eq!(spring.velocity.to_bits(), 0x4007_7e40);

        let mut turn = Spring::new(220.0);
        turn.target = 1.0;
        for _ in 0..12 {
            turn.step(1.0 / 60.0);
        }
        assert_eq!(turn.value.to_bits(), 0x3f4c_e683);
        assert_eq!(turn.velocity.to_bits(), 0x400c_876c);

        let stiffness = [5.0, 11.0, 44.0, 100.0, 220.0, 900.0];
        let dts = [STEP, 1.0 / 60.0, 1.0 / 144.0, 0.007, 0.02];
        let ratios = [1.0, 0.5, 1.4];
        for &k in &stiffness {
            for &dt in &dts {
                for &ratio in &ratios {
                    let mut value = 0.2;
                    let mut velocity = -0.3;
                    let mut spring = Spring::at(k, value);
                    spring.velocity = velocity;
                    spring.target = 0.7;
                    spring.damping_ratio = ratio;
                    for _ in 0..25 {
                        let (next, speed) = coarse_reference(value, velocity, 0.7, k, ratio, dt);
                        spring.step(dt);
                        assert_eq!(spring.value.to_bits(), next.to_bits());
                        assert_eq!(spring.velocity.to_bits(), speed.to_bits());
                        value = next;
                        velocity = speed;
                    }
                }
            }
        }

        let mut scalar = [Spring::new(132.0), Spring::new(132.0), Spring::new(132.0)];
        let targets = [0.4, -1.2, 2.5];
        for (spring, target) in scalar.iter_mut().zip(targets) {
            spring.target = target;
        }
        let mut vector = Spring::new(132.0);
        vector.target = targets;
        for _ in 0..17 {
            for spring in &mut scalar {
                spring.step(0.007);
            }
            vector.step(0.007);
        }
        for (lane, (value, velocity)) in scalar
            .iter()
            .zip(vector.value.into_iter().zip(vector.velocity))
        {
            assert_eq!(value.to_bits(), lane.value.to_bits());
            assert_eq!(velocity.to_bits(), lane.velocity.to_bits());
        }
    }

    #[test]
    fn cascade_lags_its_leader() {
        let mut cascade = Cascade::new(44.0, 0.0);
        cascade.lead.target = 1.0;
        cascade.step(1.0 / 60.0);
        assert!(cascade.follow.value > 0.0);
        assert!(cascade.follow.value < cascade.lead.value);
        assert!(cascade.lead.value < 1.0);
        for _ in 0..24 {
            cascade.step(1.0 / 60.0);
            assert!(cascade.follow.value <= cascade.lead.value);
        }
        assert!(cascade.lead.value - cascade.follow.value > 0.05);
        assert!(!cascade.settled());

        let mut spatial = Cascade::new(44.0, [0.0; 3]);
        spatial.lead.target = [1.0, -2.0, 0.5];
        for _ in 0..8 {
            spatial.step(1.0 / 60.0);
        }
        for (lead, follow) in spatial.lead.value.into_iter().zip(spatial.follow.value) {
            assert!(follow.abs() < lead.abs());
        }
    }

    #[test]
    fn a_lower_damping_ratio_overshoots() {
        let mut spring = Spring::new(100.0);
        spring.damping_ratio = 0.4;
        spring.target = 1.0;
        let mut peak = 0.0f32;
        for _ in 0..180 {
            spring.step(1.0 / 60.0);
            peak = peak.max(spring.value);
        }
        assert!(peak > 1.2);
    }
}
