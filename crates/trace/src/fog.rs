use pfx_load::scene::OutdoorHaze;

pub const ABSORPTION: f32 = 0.3;
pub const SCATTERING: f32 = 0.3;
pub const BOUNCES: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    pub height: f32,
    pub falloff: f32,
    pub density: f32,
    pub reach: f32,
    pub shafts: f32,
    pub color: [f32; 3],
    pub phase: f32,
    pub bounces: u32,
}

impl Default for Fog {
    fn default() -> Self {
        Fog::outdoor(&OutdoorHaze::default())
    }
}

impl Fog {
    pub fn outdoor(haze: &OutdoorHaze) -> Fog {
        Fog {
            height: haze.height,
            falloff: haze.falloff,
            density: haze.density,
            reach: haze.reach,
            shafts: haze.shafts,
            color: haze.color,
            phase: haze.phase,
            bounces: haze.bounces,
        }
    }

    pub fn valid(&self) -> bool {
        [
            self.height,
            self.falloff,
            self.density,
            self.reach,
            self.shafts,
            self.phase,
        ]
        .into_iter()
        .chain(self.color)
        .all(f32::is_finite)
            && self.falloff >= 0.0
            && self.density >= 0.0
            && self.reach >= 0.0
            && (0.0..=1.0).contains(&self.shafts)
            && self.color.iter().all(|&c| c >= 0.0)
            && (-0.9..=0.9).contains(&self.phase)
            && self.bounces <= 64
    }

    pub fn extinction(&self) -> f32 {
        self.density * (ABSORPTION + SCATTERING)
    }

    pub fn albedo(&self) -> [f32; 3] {
        self.color
            .map(|c| (c * SCATTERING / (ABSORPTION + SCATTERING)).min(1.0))
    }

    pub(crate) fn rows(&self) -> [[f32; 4]; 3] {
        let [r, g, b] = self.albedo();
        [
            [self.height, self.falloff, self.extinction(), self.reach],
            [r, g, b, self.phase],
            [self.shafts, self.bounces as f32, 0.0, 0.0],
        ]
    }
}
