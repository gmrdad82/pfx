use std::f32::consts::{FRAC_PI_2, TAU};

pub use crate::motion::{
    cubic_in, cubic_in_out, cubic_out, quad_in, quad_out, smootherstep, smoothstep,
};

const BACK: f32 = 1.70158;
const BACK_IN_OUT: f32 = BACK * 1.525;
const ELASTIC: f32 = 3.0;
const ELASTIC_IN_OUT: f32 = 4.5;

fn shaped(t: f32, curve: impl Fn(f32) -> f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        0.0
    } else if t >= 1.0 {
        1.0
    } else {
        curve(t)
    }
}

fn folded(t: f32, rising: impl Fn(f32) -> f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let u = if t < 0.5 { t } else { 1.0 - t };
    let y = rising(2.0 * u) * 0.5;
    if t < 0.5 { y } else { 1.0 - y }
}

pub fn linear(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

pub fn sine_in(t: f32) -> f32 {
    shaped(t, |t| 1.0 - (t * FRAC_PI_2).cos())
}

pub fn sine_out(t: f32) -> f32 {
    shaped(t, |t| (t * FRAC_PI_2).sin())
}

pub fn sine_in_out(t: f32) -> f32 {
    folded(t, sine_in)
}

pub fn quad_in_out(t: f32) -> f32 {
    folded(t, quad_in)
}

pub fn quart_in(t: f32) -> f32 {
    shaped(t, |t| {
        let s = t * t;
        s * s
    })
}

pub fn quart_out(t: f32) -> f32 {
    shaped(t, |t| {
        let u = 1.0 - t;
        let s = u * u;
        1.0 - s * s
    })
}

pub fn quart_in_out(t: f32) -> f32 {
    folded(t, quart_in)
}

pub fn quint_in(t: f32) -> f32 {
    shaped(t, |t| {
        let s = t * t;
        s * s * t
    })
}

pub fn quint_out(t: f32) -> f32 {
    shaped(t, |t| {
        let u = 1.0 - t;
        let s = u * u;
        1.0 - s * s * u
    })
}

pub fn quint_in_out(t: f32) -> f32 {
    folded(t, quint_in)
}

pub fn expo_in(t: f32) -> f32 {
    shaped(t, |t| (10.0 * t - 10.0).exp2())
}

pub fn expo_out(t: f32) -> f32 {
    shaped(t, |t| 1.0 - (-10.0 * t).exp2())
}

pub fn expo_in_out(t: f32) -> f32 {
    folded(t, expo_in)
}

pub fn circ_in(t: f32) -> f32 {
    shaped(t, |t| 1.0 - (1.0 - t * t).sqrt())
}

pub fn circ_out(t: f32) -> f32 {
    shaped(t, |t| {
        let u = t - 1.0;
        (1.0 - u * u).sqrt()
    })
}

pub fn circ_in_out(t: f32) -> f32 {
    folded(t, circ_in)
}

fn back_in_with(t: f32, overshoot: f32) -> f32 {
    shaped(t, |t| t * t * ((overshoot + 1.0) * t - overshoot))
}

pub fn back_in(t: f32) -> f32 {
    back_in_with(t, BACK)
}

pub fn back_out(t: f32) -> f32 {
    shaped(t, |t| {
        let u = t - 1.0;
        1.0 + u * u * ((BACK + 1.0) * u + BACK)
    })
}

pub fn back_in_out(t: f32) -> f32 {
    folded(t, |t| back_in_with(t, BACK_IN_OUT))
}

fn elastic_in_with(t: f32, period: f32) -> f32 {
    shaped(t, |t| {
        let phase = (10.0 * t - 10.0 - period * 0.25) * (TAU / period);
        -(10.0 * t - 10.0).exp2() * phase.sin()
    })
}

pub fn elastic_in(t: f32) -> f32 {
    elastic_in_with(t, ELASTIC)
}

pub fn elastic_out(t: f32) -> f32 {
    1.0 - elastic_in(1.0 - t.clamp(0.0, 1.0))
}

pub fn elastic_in_out(t: f32) -> f32 {
    folded(t, |t| elastic_in_with(t, ELASTIC_IN_OUT))
}

pub fn bounce_out(t: f32) -> f32 {
    shaped(t, |t| {
        const N: f32 = 7.5625;
        const D: f32 = 2.75;
        if t < 1.0 / D {
            N * t * t
        } else if t < 2.0 / D {
            let t = t - 1.5 / D;
            N * t * t + 0.75
        } else if t < 2.5 / D {
            let t = t - 2.25 / D;
            N * t * t + 0.9375
        } else {
            let t = t - 2.625 / D;
            N * t * t + 0.984375
        }
    })
}

pub fn bounce_in(t: f32) -> f32 {
    1.0 - bounce_out(1.0 - t.clamp(0.0, 1.0))
}

pub fn bounce_in_out(t: f32) -> f32 {
    folded(t, bounce_in)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Ease {
    #[default]
    Linear,
    SineIn,
    SineOut,
    SineInOut,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    QuartIn,
    QuartOut,
    QuartInOut,
    QuintIn,
    QuintOut,
    QuintInOut,
    ExpoIn,
    ExpoOut,
    ExpoInOut,
    CircIn,
    CircOut,
    CircInOut,
    BackIn,
    BackOut,
    BackInOut,
    ElasticIn,
    ElasticOut,
    ElasticInOut,
    BounceIn,
    BounceOut,
    BounceInOut,
}

type Curve = fn(f32) -> f32;

const TABLE: [(Ease, &str, Curve); 31] = [
    (Ease::Linear, "linear", linear),
    (Ease::SineIn, "sine_in", sine_in),
    (Ease::SineOut, "sine_out", sine_out),
    (Ease::SineInOut, "sine_in_out", sine_in_out),
    (Ease::QuadIn, "quad_in", quad_in),
    (Ease::QuadOut, "quad_out", quad_out),
    (Ease::QuadInOut, "quad_in_out", quad_in_out),
    (Ease::CubicIn, "cubic_in", cubic_in),
    (Ease::CubicOut, "cubic_out", cubic_out),
    (Ease::CubicInOut, "cubic_in_out", cubic_in_out),
    (Ease::QuartIn, "quart_in", quart_in),
    (Ease::QuartOut, "quart_out", quart_out),
    (Ease::QuartInOut, "quart_in_out", quart_in_out),
    (Ease::QuintIn, "quint_in", quint_in),
    (Ease::QuintOut, "quint_out", quint_out),
    (Ease::QuintInOut, "quint_in_out", quint_in_out),
    (Ease::ExpoIn, "expo_in", expo_in),
    (Ease::ExpoOut, "expo_out", expo_out),
    (Ease::ExpoInOut, "expo_in_out", expo_in_out),
    (Ease::CircIn, "circ_in", circ_in),
    (Ease::CircOut, "circ_out", circ_out),
    (Ease::CircInOut, "circ_in_out", circ_in_out),
    (Ease::BackIn, "back_in", back_in),
    (Ease::BackOut, "back_out", back_out),
    (Ease::BackInOut, "back_in_out", back_in_out),
    (Ease::ElasticIn, "elastic_in", elastic_in),
    (Ease::ElasticOut, "elastic_out", elastic_out),
    (Ease::ElasticInOut, "elastic_in_out", elastic_in_out),
    (Ease::BounceIn, "bounce_in", bounce_in),
    (Ease::BounceOut, "bounce_out", bounce_out),
    (Ease::BounceInOut, "bounce_in_out", bounce_in_out),
];

impl Ease {
    pub const ALL: [Ease; 31] = [
        Ease::Linear,
        Ease::SineIn,
        Ease::SineOut,
        Ease::SineInOut,
        Ease::QuadIn,
        Ease::QuadOut,
        Ease::QuadInOut,
        Ease::CubicIn,
        Ease::CubicOut,
        Ease::CubicInOut,
        Ease::QuartIn,
        Ease::QuartOut,
        Ease::QuartInOut,
        Ease::QuintIn,
        Ease::QuintOut,
        Ease::QuintInOut,
        Ease::ExpoIn,
        Ease::ExpoOut,
        Ease::ExpoInOut,
        Ease::CircIn,
        Ease::CircOut,
        Ease::CircInOut,
        Ease::BackIn,
        Ease::BackOut,
        Ease::BackInOut,
        Ease::ElasticIn,
        Ease::ElasticOut,
        Ease::ElasticInOut,
        Ease::BounceIn,
        Ease::BounceOut,
        Ease::BounceInOut,
    ];

    pub fn at(self, t: f32) -> f32 {
        (TABLE[self as usize].2)(t)
    }

    pub fn name(self) -> &'static str {
        TABLE[self as usize].1
    }

    pub fn from_name(name: &str) -> Option<Ease> {
        TABLE
            .iter()
            .find(|entry| entry.1 == name)
            .map(|entry| entry.0)
    }

    pub fn monotonic(self) -> bool {
        !matches!(
            self,
            Ease::BackIn
                | Ease::BackOut
                | Ease::BackInOut
                | Ease::ElasticIn
                | Ease::ElasticOut
                | Ease::ElasticInOut
                | Ease::BounceIn
                | Ease::BounceOut
                | Ease::BounceInOut
        )
    }

    pub fn symmetric(self) -> bool {
        matches!(
            self,
            Ease::Linear
                | Ease::SineInOut
                | Ease::QuadInOut
                | Ease::CubicInOut
                | Ease::QuartInOut
                | Ease::QuintInOut
                | Ease::ExpoInOut
                | Ease::CircInOut
                | Ease::BackInOut
                | Ease::ElasticInOut
                | Ease::BounceInOut
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 31] = [
        "linear",
        "sine_in",
        "sine_out",
        "sine_in_out",
        "quad_in",
        "quad_out",
        "quad_in_out",
        "cubic_in",
        "cubic_out",
        "cubic_in_out",
        "quart_in",
        "quart_out",
        "quart_in_out",
        "quint_in",
        "quint_out",
        "quint_in_out",
        "expo_in",
        "expo_out",
        "expo_in_out",
        "circ_in",
        "circ_out",
        "circ_in_out",
        "back_in",
        "back_out",
        "back_in_out",
        "elastic_in",
        "elastic_out",
        "elastic_in_out",
        "bounce_in",
        "bounce_out",
        "bounce_in_out",
    ];

    const SAMPLES: u32 = 4000;

    fn sweep(ease: Ease) -> impl Iterator<Item = (f32, f32)> {
        (0..=SAMPLES).map(move |i| {
            let t = i as f32 / SAMPLES as f32;
            (t, ease.at(t))
        })
    }

    #[test]
    fn the_name_table_is_exhaustive_and_in_order() {
        assert_eq!(Ease::ALL.len(), NAMES.len());
        assert_eq!(TABLE.len(), NAMES.len());
        for (index, (ease, name)) in Ease::ALL.iter().zip(NAMES).enumerate() {
            assert_eq!(*ease as usize, index);
            assert_eq!(TABLE[index].0, *ease);
            assert_eq!(ease.name(), name);
            assert_eq!(Ease::from_name(name), Some(*ease));
        }
        assert_eq!(Ease::from_name("Linear"), None);
        assert_eq!(Ease::from_name("sine"), None);
        assert_eq!(Ease::from_name(""), None);
        assert_eq!(Ease::default(), Ease::Linear);
        let mut sorted = NAMES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), NAMES.len());
    }

    #[test]
    fn the_table_calls_the_named_functions() {
        let pairs: [(Ease, Curve); 31] = [
            (Ease::Linear, linear),
            (Ease::SineIn, sine_in),
            (Ease::SineOut, sine_out),
            (Ease::SineInOut, sine_in_out),
            (Ease::QuadIn, quad_in),
            (Ease::QuadOut, quad_out),
            (Ease::QuadInOut, quad_in_out),
            (Ease::CubicIn, cubic_in),
            (Ease::CubicOut, cubic_out),
            (Ease::CubicInOut, cubic_in_out),
            (Ease::QuartIn, quart_in),
            (Ease::QuartOut, quart_out),
            (Ease::QuartInOut, quart_in_out),
            (Ease::QuintIn, quint_in),
            (Ease::QuintOut, quint_out),
            (Ease::QuintInOut, quint_in_out),
            (Ease::ExpoIn, expo_in),
            (Ease::ExpoOut, expo_out),
            (Ease::ExpoInOut, expo_in_out),
            (Ease::CircIn, circ_in),
            (Ease::CircOut, circ_out),
            (Ease::CircInOut, circ_in_out),
            (Ease::BackIn, back_in),
            (Ease::BackOut, back_out),
            (Ease::BackInOut, back_in_out),
            (Ease::ElasticIn, elastic_in),
            (Ease::ElasticOut, elastic_out),
            (Ease::ElasticInOut, elastic_in_out),
            (Ease::BounceIn, bounce_in),
            (Ease::BounceOut, bounce_out),
            (Ease::BounceInOut, bounce_in_out),
        ];
        for (ease, function) in pairs {
            for (t, y) in sweep(ease) {
                assert_eq!(y.to_bits(), function(t).to_bits(), "{}", ease.name());
            }
        }
    }

    #[test]
    fn every_curve_starts_at_zero_ends_at_one_and_clamps() {
        for ease in Ease::ALL {
            let name = ease.name();
            assert_eq!(ease.at(0.0), 0.0, "{name}");
            assert_eq!(ease.at(1.0), 1.0, "{name}");
            assert_eq!(ease.at(-0.4), 0.0, "{name}");
            assert_eq!(ease.at(-1e9), 0.0, "{name}");
            assert_eq!(ease.at(1.7), 1.0, "{name}");
            assert_eq!(ease.at(1e9), 1.0, "{name}");
            assert_eq!(ease.at(f32::INFINITY), 1.0, "{name}");
            assert_eq!(ease.at(f32::NEG_INFINITY), 0.0, "{name}");
            for (_, y) in sweep(ease) {
                assert!(y.is_finite(), "{name}");
            }
        }
    }

    #[test]
    fn the_monotonic_curves_never_step_back() {
        for ease in Ease::ALL {
            let mut previous = 0.0;
            for (t, y) in sweep(ease) {
                if ease.monotonic() {
                    assert!(
                        (0.0..=1.0).contains(&y) && y >= previous,
                        "{} at {t}",
                        ease.name()
                    );
                }
                previous = y;
            }
        }
        let wobbly = Ease::ALL.iter().filter(|e| !e.monotonic()).count();
        assert_eq!(wobbly, 9);
        for ease in Ease::ALL.iter().filter(|e| !e.monotonic()) {
            let mut previous = 0.0;
            let mut stepped_back = false;
            for (_, y) in sweep(*ease) {
                stepped_back |= y < previous;
                previous = y;
            }
            assert!(stepped_back, "{}", ease.name());
        }
    }

    #[test]
    fn in_and_out_mirror_each_other() {
        let pairs = [
            (Ease::SineIn, Ease::SineOut),
            (Ease::QuadIn, Ease::QuadOut),
            (Ease::CubicIn, Ease::CubicOut),
            (Ease::QuartIn, Ease::QuartOut),
            (Ease::QuintIn, Ease::QuintOut),
            (Ease::ExpoIn, Ease::ExpoOut),
            (Ease::CircIn, Ease::CircOut),
            (Ease::BackIn, Ease::BackOut),
            (Ease::ElasticIn, Ease::ElasticOut),
            (Ease::BounceIn, Ease::BounceOut),
        ];
        for (rising, falling) in pairs {
            for i in 1..SAMPLES {
                let t = i as f32 / SAMPLES as f32;
                let mirrored = 1.0 - rising.at(1.0 - t);
                let tolerance = if matches!(rising, Ease::ExpoIn | Ease::ElasticIn) {
                    1e-3
                } else {
                    2e-6
                };
                assert!(
                    (falling.at(t) - mirrored).abs() <= tolerance,
                    "{} at {t}",
                    falling.name()
                );
            }
        }
    }

    #[test]
    fn the_in_out_curves_are_point_symmetric_about_the_middle() {
        for ease in Ease::ALL.iter().filter(|e| e.symmetric()) {
            assert_eq!(ease.at(0.5), 0.5, "{}", ease.name());
            for i in 0..=SAMPLES / 2 {
                let t = i as f32 / SAMPLES as f32;
                let sum = ease.at(t) + ease.at(1.0 - t);
                assert!((sum - 1.0).abs() < 2e-6, "{} at {t}: {sum}", ease.name());
            }
        }
        let named = Ease::ALL.iter().filter(|e| e.symmetric()).count();
        assert_eq!(named, 11);
    }

    #[test]
    fn known_midpoints() {
        assert_eq!(quad_in_out(0.25), 0.125);
        assert_eq!(quad_in_out(0.75), 0.875);
        assert_eq!(quart_in(0.5), 0.0625);
        assert_eq!(quart_out(0.5), 0.9375);
        assert_eq!(quart_in_out(0.25), 0.03125);
        assert_eq!(quint_in(0.5), 0.03125);
        assert_eq!(quint_out(0.5), 0.96875);
        assert_eq!(quint_in_out(0.25), 0.015625);
        assert_eq!(expo_in(0.5), 2.0f32.powi(-5));
        assert_eq!(expo_out(0.5), 1.0 - 2.0f32.powi(-5));
        assert!((sine_in(0.5) - (1.0 - 0.5f32.sqrt())).abs() < 1e-6);
        assert!((sine_out(0.5) - 0.5f32.sqrt()).abs() < 1e-6);
        assert!((circ_in(0.6) - 0.2).abs() < 1e-6);
        assert!((circ_out(0.6) - (1.0 - 0.16f32).sqrt()).abs() < 1e-6);
        assert!((back_in(0.5) - 0.25 * (2.70158 * 0.5 - 1.70158)).abs() < 1e-6);
        assert!((bounce_out(0.5) - 0.765625).abs() < 1e-6);
        assert!((bounce_out(1.0 / 2.75) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn back_overshoots_by_about_a_tenth() {
        let low = sweep(Ease::BackIn).map(|(_, y)| y).fold(0.0f32, f32::min);
        assert!((-0.1001..-0.09).contains(&low), "{low}");
        assert!(sweep(Ease::BackIn).all(|(_, y)| y <= 1.0));

        let high = sweep(Ease::BackOut).map(|(_, y)| y).fold(1.0f32, f32::max);
        assert!((1.09..1.1001).contains(&high), "{high}");
        assert!(sweep(Ease::BackOut).all(|(_, y)| y >= 0.0));

        let low = sweep(Ease::BackInOut)
            .map(|(_, y)| y)
            .fold(0.0f32, f32::min);
        let high = sweep(Ease::BackInOut)
            .map(|(_, y)| y)
            .fold(1.0f32, f32::max);
        assert!((-0.1002..-0.09).contains(&low), "{low}");
        assert!((1.09..1.1002).contains(&high), "{high}");
    }

    #[test]
    fn elastic_overshoots_and_rings_down() {
        let low = sweep(Ease::ElasticIn)
            .map(|(_, y)| y)
            .fold(0.0f32, f32::min);
        assert!((-0.4..-0.3).contains(&low), "{low}");
        let high = sweep(Ease::ElasticOut)
            .map(|(_, y)| y)
            .fold(1.0f32, f32::max);
        assert!((1.3..1.4).contains(&high), "{high}");
        assert!(sweep(Ease::ElasticIn).all(|(_, y)| y <= 1.0));
        assert!(sweep(Ease::ElasticOut).all(|(_, y)| y >= 0.0));

        let low = sweep(Ease::ElasticInOut)
            .map(|(_, y)| y)
            .fold(0.0f32, f32::min);
        let high = sweep(Ease::ElasticInOut)
            .map(|(_, y)| y)
            .fold(1.0f32, f32::max);
        assert!((-0.2..-0.1).contains(&low), "{low}");
        assert!((1.1..1.2).contains(&high), "{high}");

        let mut peaks = Vec::new();
        let mut previous = [0.0f32; 2];
        for (_, y) in sweep(Ease::ElasticOut) {
            if previous[1] > previous[0] && previous[1] > y && previous[1] > 1.0 {
                peaks.push(previous[1] - 1.0);
            }
            previous = [previous[1], y];
        }
        assert!(peaks.len() >= 3, "{peaks:?}");
        assert!(peaks.windows(2).all(|pair| pair[1] < pair[0]));
    }

    #[test]
    fn bounce_stays_inside_the_unit_range() {
        for ease in [Ease::BounceIn, Ease::BounceOut, Ease::BounceInOut] {
            for (t, y) in sweep(ease) {
                assert!((0.0..=1.0).contains(&y), "{} at {t}: {y}", ease.name());
            }
        }
        let mut touches = 0;
        let mut previous = [0.0f32; 2];
        for (_, y) in sweep(Ease::BounceOut) {
            if previous[1] < previous[0] && previous[1] < y {
                touches += 1;
            }
            previous = [previous[1], y];
        }
        assert_eq!(touches, 3);
    }

    #[test]
    fn the_older_easings_keep_their_results() {
        assert_eq!(Ease::QuadIn.at(0.5), 0.25);
        assert_eq!(Ease::QuadOut.at(0.5), 0.75);
        assert_eq!(Ease::CubicIn.at(0.5), 0.125);
        assert_eq!(Ease::CubicOut.at(0.5), 0.875);
        assert_eq!(Ease::CubicInOut.at(0.25), 4.0 * 0.25 * 0.25 * 0.25);
        assert_eq!(smoothstep(0.25), 0.15625);
        assert_eq!(smootherstep(0.5), 0.5);
        assert_eq!(linear(0.3), 0.3);
    }
}
