const FEEDBACK_MAX: f32 = 0.95;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delay {
    pub frames: u64,
    pub feedback: f32,
    pub wet: f32,
}

impl Delay {
    pub fn process(&self, buf: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let len = usize::try_from(self.frames).unwrap_or(0);
        if len == 0 {
            return;
        }
        let feedback = if self.feedback.is_nan() {
            0.0
        } else {
            self.feedback.clamp(0.0, FEEDBACK_MAX)
        };
        let wet = if self.wet.is_nan() {
            0.0
        } else {
            self.wet.max(0.0)
        };
        let mut line = vec![0.0f32; len * channels];
        let mut at = 0usize;
        for frame in buf.chunks_exact_mut(channels) {
            let tap = &mut line[at * channels..(at + 1) * channels];
            for (sample, held) in frame.iter_mut().zip(tap.iter_mut()) {
                let echo = *held;
                *held = *sample + echo * feedback;
                *sample += echo * wet;
            }
            at += 1;
            if at == len {
                at = 0;
            }
        }
    }
}
