use pfx_live::flat::svg;
use pfx_live::flat::{
    Draw, Icon, IconShape, Icons, Matrix, Shadow, Shape, SpriteImage, Srgba, multiply, scaling,
    translation,
};

use crate::art;
use crate::lockup::{Painted, atlas, bake};
use crate::timeline::LENGTH;

const SITE: Srgba = Srgba::hex(0xb6abcb);
const SITE_ON_LIGHT: Srgba = Srgba::hex(0x5331a8);
const DOLLY: f32 = 0.05;
pub(crate) const RULE_FROM: f64 = 0.45;
pub(crate) const RULE_TO: f64 = 0.85;
pub(crate) const SITE_IN: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Motion {
    Still,
    Rule { centre: f32 },
    AfterRule,
}

#[derive(Clone, Debug)]
struct Part {
    draws: std::ops::Range<usize>,
    motion: Motion,
}

#[derive(Clone, Debug)]
pub struct Composition {
    size: [u32; 2],
    background: Srgba,
    draws: Vec<Draw>,
    parts: Vec<Part>,
    icons: Icons,
    sprites: Option<SpriteImage>,
}

pub struct Frame {
    pub clear: Srgba,
    pub draws: Vec<Draw>,
}

pub(crate) struct Piece {
    source: String,
    colour: Srgba,
    pub(crate) height: f32,
    pub(crate) at: [f32; 2],
    motion: Motion,
}

fn rule(width: f32, height: f32) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\"><defs><linearGradient id=\"r\" x1=\"0\" y1=\"0\" x2=\"{width}\" y2=\"0\" gradientUnits=\"userSpaceOnUse\"><stop offset=\"0\" stop-color=\"#542ACD\" stop-opacity=\"0\"/><stop offset=\"0.3\" stop-color=\"#542ACD\" stop-opacity=\"1\"/><stop offset=\"0.5\" stop-color=\"#B6ABCB\" stop-opacity=\"1\"/><stop offset=\"0.7\" stop-color=\"#542ACD\" stop-opacity=\"1\"/><stop offset=\"1\" stop-color=\"#542ACD\" stop-opacity=\"0\"/></linearGradient></defs><rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" fill=\"url(#r)\"/></svg>"
    )
}

fn aspect(source: &str) -> Result<f32, String> {
    let board = svg::read_mark(source, Srgba::WHITE)?;
    Ok(board.size[0] / board.size[1])
}

pub(crate) fn pieces(size: [u32; 2], light: bool) -> Result<Vec<Piece>, String> {
    let [w, h] = size.map(|v| v as f32);
    let u = w.min(h);
    let centre = w * 0.5;
    let piece = |source: &str, colour: Srgba, height: f32, top: f32| -> Result<Piece, String> {
        Ok(Piece {
            source: source.to_string(),
            colour,
            height,
            at: [centre, top],
            motion: Motion::Still,
        })
    };
    let stack = |heights: &[(f32, f32)]| -> Vec<f32> {
        let total: f32 = heights.iter().map(|(height, gap)| height + gap).sum();
        let mut top = (h - total) * 0.5;
        heights
            .iter()
            .map(|(height, gap)| {
                let at = top;
                top += height + gap;
                at
            })
            .collect()
    };
    let artwork = art::powered_by(light);
    let ratio = aspect(artwork)?;
    let credit = (0.12 * u).min(0.66 * w / ratio);
    let (line, site) = ((0.0022 * u).max(2.0), 0.019 * u);
    let tops = stack(&[(credit, 0.055 * u), (line, 0.05 * u), (site, 0.0)]);
    let rule_width = ratio * credit * 1.15;
    let site_colour = if light { SITE_ON_LIGHT } else { SITE };
    Ok(vec![
        piece(artwork, Srgba::WHITE, credit, tops[0])?,
        Piece {
            source: rule(rule_width, line),
            colour: Srgba::WHITE,
            height: line,
            at: [centre, tops[1]],
            motion: Motion::Rule { centre },
        },
        Piece {
            motion: Motion::AfterRule,
            ..piece(art::SITE, site_colour, site, tops[2])?
        },
    ])
}

pub(crate) fn ease_out(x: f64) -> f32 {
    let x = x.clamp(0.0, 1.0);
    (1.0 - (1.0 - x).powi(3)) as f32
}

fn about(centre: [f32; 2], scale: [f32; 2]) -> Matrix {
    multiply(
        translation(centre[0], centre[1]),
        multiply(
            scaling(scale[0], scale[1]),
            translation(-centre[0], -centre[1]),
        ),
    )
}

impl Composition {
    pub fn new(size: [u32; 2], background: Srgba) -> Result<Composition, String> {
        if size[0] == 0 || size[1] == 0 {
            return Err("a splash needs a window with pixels".into());
        }
        let mut shapes: Vec<IconShape> = Vec::new();
        let mut draws = Vec::new();
        let mut parts = Vec::new();
        let mut painted: Vec<Painted> = Vec::new();
        for piece in pieces(size, is_light(background))? {
            let baked = bake(
                &piece.source,
                piece.colour,
                piece.height,
                piece.at,
                0.5,
                1.0,
            )?;
            let base = shapes.len() as u32;
            let start = draws.len();
            painted.extend(baked.painted.into_iter().map(|p| Painted {
                draw: start + p.draw,
                ..p
            }));
            for mut draw in baked.draws {
                if let Shape::Icon(Icon(i)) = draw.shape {
                    draw.shape = Shape::Icon(Icon(base + i));
                }
                draws.push(draw);
            }
            shapes.extend(baked.shapes);
            parts.push(Part {
                draws: start..draws.len(),
                motion: piece.motion,
            });
        }
        let sprites = atlas(&painted, &mut draws)?;
        let icons = Icons::bake(&shapes)?;
        Ok(Composition {
            size,
            background,
            draws,
            parts,
            icons,
            sprites,
        })
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn background(&self) -> Srgba {
        self.background
    }

    pub fn icons(&self) -> &Icons {
        &self.icons
    }

    pub fn sprites(&self) -> Option<&SpriteImage> {
        self.sprites.as_ref()
    }

    pub fn frame(&self, at: f64, opacity: f32) -> Frame {
        let [w, h] = self.size.map(|v| v as f32);
        let centre = [w * 0.5, h * 0.5];
        let dolly = about(centre, [1.0 - DOLLY * (1.0 - ease_out(at / LENGTH)); 2]);
        let mut draws = Vec::with_capacity(self.draws.len() + 1);
        for part in &self.parts {
            let (own, shown) = match part.motion {
                Motion::Still => (dolly, 1.0),
                Motion::Rule { centre: x } => {
                    let grow = ease_out((at - RULE_FROM) / (RULE_TO - RULE_FROM));
                    let y = self.draws[part.draws.start].transform[3][1];
                    let own = multiply(dolly, about([x, y], [grow.max(1e-3), 1.0]));
                    (own, if at > RULE_FROM { 1.0 } else { 0.0 })
                }
                Motion::AfterRule => (dolly, ease_out((at - RULE_TO) / SITE_IN)),
            };
            if shown <= 0.0 {
                continue;
            }
            for draw in &self.draws[part.draws.clone()] {
                let mut draw = draw.transform(multiply(own, draw.transform));
                draw.opacity *= shown;
                draws.push(draw);
            }
        }
        let veil = 1.0 - opacity.clamp(0.0, 1.0);
        if veil > 0.0 {
            let mut cover =
                Draw::rect(centre, [w + 2.0, h + 2.0], 0.0).fill(self.background.alpha(veil));
            cover.shadow = Shadow::None;
            draws.push(cover);
        }
        Frame {
            clear: self.background,
            draws,
        }
    }
}

pub(crate) fn is_light(colour: Srgba) -> bool {
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b, _] = colour.0;
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b) > 0.179
}
