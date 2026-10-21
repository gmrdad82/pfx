use crate::camera::Camera;
use pfx_live::flat::look::{
    Aberration, Bloom, Crt, Curvature, Fade, Grade as Tone, Grain, Letterbox, Look, LookPass,
    Scanlines, SplitTone, Vignette,
};

pub const GLOW_RADIUS: f32 = 12.0;
pub const VIGNETTE: [f32; 2] = [430.0, 1260.0];
pub const SCANLINE_PERIOD: f32 = 3.0;
pub const GRAIN_RESPONSE: f32 = 0.5;
pub const CURVE_CORNER: f32 = 0.01;
const GAMMA: f32 = 1.0 / 2.2;
const GRAIN_SIZE: f32 = 1.0;
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Toning {
    pub amount: f32,
    pub shadow: [f32; 3],
    pub highlight: [f32; 3],
    pub range: [f32; 2],
}

impl Default for Toning {
    fn default() -> Toning {
        Toning {
            amount: 0.0,
            shadow: [1.0; 3],
            highlight: [1.0; 3],
            range: [0.0, 1.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    pub scanlines: f32,
    pub scanline_period: f32,
    pub curvature: f32,
    pub grain_response: f32,
    pub glow_threshold: f32,
    pub tone: Toning,
}

impl Default for Finish {
    fn default() -> Finish {
        Finish {
            scanlines: 0.0,
            scanline_period: SCANLINE_PERIOD,
            curvature: 0.0,
            grain_response: GRAIN_RESPONSE,
            glow_threshold: 0.0,
            tone: Toning::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    pub camera: Camera,
    pub game: bool,
    pub bars: f32,
    pub black: f32,
    pub flash: [f32; 4],
    pub desat: f32,
    pub tint: [f32; 3],
    pub contrast: f32,
    pub dim: f32,
    pub vignette: f32,
    pub grain: f32,
    pub glow: f32,
    pub split: f32,
    pub finish: Finish,
}

impl Default for Grade {
    fn default() -> Grade {
        Grade {
            camera: Camera::default(),
            game: true,
            bars: 0.0,
            black: 0.0,
            flash: [1.0, 1.0, 1.0, 0.0],
            desat: 0.0,
            tint: [1.0; 3],
            contrast: 0.0,
            dim: 0.0,
            vignette: 0.0,
            grain: 0.0,
            glow: 0.0,
            split: 0.0,
            finish: Finish::default(),
        }
    }
}

fn linear(shade: f32) -> f32 {
    1.0 - (1.0 - shade.clamp(0.0, 1.0)).powf(GAMMA)
}

fn encoded(tint: f32, amount: f32) -> f32 {
    let factor = (1.0 - amount + amount * tint.clamp(0.0, 1.0)).powf(GAMMA);
    (factor - (1.0 - amount)) / amount
}

fn short_side(wide: [f32; 2], out: [u32; 2]) -> f32 {
    let size = [out[0].max(1) as f32, out[1].max(1) as f32];
    let s = (size[0] / wide[0]).min(size[1] / wide[1]);
    (size[0].min(size[1]) / s / wide[1]).max(1.0)
}

impl Toning {
    pub fn split(&self) -> Option<SplitTone> {
        let amount = self.amount.clamp(0.0, 1.0);
        (amount > 0.0).then(|| {
            let tint = |c: [f32; 3]| {
                let [r, g, b] = c.map(|v| encoded(v, amount));
                [r, g, b, amount]
            };
            SplitTone::new(tint(self.shadow), tint(self.highlight))
                .range(self.range[0], self.range[1])
        })
    }
}

impl Grade {
    pub fn game_look(&self, out: [u32; 2]) -> Look {
        let wide = self.camera.wide;
        let k = short_side(wide, out);
        let finish = &self.finish;
        let mut look = Look::new();
        let desat = self.desat.clamp(0.0, 1.0);
        if desat > 0.0 {
            let [r, g, b] = self.tint.map(|c| encoded(c, desat));
            look = look.with(LookPass::Grade(Tone {
                saturation: 1.0 - desat,
                tint: [r, g, b, desat],
                ..Tone::new()
            }));
        }
        if self.contrast != 0.0 {
            look = look.with(LookPass::Grade(Tone {
                contrast: (1.0 + self.contrast).max(0.0),
                ..Tone::new()
            }));
        }
        if let Some(split) = finish.tone.split() {
            look = look.with(LookPass::Grade(Tone::new().split(split)));
        }
        if self.dim > 0.0 {
            look = look.with(LookPass::Grade(Tone {
                tint: [0.0, 0.0, 0.0, linear(self.dim)],
                ..Tone::new()
            }));
        }
        if self.vignette > 0.0 {
            look = look.with(LookPass::Vignette(Vignette {
                centre: [wide[0] * 0.5, wide[1] * 0.5],
                radius: [VIGNETTE[0] * k; 2],
                falloff: VIGNETTE[1] / VIGNETTE[0] - 1.0,
                floor: 0.0,
                colour: [0.0, 0.0, 0.0, linear(self.vignette)],
            }));
        }
        if self.grain > 0.0 {
            look = look.with(LookPass::Grain(
                Grain::new(self.grain / 3.0, GRAIN_SIZE * k).response(finish.grain_response),
            ));
        }
        let crt = Crt {
            scanlines: (finish.scanlines > 0.0).then(|| Scanlines {
                period: finish.scanline_period * k,
                strength: finish.scanlines,
            }),
            curvature: (finish.curvature > 0.0).then_some(Curvature {
                amount: finish.curvature,
                corner: CURVE_CORNER,
                bezel: BLACK,
            }),
            aberration: (self.split > 0.0).then(|| Aberration {
                shift: self.split * k,
            }),
            glow: None,
        };
        if !crt.is_empty() {
            look = look.with(LookPass::Crt(crt));
        }
        look
    }

    pub fn over_look(&self, out: [u32; 2]) -> Look {
        let mut look = Look::new();
        if self.glow > 0.0 {
            look = look.with(LookPass::Bloom(
                Bloom::new(GLOW_RADIUS, self.glow).threshold(self.finish.glow_threshold),
            ));
        }
        let [r, g, b, a] = self.flash;
        if a > 0.0 {
            look = look.with(LookPass::Fade(Fade::to([r, g, b, 1.0], a.clamp(0.0, 1.0))));
        }
        if self.black > 0.0 {
            look = look.with(LookPass::Fade(Fade::to(BLACK, self.black.clamp(0.0, 1.0))));
        }
        let aspect = out[0] as f32 / out[1].max(1) as f32;
        if self.bars > aspect {
            look = look.with(LookPass::Letterbox(Letterbox::aspect(self.bars, BLACK)));
        }
        look
    }
}
