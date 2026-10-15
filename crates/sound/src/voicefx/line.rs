use crate::interp::hermite;

const MIN_DELAY: f32 = 1.000_01;

pub(crate) struct Line {
    buffer: Vec<f32>,
    mask: usize,
    at: usize,
    longest: f32,
}

impl Line {
    pub(crate) fn new(longest: f32) -> Self {
        let longest = longest.max(MIN_DELAY);
        let len = (longest.ceil() as usize + 3).next_power_of_two();
        Self {
            buffer: vec![0.0; len],
            mask: len - 1,
            at: 0,
            longest,
        }
    }

    pub(crate) fn write(&mut self, x: f32) {
        self.buffer[self.at] = x;
    }

    pub(crate) fn read(&self, delay: f32, shortest: f32) -> f32 {
        let delay = delay.clamp(shortest.max(MIN_DELAY), self.longest);
        let floor = delay as usize;
        let i1 = self.at.wrapping_sub(floor) & self.mask;
        let i0 = i1.wrapping_add(1) & self.mask;
        let i2 = i1.wrapping_sub(1) & self.mask;
        let i3 = i1.wrapping_sub(2) & self.mask;
        hermite(
            self.buffer[i0],
            self.buffer[i1],
            self.buffer[i2],
            self.buffer[i3],
            delay - floor as f32,
        )
    }

    pub(crate) fn advance(&mut self) {
        self.at = (self.at + 1) & self.mask;
    }
}
