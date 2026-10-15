use crate::synth::sine;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub hz: f32,
    pub mix: f32,
    pub feedback: f32,
}

impl Default for Ring {
    fn default() -> Self {
        Self {
            hz: 1000.0,
            mix: 1.0,
            feedback: 0.0,
        }
    }
}

pub fn ring(samples: &[f32], channels: usize, rate: u32, settings: &Ring) -> Vec<f32> {
    let channels = channels.max(1);
    let step = f64::from(settings.hz.max(0.0)) / f64::from(rate.max(1));
    let feedback = 0.95 * settings.feedback.clamp(0.0, 1.0);
    let mix = settings.mix.clamp(0.0, 1.0);
    let mut phase = 0.0f64;
    let mut previous = vec![0.0f32; channels];
    let mut out = Vec::with_capacity(samples.len());
    for frame in samples.chunks(channels) {
        let carrier = sine(phase);
        phase += step;
        phase -= phase.floor();
        for (x, held) in frame.iter().zip(previous.iter_mut()) {
            *held = (feedback * *held + x) * carrier;
            out.push((1.0 - mix) * x + mix * *held);
        }
    }
    out
}
