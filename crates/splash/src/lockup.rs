use pfx_live::flat::svg::{self, Board};
use pfx_live::flat::{
    Draw, Fill, Icon, IconShape, Icons, Shadow, Shape, SpriteFill, SpriteFilter, SpriteImage,
    Srgba, multiply, scaling, translation,
};

use crate::art;
use crate::compose::is_light;

const MOST_RASTER: u32 = 4096;
const GUTTER: u32 = 2;

pub(crate) struct Painted {
    pub(crate) size: [u32; 2],
    pub(crate) rgba: Vec<u8>,
    pub(crate) draw: usize,
}

pub(crate) struct Baked {
    pub(crate) size: [f32; 2],
    pub(crate) draws: Vec<Draw>,
    pub(crate) shapes: Vec<IconShape>,
    pub(crate) painted: Vec<Painted>,
}

pub(crate) fn bake(
    source: &str,
    colour: Srgba,
    height: f32,
    at: [f32; 2],
    align: f32,
    density: f32,
) -> Result<Baked, String> {
    let board: Board = svg::read_mark(source, colour)?;
    let scale = height / board.size[1];
    let left = at[0] - board.size[0] * scale * align;
    let place = multiply(translation(left, at[1]), scaling(scale, scale));
    let mut painted = Vec::new();
    for raster in &board.rasters {
        let pixels = raster.pixels(scale * density, MOST_RASTER);
        let out = raster.paint(&board.icons, pixels)?;
        painted.push(Painted {
            size: out.size,
            rgba: out.rgba,
            draw: raster.draw,
        });
    }
    let draws = board
        .draws
        .into_iter()
        .map(|mut draw| {
            if let Some(g) = draw.group.take() {
                draw.opacity *= board.groups.get(usize::from(g)).map_or(1.0, |g| g.opacity);
            }
            draw.transform = multiply(place, draw.transform);
            draw.shadow = Shadow::None;
            draw.elevation = 0.0;
            draw
        })
        .collect();
    Ok(Baked {
        size: [board.size[0] * scale, height],
        draws,
        shapes: board.icons,
        painted,
    })
}

pub(crate) fn atlas(
    painted: &[Painted],
    draws: &mut [Draw],
) -> Result<Option<SpriteImage>, String> {
    if painted.is_empty() {
        return Ok(None);
    }
    let width = painted.iter().map(|p| p.size[0]).max().unwrap_or(1) + 2 * GUTTER;
    let height: u32 = painted.iter().map(|p| p.size[1] + 2 * GUTTER).sum();
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let mut top = 0;
    for p in painted {
        let [x0, y0] = [GUTTER, top + GUTTER];
        for row in 0..p.size[1] {
            let from = (row * p.size[0] * 4) as usize;
            let to = (((y0 + row) * width + x0) * 4) as usize;
            let len = (p.size[0] * 4) as usize;
            rgba[to..to + len].copy_from_slice(&p.rgba[from..from + len]);
        }
        let uv = [
            x0 as f32 / width as f32,
            y0 as f32 / height as f32,
            (x0 + p.size[0]) as f32 / width as f32,
            (y0 + p.size[1]) as f32 / height as f32,
        ];
        let draw = draws
            .get_mut(p.draw)
            .ok_or("a filtered shape lost its place")?;
        draw.fill = Fill::Image(SpriteFill::new(uv));
        top += p.size[1] + 2 * GUTTER;
    }
    SpriteImage::new(width, height, rgba, SpriteFilter::Linear).map(Some)
}

#[derive(Clone, Debug)]
pub struct Lockup {
    size: [f32; 2],
    draws: Vec<Draw>,
    shapes: Vec<IconShape>,
    sprites: Option<SpriteImage>,
}

impl Lockup {
    pub fn new(ground: Srgba, height: f32, density: f32) -> Result<Lockup, String> {
        if !(height.is_finite() && height > 0.0) {
            return Err("a lockup needs a height above zero".into());
        }
        if !(density.is_finite() && density > 0.0) {
            return Err("a lockup needs a pixel density above zero".into());
        }
        let source = art::powered_by(is_light(ground));
        let Baked {
            size,
            mut draws,
            shapes,
            painted,
        } = bake(source, Srgba::WHITE, height, [0.0, 0.0], 0.0, density)?;
        let sprites = atlas(&painted, &mut draws)?;
        Ok(Lockup {
            size,
            draws,
            shapes,
            sprites,
        })
    }

    pub fn size(&self) -> [f32; 2] {
        self.size
    }

    pub fn shapes(&self) -> &[IconShape] {
        &self.shapes
    }

    pub fn icons(&self) -> Result<Icons, String> {
        Icons::bake(&self.shapes)
    }

    pub fn sprites(&self) -> Option<&SpriteImage> {
        self.sprites.as_ref()
    }

    pub fn draws(&self, at: [f32; 2]) -> Vec<Draw> {
        self.draws_in(at, 0, [0.0, 0.0, 1.0, 1.0])
    }

    pub fn draws_in(&self, at: [f32; 2], icon_base: u32, frame: [f32; 4]) -> Vec<Draw> {
        let place = translation(at[0], at[1]);
        let [u0, v0, u1, v1] = frame;
        let (du, dv) = (u1 - u0, v1 - v0);
        self.draws
            .iter()
            .map(|draw| {
                let mut draw = draw.transform(multiply(place, draw.transform));
                if let Shape::Icon(Icon(i)) = draw.shape {
                    draw.shape = Shape::Icon(Icon(icon_base.saturating_add(i)));
                }
                if let Fill::Image(sprite) = &mut draw.fill {
                    let [a, b, c, d] = sprite.frame;
                    sprite.frame = [u0 + a * du, v0 + b * dv, u0 + c * du, v0 + d * dv];
                }
                draw
            })
            .collect()
    }
}
