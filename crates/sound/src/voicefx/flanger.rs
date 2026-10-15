use super::line::Line;
use crate::DriveCurve;
use crate::drive::shape;
use crate::synth::sine;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flanger {
    pub delay: f32,
    pub depth: f32,
    pub rate: f32,
    pub feedback: f32,
    pub mix: f32,
}

impl Default for Flanger {
    fn default() -> Self {
        Self {
            delay: 0.005,
            depth: 0.005,
            rate: 0.1,
            feedback: 0.5,
            mix: 0.5,
        }
    }
}

pub fn flanger(samples: &[f32], channels: usize, rate: u32, settings: &Flanger) -> Vec<f32> {
    let channels = channels.max(1);
    let frames_per_second = rate.max(1) as f32;
    let shortest = settings.delay.max(0.0) * frames_per_second;
    let longest = shortest + settings.depth.max(0.0) * frames_per_second;
    let step = f64::from(settings.rate.max(0.0)) / f64::from(rate.max(1));
    let feedback = settings.feedback.clamp(-1.0, 1.0);
    let mix = settings.mix.clamp(0.0, 1.0);
    let mut lines: Vec<Line> = (0..channels).map(|_| Line::new(longest)).collect();
    let mut looped = vec![0.0f32; channels];
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(samples.len());
    for frame in samples.chunks(channels) {
        let swing = sine(phase) * 0.5 + 0.5;
        phase += step;
        phase -= phase.floor();
        let delay = shortest + (longest - shortest) * swing;
        for ((x, line), value) in frame.iter().zip(lines.iter_mut()).zip(looped.iter_mut()) {
            line.write(x + *value);
            let wet = line.read(delay, shortest);
            line.advance();
            *value = shape(DriveCurve::Tanh, feedback * wet);
            out.push((1.0 - mix) * x + mix * wet);
        }
    }
    out
}
