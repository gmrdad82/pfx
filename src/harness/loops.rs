use super::shots::Closure;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    One(usize),
    Blend(usize, usize, f32),
}

fn frames(seconds: f64, fps: u32) -> usize {
    (seconds * fps as f64).round().max(1.0) as usize
}

pub fn picks(closure: Closure, seconds: f64, master: u32, fps: u32) -> Vec<Pick> {
    let step = (master / fps).max(1) as usize;
    let at = |i: usize| i * step;
    match closure {
        Closure::Open => (0..frames(seconds, fps))
            .map(|i| Pick::One(at(i)))
            .collect(),
        Closure::Palindrome => {
            let half = frames(seconds / 2.0, fps) + 1;
            (0..half)
                .chain((1..half - 1).rev())
                .map(|i| Pick::One(at(i)))
                .collect()
        }
        Closure::Crossfade(fade) => {
            let (out, fade) = (
                frames(seconds, fps),
                frames(fade, fps).min(frames(seconds, fps) / 2),
            );
            (0..out)
                .map(|k| {
                    let main = fade + k;
                    if k + fade < out {
                        Pick::One(at(main))
                    } else {
                        let m = k + fade - out;
                        Pick::Blend(at(main), at(m), (m + 1) as f32 / (fade + 1) as f32)
                    }
                })
                .collect()
        }
    }
}

pub fn needed(closure: Closure, seconds: f64, master: u32, rates: &[u32]) -> usize {
    rates
        .iter()
        .flat_map(|&fps| picks(closure, seconds, master, fps))
        .map(|p| match p {
            Pick::One(i) => i,
            Pick::Blend(a, b, _) => a.max(b),
        })
        .max()
        .map_or(1, |m| m + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crossfade_loop_hands_its_last_frame_back_to_its_first() {
        let p = picks(Closure::Crossfade(0.5), 11.0, 30, 30);
        assert_eq!(p.len(), 330);
        assert_eq!(p[0], Pick::One(15));
        assert_eq!(p[314], Pick::One(329));
        assert_eq!(p[315], Pick::Blend(330, 0, 1.0 / 16.0));
        assert_eq!(p[329], Pick::Blend(344, 14, 15.0 / 16.0));
        assert_eq!(needed(Closure::Crossfade(0.5), 11.0, 30, &[30]), 345);
    }

    #[test]
    fn thirty_frames_are_the_even_frames_of_a_sixty_master() {
        let p30 = picks(Closure::Crossfade(0.5), 11.0, 60, 30);
        let p60 = picks(Closure::Crossfade(0.5), 11.0, 60, 60);
        assert_eq!(p30.len(), 330);
        assert_eq!(p60.len(), 660);
        assert_eq!(p30[0], Pick::One(30));
        assert_eq!(p60[0], Pick::One(30));
        assert!(p30.iter().all(|p| match p {
            Pick::One(i) => i % 2 == 0,
            Pick::Blend(a, b, _) => a % 2 == 0 && b % 2 == 0,
        }));
        assert_eq!(needed(Closure::Crossfade(0.5), 11.0, 60, &[30, 60]), 690);
    }

    #[test]
    fn a_palindrome_plays_forward_then_back_without_repeating_its_ends() {
        let p = picks(Closure::Palindrome, 2.0, 30, 30);
        assert_eq!(p.len(), 60);
        assert_eq!(p[0], Pick::One(0));
        assert_eq!(p[30], Pick::One(30));
        assert_eq!(p[31], Pick::One(29));
        assert_eq!(p[59], Pick::One(1));
    }
}
