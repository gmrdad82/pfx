#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crush {
    pub bits: Option<u32>,
    pub hold: f32,
    pub average: bool,
    pub low_pass: Option<f32>,
}

impl Default for Crush {
    fn default() -> Self {
        Self {
            bits: Some(8),
            hold: 1.0,
            average: false,
            low_pass: None,
        }
    }
}

pub const MAX_BITS: u32 = 24;

pub fn crush(samples: &[f32], channels: usize, rate: u32, settings: &Crush) -> Vec<f32> {
    let channels = channels.max(1);
    let hold = if settings.hold.is_finite() {
        settings.hold.max(1.0)
    } else {
        1.0
    };
    let scale = settings
        .bits
        .map(|bits| (1u32 << (bits.clamp(1, MAX_BITS) - 1)) as f32);
    let post = settings.low_pass.map(|hz| {
        let o2 = filter_freq(hz.clamp(10.0, 0.49 * rate.max(1) as f32), rate.max(1));
        let i2 = (1.0 - o2) * (1.0 - o2);
        (i2 * i2, o2)
    });
    let mut states = vec![State::default(); channels];
    let mut count = hold - 1.0;
    let mut out = Vec::with_capacity(samples.len());
    for frame in samples.chunks(channels) {
        count += 1.0;
        let latch = count >= hold;
        if latch {
            count -= hold;
        }
        for (x, state) in frame.iter().zip(states.iter_mut()) {
            if settings.average {
                state.sum += x;
                state.taken += 1;
            } else {
                state.sum = *x;
                state.taken = 1;
            }
            if latch {
                let value = state.sum / state.taken as f32;
                state.held = match scale {
                    Some(scale) => (value * scale).trunc() / scale,
                    None => value,
                };
                state.sum = 0.0;
                state.taken = 0;
            }
            out.push(match post {
                Some((i2, o2)) => state.filter(i2, o2),
                None => state.held,
            });
        }
    }
    out
}

#[derive(Clone, Copy, Default)]
struct State {
    sum: f32,
    taken: u32,
    held: f32,
    b: [f32; 8],
}

impl State {
    fn filter(&mut self, i2: f32, o2: f32) -> f32 {
        let b = &mut self.b;
        b[0] = i2 * self.held + o2 * b[0];
        b[1] = b[0] + o2 * b[1];
        b[2] = b[1] + o2 * b[2];
        b[3] = b[2] + o2 * b[3];
        b[4] = i2 * b[3] + o2 * b[4];
        b[5] = b[4] + o2 * b[5];
        b[6] = b[5] + o2 * b[6];
        b[7] = b[6] + o2 * b[7];
        b[7]
    }
}

fn filter_freq(hz: f32, rate: u32) -> f32 {
    let r = 0.999f32;
    let j = r * r - 1.0;
    let k = 2.0 - 2.0 * r * r * pfx_core::sim::cosf(0.647 * hz / rate as f32);
    (pfx_core::sim::sqrtf(k * k - 4.0 * j * j) - k) / (2.0 * j)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| -1.0 + 2.0 * i as f32 / frames as f32)
            .collect()
    }

    #[test]
    fn the_crusher_lands_every_sample_on_its_bit_depth_steps() {
        let input = ramp(4_000);
        for bits in [1u32, 3, 8] {
            let out = crush(
                &input,
                1,
                48_000,
                &Crush {
                    bits: Some(bits),
                    ..Crush::default()
                },
            );
            assert_eq!(out.len(), input.len());
            let scale = (1u32 << (bits - 1)) as f32;
            let mut levels: Vec<i32> = out
                .iter()
                .map(|s| {
                    let k = s * scale;
                    assert_eq!(k, k.round(), "{bits} bits: {s}");
                    k as i32
                })
                .collect();
            levels.dedup();
            assert_eq!(levels.len() as u32, 1 << bits, "{bits} bits");
            assert!(
                out.iter()
                    .zip(&input)
                    .all(|(o, i)| (o - i).abs() <= 1.0 / scale)
            );
        }
    }

    #[test]
    fn sample_and_hold_repeats_each_sample_for_the_hold() {
        let input: Vec<f32> = (0..64)
            .flat_map(|i| [i as f32 / 64.0, -(i as f32) / 64.0])
            .collect();
        let out = crush(
            &input,
            2,
            48_000,
            &Crush {
                bits: None,
                hold: 4.0,
                ..Crush::default()
            },
        );
        assert_eq!(out.len(), input.len());
        for (block, chunk) in out.chunks(8).enumerate() {
            let first = (block * 4) as f32 / 64.0;
            for frame in chunk.chunks(2) {
                assert_eq!(frame, [first, -first]);
            }
        }
        let averaged = crush(
            &input,
            2,
            48_000,
            &Crush {
                bits: None,
                hold: 4.0,
                average: true,
                ..Crush::default()
            },
        );
        assert_eq!(averaged[8], (1.0 + 2.0 + 3.0 + 4.0) / 4.0 / 64.0);
    }
}
