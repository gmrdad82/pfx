use crate::Wave;
use crate::synth::Synth;

const TUNED_RATE: f64 = 44_100.0;
const BLOCK: usize = 512;
const EIGHT: [f32; 8] = [0.0, 3000.0, 2200.0, 1500.0, 1080.0, 700.0, 390.0, 190.0];
const SIXTEEN: [f32; 16] = [
    0.0, 5000.0, 4000.0, 3250.0, 2750.0, 2300.0, 2000.0, 1750.0, 1500.0, 1250.0, 1000.0, 750.0,
    540.0, 350.0, 195.0, 95.0,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bands {
    #[default]
    Eight,
    Sixteen,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Carrier<'a> {
    Wave { wave: Wave, hz: f32, seed: u64 },
    Buffer(&'a [f32]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vocoder {
    pub bands: Bands,
    pub output_db: f32,
    pub hi_thru: f32,
    pub hi_band: f32,
    pub envelope_ms: f32,
    pub q: f32,
    pub range_hz: f32,
}

impl Default for Vocoder {
    fn default() -> Self {
        Self {
            bands: Bands::Eight,
            output_db: 0.0,
            hi_thru: 0.4,
            hi_band: 0.4,
            envelope_ms: 30.199_518,
            q: 0.55,
            range_hz: 800.0,
        }
    }
}

struct Bank {
    gain: f32,
    thru: f32,
    high: f32,
    coef: Vec<[f32; 3]>,
    speed: Vec<f32>,
}

impl Bank {
    fn new(settings: &Vocoder, rate: u32) -> Self {
        let rate = f64::from(rate.max(1));
        let tpofs = std::f64::consts::TAU / rate;
        let pow10 = |x: f64| pfx_core::sim::pow(10.0, x);
        let output = ((f64::from(settings.output_db) + 20.0) / 40.0).clamp(0.0, 1.0);
        let q = f64::from(settings.q).clamp(0.0, 1.0);
        let gain = pow10(2.0 * output - 3.0 * q - 2.0);
        let mut thru = pow10(0.5 + 2.0 * output);
        let high = f64::from(settings.hi_band).clamp(0.0, 1.0).powi(3) * thru;
        thru *= f64::from(settings.hi_thru).clamp(0.0, 1.0).powi(3);
        let table: &[f32] = match settings.bands {
            Bands::Eight => &EIGHT,
            Bands::Sixteen => &SIXTEEN,
        };
        let bands = table.len();
        let ms = f64::from(settings.envelope_ms).max(1e-3);
        let envelope =
            ((pfx_core::sim::ln(ms) / std::f64::consts::LN_10 - 1.0) / 3.0).clamp(0.05, 1.0);
        let top = pow10(-1.7 - 2.7 * envelope);
        let floor = 0.022 / bands as f64;
        let per_rate = TUNED_RATE / rate;
        let scaled = |c: f64| (1.0 - pfx_core::sim::pow(1.0 - c.min(1.0), per_rate)) as f32;
        let mut speed = vec![scaled(0.5 * top)];
        speed.extend((1..bands).map(|i| scaled((0.025 - floor * i as f64).min(top))));
        let rr = 1.0 - pow10(-1.0 - 1.2 * q);
        let shift = f64::from(settings.range_hz.max(1.0)) / 400.0;
        let pole =
            |hz: f64| 2.0 * rr * (2.0 * rr * pfx_core::sim::cos(tpofs * hz) / (1.0 + rr * rr));
        let mut coef = vec![[0.0f32; 3]];
        coef.extend(table[1..].iter().map(|hz| {
            let hz = f64::from(*hz) * shift;
            [pole(hz) as f32, (-rr * rr) as f32, pole(hz * 0.96) as f32]
        }));
        Self {
            gain: gain as f32,
            thru: thru as f32,
            high: high as f32,
            coef,
            speed,
        }
    }
}

pub fn vocoder(
    modulator: &[f32],
    channels: usize,
    rate: u32,
    carrier: Carrier,
    settings: &Vocoder,
) -> Vec<f32> {
    let channels = channels.max(1);
    let frames = modulator.len().div_ceil(channels);
    let excitation = excitation(carrier, frames, rate);
    let bank = Bank::new(settings, rate);
    let mut out = vec![0.0f32; modulator.len()];
    for channel in 0..channels {
        let voice: Vec<f32> = modulator
            .iter()
            .skip(channel)
            .step_by(channels)
            .copied()
            .collect();
        let mut state = Voice::new(bank.coef.len());
        for (n, o) in state
            .run(&bank, &voice, &excitation)
            .into_iter()
            .enumerate()
        {
            out[n * channels + channel] = o;
        }
    }
    out
}

fn excitation(carrier: Carrier, frames: usize, rate: u32) -> Vec<f32> {
    match carrier {
        Carrier::Wave { wave, hz, seed } => {
            let mut synth = Synth::new(wave, hz, seed);
            (0..frames).map(|_| synth.next(1.0, rate.max(1))).collect()
        }
        Carrier::Buffer([]) => vec![0.0; frames],
        Carrier::Buffer(buffer) => (0..frames).map(|n| buffer[n % buffer.len()]).collect(),
    }
}

struct Voice {
    f: Vec<[f32; 12]>,
    k: u32,
    kout: f32,
}

impl Voice {
    fn new(bands: usize) -> Self {
        Self {
            f: vec![[0.0; 12]; bands],
            k: 0,
            kout: 0.0,
        }
    }

    fn suspend(&mut self) {
        for band in &mut self.f {
            band[3..12].fill(0.0);
        }
        self.kout = 0.0;
        self.k = 0;
    }

    fn run(&mut self, bank: &Bank, voice: &[f32], carrier: &[f32]) -> Vec<f32> {
        let (g, ht, hh) = (bank.gain, bank.thru, bank.high);
        let mut out = Vec::with_capacity(voice.len());
        for (voice, carrier) in voice.chunks(BLOCK).zip(carrier.chunks(BLOCK)) {
            let mut o = 0.0f32;
            let mut oo = self.kout;
            let mut k = self.k;
            let (top, rest) = self.f.split_first_mut().expect("a band");
            for (a, b) in voice.iter().zip(carrier) {
                let (mut a, b) = (*a, *b);
                let mut tmp = a - top[7];
                top[7] = a;
                a = tmp;
                tmp = tmp.abs();
                top[11] -= bank.speed[0] * (top[11] - tmp);
                o = top[11] * (ht * a + hh * (b - top[3]));
                top[3] = b;
                k = k.wrapping_add(1);
                if k & 1 != 0 {
                    oo = 0.0;
                    let aa = a + top[9] - top[8] - top[8];
                    top[9] = top[8];
                    top[8] = a;
                    let bb = b + top[5] - top[4] - top[4];
                    top[5] = top[4];
                    top[4] = b;
                    for ((f, c), speed) in
                        rest.iter_mut().zip(&bank.coef[1..]).zip(&bank.speed[1..])
                    {
                        let mut tmp = c[0] * f[3] + c[1] * f[4] + bb;
                        f[4] = f[3];
                        f[3] = tmp;
                        tmp += c[2] * f[5] + c[1] * f[6];
                        f[6] = f[5];
                        f[5] = tmp;
                        tmp = c[0] * f[7] + c[1] * f[8] + aa;
                        f[8] = f[7];
                        f[7] = tmp;
                        tmp += c[2] * f[9] + c[1] * f[10];
                        f[10] = f[9];
                        f[9] = tmp;
                        f[11] -= speed * (f[11] - tmp.abs());
                        oo += f[5] * f[11];
                    }
                }
                o += oo * g;
                out.push(o);
            }
            self.kout = oo;
            self.k = k & 1;
            if top[11].abs() < 1.0e-10 {
                top[11] = 0.0;
            }
            for f in rest.iter_mut() {
                if f[3].abs() < 1.0e-10 || f[7].abs() < 1.0e-10 {
                    f[3..12].fill(0.0);
                }
            }
            if o.abs() > 10.0 {
                self.suspend();
            }
        }
        out
    }
}

pub(crate) fn band_powers(mono: &[f32], rate: u32, hop: usize) -> Vec<Vec<f32>> {
    let bank = Bank::new(
        &Vocoder {
            bands: Bands::Sixteen,
            ..Vocoder::default()
        },
        rate,
    );
    let hop = hop.max(1);
    let bands = bank.coef.len();
    let mut f = vec![[0.0f32; 12]; bands];
    let mut k = 0u32;
    let mut powers = Vec::with_capacity(mono.len().div_ceil(hop));
    for chunk in mono.chunks(hop) {
        let mut power = vec![0.0f32; bands];
        for x in chunk {
            let (top, rest) = f.split_first_mut().expect("a band");
            let a = x - top[7];
            top[7] = *x;
            power[0] += a * a;
            k = k.wrapping_add(1);
            if k & 1 != 0 {
                let aa = a + top[9] - top[8] - top[8];
                top[9] = top[8];
                top[8] = a;
                for ((f, c), p) in rest.iter_mut().zip(&bank.coef[1..]).zip(&mut power[1..]) {
                    let mut tmp = c[0] * f[7] + c[1] * f[8] + aa;
                    f[8] = f[7];
                    f[7] = tmp;
                    tmp += c[2] * f[9] + c[1] * f[10];
                    f[10] = f[9];
                    f[9] = tmp;
                    *p += tmp * tmp;
                }
            }
        }
        powers.push(power);
    }
    powers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::tone_level;
    use crate::sound::Rng;

    #[test]
    fn the_vocoder_sings_at_the_carrier_pitch_and_follows_the_voice() {
        let rate = 48_000u32;
        let mut rng = Rng::new(5);
        let gate = |n: usize| (n / 12_000).is_multiple_of(2);
        let voice: Vec<f32> = (0..96_000)
            .map(|n| if gate(n) { 0.5 * rng.signed() } else { 0.0 })
            .collect();
        let carrier = Carrier::Wave {
            wave: Wave::Saw,
            hz: 150.0,
            seed: 0,
        };
        let settings = Vocoder {
            hi_thru: 0.0,
            hi_band: 0.0,
            ..Vocoder::default()
        };
        let out = vocoder(&voice, 1, rate, carrier, &settings);
        assert_eq!(out.len(), voice.len());
        assert_eq!(out, vocoder(&voice, 1, rate, carrier, &settings));
        let level = |range: std::ops::Range<usize>| {
            (out[range.clone()].iter().map(|s| s * s).sum::<f32>() / range.len() as f32).sqrt()
        };
        for start in [0usize, 24_000, 48_000, 72_000] {
            let on = level(start + 3_000..start + 12_000);
            let off = level(start + 15_000..start + 24_000);
            assert!(on > 0.01, "{on}");
            assert!(off < on * 0.05, "{on} {off}");
        }
        let held = &out[27_000..36_000];
        let harmonic = tone_level(held, rate as f32, 450.0);
        let between = tone_level(held, rate as f32, 525.0);
        assert!(harmonic > 8.0 * between, "{harmonic} {between}");
    }
}
