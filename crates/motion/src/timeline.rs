use pfx_core::rate::Rate;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timeline {
    pub fps: Rate,
    pub frames: u32,
    pub size: [u32; 2],
    pub seed: u64,
}

impl Timeline {
    pub const FPS: Rate = Rate::whole(60);
    pub const SIZE: [u32; 2] = [1080, 1920];
    pub const MAX_FPS: u32 = 240;
    pub const MAX_FRAMES: u32 = 1_000_000;
    pub const MAX_SIDE: u32 = 16384;

    pub fn frames(&self) -> u32 {
        self.frames
    }

    pub fn time(&self, frame: u32) -> f64 {
        self.fps.time(u64::from(frame))
    }

    pub fn seconds(&self) -> f64 {
        self.time(self.frames)
    }

    pub fn step(&self) -> f64 {
        self.fps.step()
    }

    pub fn aspect(&self) -> f64 {
        f64::from(self.size[0]) / f64::from(self.size[1].max(1))
    }

    pub fn holds(&self, time: f64) -> bool {
        time.is_finite() && (0.0..=self.seconds()).contains(&time)
    }
}
