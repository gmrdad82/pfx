mod gate;
mod harmonic;
mod preview;
mod spectrum;
mod transient;

use crate::Clip;
use crate::voicefx::fft::Complex;
use spectrum::Grid;

pub use harmonic::{HIGH_HZ, LOW_HZ};
pub use preview::Preview;

const MAIN_SECONDS: f32 = 0.085;
const HARMONIC_SECONDS: f32 = 0.17;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Harmonic {
    pub low_hz: f32,
    pub high_hz: f32,
}

impl Default for Harmonic {
    fn default() -> Self {
        Harmonic {
            low_hz: LOW_HZ,
            high_hz: HIGH_HZ,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gate {
    pub noise: Option<(f32, f32)>,
    pub reduce_db: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Options {
    pub harmonic: Option<Harmonic>,
    pub transients: bool,
    pub gate: Option<Gate>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HarmonicReport {
    pub low_hz: f32,
    pub median_hz: f32,
    pub high_hz: f32,
    pub voiced: f32,
    pub removed_db: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub start: f32,
    pub end: f32,
    pub removed_db: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GateReport {
    pub noise: (f32, f32),
    pub auto: bool,
    pub reduce_db: Option<f32>,
    pub removed_db: f32,
    pub noise_removed_db: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub seconds: f32,
    pub rate: u32,
    pub channels: u16,
    pub input_db: f32,
    pub output_db: f32,
    pub harmonic: Option<HarmonicReport>,
    pub events: Option<Vec<Event>>,
    pub gate: Option<GateReport>,
}

#[derive(Clone, Debug)]
pub(crate) struct Pass {
    pub(crate) grid: Grid,
    pub(crate) gains: Vec<f32>,
    pub(crate) fill: Vec<(usize, Complex)>,
}

impl Pass {
    pub(crate) fn gain_at(&self, seconds: f32, hz: f32) -> f32 {
        let g = self.grid;
        let t = (g.frame(seconds).round().max(0.0) as usize).min(g.frames - 1);
        let k = (g.bin(hz).round().max(0.0) as usize).min(g.bins - 1);
        self.gains[t * g.bins + k]
    }

    fn run(&self, signal: &[f32], filled: bool) -> Vec<f32> {
        let fill: &[(usize, Complex)] = if filled { &self.fill } else { &[] };
        spectrum::synthesise(&spectrum::analyse(signal, self.grid), &self.gains, fill)
    }
}

#[derive(Clone, Debug)]
pub struct Masks {
    rate: u32,
    len: usize,
    pub(crate) passes: Vec<Pass>,
}

impl Masks {
    pub fn apply(&self, clip: &Clip) -> Result<Clip, String> {
        self.replay(clip, false)
    }

    fn replay(&self, clip: &Clip, filled: bool) -> Result<Clip, String> {
        let channels = clip.channels.max(1) as usize;
        let len = clip.samples.len() / channels;
        if clip.rate != self.rate || len != self.len {
            return Err(format!(
                "the masks were made for {} frames at {} Hz, not {len} at {} Hz",
                self.len, self.rate, clip.rate
            ));
        }
        let mut out = clip.clone();
        for c in 0..channels {
            let mut signal: Vec<f32> = (0..len).map(|i| clip.samples[i * channels + c]).collect();
            for pass in &self.passes {
                signal = pass.run(&signal, filled);
            }
            for (i, s) in signal.into_iter().enumerate() {
                out.samples[i * channels + c] = s;
            }
        }
        Ok(out)
    }
}

pub struct Cleaned {
    pub clip: Clip,
    pub report: Report,
    pub masks: Masks,
    input: Vec<f32>,
}

impl Cleaned {
    pub fn preview(&self) -> Preview {
        preview::draw(
            &self.input,
            &self.masks,
            size(self.masks.rate, MAIN_SECONDS),
        )
    }
}

fn mono(clip: &Clip) -> Vec<f32> {
    let channels = clip.channels.max(1) as usize;
    clip.samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn rms_db(samples: &[f32]) -> f32 {
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    let mean = sum / samples.len().max(1) as f64;
    (10.0 * (mean + 1e-30).log10()) as f32
}

fn ratio_db(before: f64, after: f64) -> f32 {
    (10.0 * ((before + 1e-30) / (after + 1e-30)).log10()) as f32
}

fn energy(mags: &[f32], gains: Option<&[f32]>, range: std::ops::Range<usize>) -> f64 {
    match gains {
        Some(g) => range.map(|i| f64::from(mags[i] * g[i]).powi(2)).sum(),
        None => range.map(|i| f64::from(mags[i]).powi(2)).sum(),
    }
}

fn size(rate: u32, seconds: f32) -> usize {
    ((seconds * rate as f32) as usize)
        .next_power_of_two()
        .max(256)
}

fn check(clip: &Clip, options: &Options) -> Result<f32, String> {
    if clip.rate == 0 || clip.channels == 0 || clip.samples.is_empty() {
        return Err("the input holds no audio".into());
    }
    if options.harmonic.is_none() && !options.transients && options.gate.is_none() {
        return Err("nothing to do: choose --keep harmonic, --remove transients or --gate".into());
    }
    let seconds = clip.seconds();
    let nyquist = clip.rate as f32 / 2.0;
    if let Some(h) = options.harmonic
        && !(h.low_hz > 0.0 && h.low_hz < h.high_hz && h.high_hz < nyquist)
    {
        return Err(format!(
            "the fundamental's range {}..{} Hz must rise from above 0 to below {nyquist} Hz",
            h.low_hz, h.high_hz
        ));
    }
    if let Some(gate) = options.gate {
        if let Some((start, end)) = gate.noise {
            let shortest = gate::N_FFT as f32 / clip.rate as f32;
            if !(start >= 0.0 && end <= seconds + 1e-6 && end - start >= shortest) {
                return Err(format!(
                    "the noise print {start}..{end} s must lie inside the {seconds:.3} s input and last at least {shortest:.3} s"
                ));
            }
        }
        if let Some(db) = gate.reduce_db
            && !(db > 0.0 && db.is_finite())
        {
            return Err(format!("the gate's reduction {db} dB must be above 0"));
        }
    }
    Ok(seconds)
}

fn total(mags: &[f32], gains: Option<&[f32]>) -> f64 {
    energy(mags, gains, 0..mags.len())
}

pub fn clean(clip: &Clip, options: &Options) -> Result<Cleaned, String> {
    let seconds = check(clip, options)?;
    let input = mono(clip);
    let rate = clip.rate;
    let mut passes = Vec::new();
    let mut harmonic_report = None;
    if let Some(h) = options.harmonic {
        let grid = Grid::new(size(rate, HARMONIC_SECONDS), rate, input.len());
        let mags = spectrum::analyse(&input, grid).magnitudes();
        let track = harmonic::track(&mags, grid, h.low_hz, h.high_hz);
        let gains = harmonic::comb(&track, grid);
        let mut sorted = track.f0.clone();
        sorted.sort_by(f32::total_cmp);
        harmonic_report = Some(HarmonicReport {
            low_hz: sorted[0],
            median_hz: sorted[sorted.len() / 2],
            high_hz: sorted[sorted.len() - 1],
            voiced: track.voiced,
            removed_db: ratio_db(total(&mags, None), total(&mags, Some(&gains))),
        });
        passes.push(Pass {
            grid,
            gains,
            fill: Vec::new(),
        });
    }
    let mut events = None;
    if options.transients {
        let grid = Grid::new(size(rate, MAIN_SECONDS), rate, input.len());
        let spectra = spectrum::analyse(&input, grid);
        let mags = spectra.magnitudes();
        let found = transient::find(&spectra, &mags);
        let bins = grid.bins;
        events = Some(
            found
                .spans
                .iter()
                .map(|span| {
                    let range = span.first * bins..(span.last + 1) * bins;
                    Event {
                        start: grid.seconds(span.first as f32).max(0.0),
                        end: grid.seconds(span.last as f32).min(seconds),
                        removed_db: ratio_db(
                            energy(&mags, None, range.clone()),
                            energy(&mags, Some(&found.gains), range),
                        ),
                    }
                })
                .collect(),
        );
        passes.push(Pass {
            grid,
            gains: found.gains,
            fill: found.fill,
        });
    }
    let mut cleaned = input.clone();
    for pass in &passes {
        cleaned = pass.run(&cleaned, true);
    }
    let mut gate_report = None;
    if let Some(gate) = options.gate {
        let (from, to, auto) = match gate.noise {
            Some((start, end)) => (
                (start * rate as f32) as usize,
                ((end * rate as f32) as usize).min(input.len()),
                false,
            ),
            None => {
                let (a, b) = gate::quietest(&input, rate);
                (a, b, true)
            }
        };
        let threshold = gate::threshold(&cleaned[from..to], rate);
        let grid = Grid::new(gate::N_FFT, rate, cleaned.len());
        let spectra = spectrum::analyse(&cleaned, grid);
        let mags = spectra.magnitudes();
        let prop = gate
            .reduce_db
            .map(|db| 1.0 - 10f32.powf(-db / 20.0))
            .unwrap_or(gate::PROP_DECREASE);
        let gains = gate::mask(&mags, grid, &threshold, prop);
        let first = (grid.frame(from as f32 / rate as f32).round() as usize).min(grid.frames);
        let last = (grid.frame(to as f32 / rate as f32).round() as usize).clamp(first, grid.frames);
        let print = first * grid.bins..last * grid.bins;
        gate_report = Some(GateReport {
            noise: (from as f32 / rate as f32, to as f32 / rate as f32),
            auto,
            reduce_db: gate.reduce_db,
            removed_db: ratio_db(total(&mags, None), total(&mags, Some(&gains))),
            noise_removed_db: ratio_db(
                energy(&mags, None, print.clone()),
                energy(&mags, Some(&gains), print),
            ),
        });
        cleaned = spectrum::synthesise(&spectra, &gains, &[]);
        passes.push(Pass {
            grid,
            gains,
            fill: Vec::new(),
        });
    }
    let masks = Masks {
        rate,
        len: input.len(),
        passes,
    };
    let out = if clip.channels == 1 {
        Clip {
            rate,
            channels: 1,
            samples: cleaned,
        }
    } else {
        masks.replay(clip, true)?
    };
    let report = Report {
        seconds,
        rate,
        channels: clip.channels,
        input_db: rms_db(&clip.samples),
        output_db: rms_db(&out.samples),
        harmonic: harmonic_report,
        events,
        gate: gate_report,
    };
    Ok(Cleaned {
        clip: out,
        report,
        masks,
        input,
    })
}
