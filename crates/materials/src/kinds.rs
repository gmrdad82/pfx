use crate::material::{NoiseKind, NoiseLayer, PARAMS};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Fibre {
    pub across: f32,
    pub along: f32,
    pub fine: f32,
    pub mottle: f32,
    pub ripple: f32,
    pub swell: f32,
    pub fibre_gain: f32,
    pub mottle_gain: f32,
    pub ripple_gain: f32,
    pub swell_gain: f32,
}

impl Fibre {
    pub const SAMPLE: Self = Self {
        across: 2400.0,
        along: 800.0,
        fine: 6000.0,
        mottle: 120.0,
        ripple: 4800.0,
        swell: 80.0,
        fibre_gain: 0.06,
        mottle_gain: 0.04,
        ripple_gain: 0.07,
        swell_gain: 0.01,
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::Fibre, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            self.across,
            self.along,
            self.fine,
            self.mottle,
            self.ripple,
            self.swell,
            self.fibre_gain,
            self.mottle_gain,
            self.ripple_gain,
            self.swell_gain,
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            across: p[0],
            along: p[1],
            fine: p[2],
            mottle: p[3],
            ripple: p[4],
            swell: p[5],
            fibre_gain: p[6],
            mottle_gain: p[7],
            ripple_gain: p[8],
            swell_gain: p[9],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Crinkle {
    pub frequencies: [f32; 2],
    pub gains: [f32; 2],
}

impl Crinkle {
    pub const SAMPLE: Self = Self {
        frequencies: [300.0, 650.0],
        gains: [0.15, 0.06],
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::Crinkle, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            self.frequencies[0],
            self.frequencies[1],
            self.gains[0],
            self.gains[1],
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            frequencies: [p[0], p[1]],
            gains: [p[2], p[3]],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PlankWood {
    pub width: f32,
    pub rings: f32,
    pub warp: f32,
    pub figure: [f32; 2],
    pub tone: f32,
    pub figure_depth: f32,
    pub seam_floor: f32,
    pub roughness: f32,
    pub jitter: f32,
}

impl PlankWood {
    pub const SAMPLE: Self = Self {
        width: 0.15,
        rings: 60.0,
        warp: 0.03,
        figure: [80.0, 600.0],
        tone: 0.3,
        figure_depth: 0.4,
        seam_floor: 0.4,
        roughness: 0.45,
        jitter: 2.5,
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::PlankWood, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            self.width,
            self.rings,
            self.warp,
            self.figure[0],
            self.figure[1],
            self.tone,
            self.figure_depth,
            self.seam_floor,
            self.roughness,
            self.jitter,
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            width: p[0],
            rings: p[1],
            warp: p[2],
            figure: [p[3], p[4]],
            tone: p[5],
            figure_depth: p[6],
            seam_floor: p[7],
            roughness: p[8],
            jitter: p[9],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct WallMottle {
    pub broad: [f32; 2],
    pub fine: [f32; 2],
    pub swell: [f32; 2],
    pub broad_gain: f32,
    pub fine_gain: f32,
    pub swell_gains: [f32; 2],
}

impl WallMottle {
    pub const SAMPLE: Self = Self {
        broad: [2.5, 7.0],
        fine: [400.0, 1400.0],
        swell: [250.0, 36.0],
        broad_gain: 0.12,
        fine_gain: 0.05,
        swell_gains: [0.016, 0.006],
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::WallMottle, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            self.broad[0],
            self.broad[1],
            self.fine[0],
            self.fine[1],
            self.swell[0],
            self.swell[1],
            self.broad_gain,
            self.fine_gain,
            self.swell_gains[0],
            self.swell_gains[1],
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            broad: [p[0], p[1]],
            fine: [p[2], p[3]],
            swell: [p[4], p[5]],
            broad_gain: p[6],
            fine_gain: p[7],
            swell_gains: [p[8], p[9]],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Grime {
    pub frequencies: [f32; 2],
    pub dirty: [f32; 3],
    pub clean: [f32; 3],
    pub roughness: [f32; 2],
}

impl Grime {
    pub const SAMPLE: Self = Self {
        frequencies: [160.0, 800.0],
        dirty: [0.9, 0.84, 0.76],
        clean: [1.03, 1.02, 1.0],
        roughness: [1.2, 0.65],
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::Grime, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            self.frequencies[0],
            self.frequencies[1],
            self.dirty[0],
            self.dirty[1],
            self.dirty[2],
            self.clean[0],
            self.clean[1],
            self.clean[2],
            self.roughness[0],
            self.roughness[1],
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            frequencies: [p[0], p[1]],
            dirty: [p[2], p[3], p[4]],
            clean: [p[5], p[6], p[7]],
            roughness: [p[8], p[9]],
        }
    }

    pub fn faded_roughness(self) -> f32 {
        let sum: f32 = WEAR_QUANTILES
            .iter()
            .map(|&wear| {
                let factor =
                    (self.roughness[0] + (self.roughness[1] - self.roughness[0]) * wear).max(1e-4);
                1.0 / (factor * factor)
            })
            .sum();
        (sum / WEAR_QUANTILES.len() as f32).sqrt().recip()
    }
}

pub const WEAR_QUANTILES: [f32; 32] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0047, 0.0180, 0.0371, 0.0603, 0.0869, 0.1155, 0.1459, 0.1784,
    0.2125, 0.2483, 0.2857, 0.3249, 0.3639, 0.4048, 0.4464, 0.4891, 0.5334, 0.5792, 0.6260, 0.6739,
    0.7232, 0.7750, 0.8274, 0.8814, 0.9362, 0.9838, 1.0,
];

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Scratch {
    pub line: f32,
    pub smudge: [f32; 2],
    pub speck: f32,
}

impl Scratch {
    pub const SAMPLE: Self = Self {
        line: 800.0,
        smudge: [20.0, 55.0],
        speck: 2400.0,
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::Scratch, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[self.line, self.smudge[0], self.smudge[1], self.speck])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            line: p[0],
            smudge: [p[1], p[2]],
            speck: p[3],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CoatWobble {
    pub frequencies: [f32; 3],
    pub weights: [f32; 2],
}

impl CoatWobble {
    pub const SAMPLE: Self = Self {
        frequencies: [800.0, 48.0, 20.0],
        weights: [0.5, 1.0],
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::CoatWobble, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            self.frequencies[0],
            self.frequencies[1],
            self.frequencies[2],
            self.weights[0],
            self.weights[1],
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            frequencies: [p[0], p[1], p[2]],
            weights: [p[3], p[4]],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Crack {
    pub cell: f32,
    pub width: f32,
    pub warp: f32,
    pub warp_frequency: f32,
    pub density: f32,
    pub spread: f32,
    pub darkness: f32,
    pub roughness: f32,
    pub randomness: f32,
}

impl Crack {
    pub const SAMPLE: Self = Self {
        cell: 1.6,
        width: 0.006,
        warp: 0.18,
        warp_frequency: 1.5,
        density: 0.45,
        spread: 0.15,
        darkness: 0.6,
        roughness: 0.95,
        randomness: 1.0,
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::Crack, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            self.cell,
            self.width,
            self.warp,
            self.warp_frequency,
            self.density,
            self.spread,
            self.darkness,
            self.roughness,
            self.randomness,
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            cell: p[0],
            width: p[1],
            warp: p[2],
            warp_frequency: p[3],
            density: p[4],
            spread: p[5],
            darkness: p[6],
            roughness: p[7],
            randomness: p[8],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Track {
    pub period: f32,
    pub offset: f32,
    pub gauge: f32,
    pub band: f32,
    pub wander: f32,
    pub wander_frequency: f32,
    pub streak: [f32; 2],
    pub tint: f32,
    pub polish: f32,
}

impl Track {
    pub const SAMPLE: Self = Self {
        period: 3.2,
        offset: 1.6,
        gauge: 0.8,
        band: 0.3,
        wander: 0.12,
        wander_frequency: 0.04,
        streak: [6.0, 0.35],
        tint: 0.8,
        polish: 0.75,
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::Track, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            self.period,
            self.offset,
            self.gauge,
            self.band,
            self.wander,
            self.wander_frequency,
            self.streak[0],
            self.streak[1],
            self.tint,
            self.polish,
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            period: p[0],
            offset: p[1],
            gauge: p[2],
            band: p[3],
            wander: p[4],
            wander_frequency: p[5],
            streak: [p[6], p[7]],
            tint: p[8],
            polish: p[9],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RampSpace {
    Position,
    Uv,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilmRamp {
    pub space: RampSpace,
    pub axis: [f32; 3],
    pub from: f32,
    pub to: f32,
    pub thickness: [f32; 2],
}

impl FilmRamp {
    pub const SAMPLE: Self = Self {
        space: RampSpace::Uv,
        axis: [1.0, 0.0, 0.0],
        from: 0.0,
        to: 1.0,
        thickness: [250.0, 750.0],
    };

    pub fn layer(self, amplitude: f32, seed: u32) -> NoiseLayer {
        layer(NoiseKind::FilmRamp, self.params(), amplitude, seed)
    }

    pub fn params(self) -> [f32; PARAMS] {
        fill(&[
            match self.space {
                RampSpace::Position => 1.0,
                RampSpace::Uv => 2.0,
            },
            self.axis[0],
            self.axis[1],
            self.axis[2],
            self.from,
            self.to,
            self.thickness[0],
            self.thickness[1],
        ])
    }

    pub fn from_params(p: &[f32; PARAMS]) -> Self {
        Self {
            space: if p[0] >= 1.5 {
                RampSpace::Uv
            } else {
                RampSpace::Position
            },
            axis: [p[1], p[2], p[3]],
            from: p[4],
            to: p[5],
            thickness: [p[6], p[7]],
        }
    }

    pub fn thickness_at(self, position: [f32; 3], uv: [f32; 2]) -> f32 {
        let along = match self.space {
            RampSpace::Position => {
                position[0] * self.axis[0] + position[1] * self.axis[1] + position[2] * self.axis[2]
            }
            RampSpace::Uv => uv[0] * self.axis[0] + uv[1] * self.axis[1],
        };
        let span = self.to - self.from;
        let t = if span.abs() <= 1e-12 {
            if along >= self.to { 1.0 } else { 0.0 }
        } else {
            ((along - self.from) / span).clamp(0.0, 1.0)
        };
        (self.thickness[0] + (self.thickness[1] - self.thickness[0]) * t).max(0.0)
    }
}

pub fn sample_params(kind: NoiseKind) -> Option<[f32; PARAMS]> {
    match kind {
        NoiseKind::Fibre => Some(Fibre::SAMPLE.params()),
        NoiseKind::Crinkle => Some(Crinkle::SAMPLE.params()),
        NoiseKind::PlankWood => Some(PlankWood::SAMPLE.params()),
        NoiseKind::WallMottle => Some(WallMottle::SAMPLE.params()),
        NoiseKind::Grime => Some(Grime::SAMPLE.params()),
        NoiseKind::Scratch => Some(Scratch::SAMPLE.params()),
        NoiseKind::CoatWobble => Some(CoatWobble::SAMPLE.params()),
        NoiseKind::Crack => Some(Crack::SAMPLE.params()),
        NoiseKind::Track => Some(Track::SAMPLE.params()),
        NoiseKind::FilmRamp => Some(FilmRamp::SAMPLE.params()),
        NoiseKind::None
        | NoiseKind::Leaf
        | NoiseKind::Bark
        | NoiseKind::Flow
        | NoiseKind::Value
        | NoiseKind::Fbm => None,
    }
}

fn layer(kind: NoiseKind, params: [f32; PARAMS], amplitude: f32, seed: u32) -> NoiseLayer {
    NoiseLayer::new(kind, 1.0, amplitude, seed).with_params(params)
}

fn fill(values: &[f32]) -> [f32; PARAMS] {
    let mut out = [0.0; PARAMS];
    out[..values.len()].copy_from_slice(values);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted(values: usize) -> [f32; PARAMS] {
        std::array::from_fn(|k| if k < values { k as f32 + 1.5 } else { 0.0 })
    }

    #[test]
    fn every_kind_reads_back_the_params_it_writes() {
        assert_eq!(Fibre::from_params(&counted(10)).params(), counted(10));
        assert_eq!(Crinkle::from_params(&counted(4)).params(), counted(4));
        assert_eq!(PlankWood::from_params(&counted(10)).params(), counted(10));
        assert_eq!(WallMottle::from_params(&counted(10)).params(), counted(10));
        assert_eq!(Grime::from_params(&counted(10)).params(), counted(10));
        assert_eq!(Scratch::from_params(&counted(4)).params(), counted(4));
        assert_eq!(CoatWobble::from_params(&counted(5)).params(), counted(5));
        assert_eq!(Crack::from_params(&counted(9)).params(), counted(9));
        assert_eq!(Track::from_params(&counted(10)).params(), counted(10));
    }

    #[test]
    fn a_kind_builds_its_own_layer() {
        let coat = CoatWobble {
            frequencies: [400.0, 30.0, 8.0],
            weights: [0.5, 0.25],
        };
        let layer = coat.layer(0.02, 4);
        assert_eq!(layer.kind, NoiseKind::CoatWobble);
        assert_eq!(layer.amplitude, 0.02);
        assert_eq!(layer.seed, 4);
        assert_eq!(CoatWobble::from_params(&layer.params), coat);
        assert!(!PlankWood::default().layer(1.0, 0).active());
        let wood = PlankWood {
            width: 0.2,
            ..PlankWood::default()
        };
        assert!(wood.layer(1.0, 0).active());
    }
}
