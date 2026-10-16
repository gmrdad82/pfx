use super::footage::Picture;
use super::{Atlas, Fit, Shown};
use crate::camera::layout_in;
use crate::overlay::Overlay;
use crate::sheet::{Letter, Sheet};
use crate::show::FilmFrame;
use pfx_gpu::window::Rect;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::flat::look::{LookFrame, TokenSet};
use pfx_live::flat::{
    Draw, FlatIcons, FlatScene, FlatSprites, FlatText, Glyphs, Group, Icon, IconShape, Icons,
    Light, Shadow, ShadowCurve, Shape, SpriteFilter, SpriteImage, Srgba, multiply, scaling,
    translation,
};
use pfx_live::renderer::Renderer;
use pfx_live::text::{GlyphPages, RichQuad, page_quads};
use pfx_live::upscale::{self, Upscale};
use pfx_text::{Anchor, Face, Placement, Representation, Span, TextEngine};
use std::collections::HashMap;
use std::sync::OnceLock;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const EVERYWHERE: [f32; 4] = [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX];
const LINE: f32 = 1.2;
const HALF_WIDTH: f32 = 0.5;
const TICK: [[f32; 2]; 3] = [[-0.42, 0.02], [-0.12, 0.32], [0.44, -0.3]];
const TICK_REACH: f32 = 0.2;
const TICK_LOW: f32 = 2.0;
const TICK_PER_OCTAVE: f32 = 4.0;
const TICK_STEPS: u32 = 36;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Backdrop {
    Solid(Srgba),
    Fall(Srgba, Srgba),
}

impl Backdrop {
    fn clear(self) -> Srgba {
        match self {
            Backdrop::Solid(c) | Backdrop::Fall(c, _) => c,
        }
    }
}

pub struct Font {
    pub engine: TextEngine,
    pub family: String,
}

pub struct Painter {
    gpu: Gpu,
    renderer: Renderer,
    target: OffscreenTarget,
    upscale: Upscale,
    icons: FlatIcons,
    font: Option<Font>,
    pages: GlyphPages,
    sprite: Option<((u64, u64), [u32; 2], FlatSprites)>,
    images: Option<FlatSprites>,
    tokens: TokenSet,
    curve: ShadowCurve,
    size: [u32; 2],
    tick: u32,
    seed: u32,
}

struct Run {
    page: u32,
    quads: Vec<RichQuad>,
    colour: Srgba,
}

type Placed = Result<Vec<(u32, Vec<RichQuad>)>, String>;

struct Page<'a> {
    size: [f32; 2],
    draws: Vec<Draw>,
    groups: Vec<Group>,
    runs: Vec<Run>,
    font: Option<&'a mut Font>,
    scale: f32,
    tick: u32,
    placed: HashMap<(String, u32, u16), Placed>,
    widths: HashMap<(String, u32, u16), f32>,
    failed: Option<String>,
}

fn span(font: &Font, text: &str, size: f32, weight: u16) -> Span {
    Span {
        text: text.to_string(),
        face: Face {
            family: font.family.clone(),
            size,
            line: size * LINE,
            weight,
            italic: false,
            spacing: 0.0,
        },
        color: [1.0; 4],
    }
}

fn place(font: &mut Font, text: &str, size: f32, weight: u16, scale: f32) -> Placed {
    let spans = [span(font, text, size, weight)];
    let fail = |e: pfx_text::Error| format!("the letter {text:?}: {e:?}");
    let baseline = font
        .engine
        .layout_spans(&spans, None, scale, Anchor::Start)
        .map_err(fail)?
        .baseline;
    let at = Placement {
        scale,
        representation: Representation::Msdf,
        origin: [0.0, -baseline],
        alpha: 1.0,
        clip: EVERYWHERE,
        turn: [0.0; 3],
    };
    let placed = font
        .engine
        .place_spans(&spans, None, Anchor::Start, &at)
        .map_err(fail)?;
    Ok(page_quads(&placed.quads))
}

impl Sheet for Page<'_> {
    type Drawn = Shown;

    fn size(&self) -> [f32; 2] {
        self.size
    }

    fn draws(&mut self) -> &mut Vec<Draw> {
        &mut self.draws
    }

    fn group(&mut self, opacity: f32) -> Option<u16> {
        if opacity >= 0.999 {
            return None;
        }
        let id = u16::try_from(self.groups.len()).ok()?;
        self.groups.push(Group { opacity });
        Some(id)
    }

    fn letter(&mut self, letter: Letter) {
        let Some(font) = self.font.as_deref_mut() else {
            self.failed = Some("the film sets text but [film] names no font".into());
            return;
        };
        let key = (letter.text.clone(), letter.size.to_bits(), letter.weight);
        let scale = self.scale;
        let placed = self
            .placed
            .entry(key)
            .or_insert_with(|| place(font, &letter.text, letter.size, letter.weight, scale));
        let groups = match placed {
            Ok(groups) => groups,
            Err(e) => {
                self.failed = Some(e.clone());
                return;
            }
        };
        let frame = multiply(letter.frame, translation(letter.at[0], letter.at[1]));
        for (page, quads) in groups.iter() {
            self.runs.push(Run {
                page: *page,
                quads: quads.clone(),
                colour: letter.colour,
            });
            let mut draw = Draw::new(Shape::Text(self.runs.len() - 1))
                .transform(frame)
                .elevation(letter.elevation)
                .opacity(letter.opacity)
                .shadow(Shadow::None);
            draw.group = letter.group;
            self.draws.push(draw);
        }
    }

    fn width(&mut self, text: &str, size: f32, weight: u16) -> f32 {
        let Some(font) = self.font.as_deref_mut() else {
            return text.chars().count() as f32 * size * HALF_WIDTH;
        };
        let key = (text.to_string(), size.to_bits(), weight);
        if let Some(w) = self.widths.get(&key) {
            return *w;
        }
        let spans = [span(font, text, size, weight)];
        let w = font
            .engine
            .layout_spans(&spans, None, self.scale, Anchor::Start)
            .map_or(0.0, |block| block.width);
        self.widths.insert(key, w);
        w
    }

    fn check(&self, centre: [f32; 2], height: f32) -> Option<Draw> {
        let (step, baked) = tick_step(height)?;
        let s = height / baked;
        let at = multiply(translation(centre[0], centre[1]), scaling(s, s));
        Some(Draw::new(Shape::Icon(Icon(self.tick + step))).transform(at))
    }

    fn art(&mut self, art: &Shown) {
        self.draws
            .push(Draw::sprite(art.centre, art.size, art.uv).shadow(Shadow::None));
    }
}

fn tick_height(step: u32) -> f32 {
    TICK_LOW * 2f32.powf(step as f32 / TICK_PER_OCTAVE)
}

fn tick_step(height: f32) -> Option<(u32, f32)> {
    if !(height.is_finite() && height > 0.0) {
        return None;
    }
    let step = (TICK_PER_OCTAVE * (height / TICK_LOW).log2())
        .round()
        .clamp(0.0, (TICK_STEPS - 1) as f32) as u32;
    Some((step, tick_height(step)))
}

pub fn ticks() -> Vec<IconShape> {
    (0..TICK_STEPS)
        .map(|step| {
            let h = tick_height(step);
            IconShape {
                lines: vec![TICK.iter().map(|p| [p[0] * h, p[1] * h]).collect()],
                reach: TICK_REACH * h,
                ..IconShape::default()
            }
        })
        .collect()
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let mant = f32::from(bits & 0x3ff);
    match exp {
        0 => sign * mant * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        _ => sign * (1.0 + mant / 1024.0) * 2f32.powi(exp - 15),
    }
}

fn table() -> &'static [u8] {
    static TABLE: OnceLock<Vec<u8>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=u16::MAX)
            .map(|h| {
                let v = half(h);
                if v.is_nan() {
                    0
                } else {
                    (v.clamp(0.0, 1.0) * 255.0).round() as u8
                }
            })
            .collect()
    })
}

fn bytes(halves: &[u16]) -> Vec<u8> {
    let table = table();
    halves.iter().map(|&h| table[usize::from(h)]).collect()
}

pub fn cropped(picture: &Picture, crop: Option<[f32; 4]>) -> Result<([u32; 2], Vec<u8>), String> {
    let [w, h] = picture.size;
    let Some([x, y, cw, ch]) = crop else {
        return Ok((picture.size, picture.rgba.clone()));
    };
    let x0 = (x.round().max(0.0) as u32).min(w);
    let y0 = (y.round().max(0.0) as u32).min(h);
    let x1 = ((x + cw).round().max(0.0) as u32).min(w);
    let y1 = ((y + ch).round().max(0.0) as u32).min(h);
    if x1 <= x0 || y1 <= y0 {
        return Err(format!(
            "the crop [{x}, {y}, {cw}, {ch}] lies outside the {w}x{h} picture"
        ));
    }
    let row = (w * 4) as usize;
    let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for line in y0..y1 {
        let at = line as usize * row;
        out.extend_from_slice(&picture.rgba[at + x0 as usize * 4..at + x1 as usize * 4]);
    }
    Ok(([x1 - x0, y1 - y0], out))
}

pub fn placed(fit: Fit, source: [u32; 2], layout: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    let [w, h] = [source[0].max(1) as f32, source[1].max(1) as f32];
    let (sx, sy) = (layout[0] / w, layout[1] / h);
    let s = match fit {
        Fit::Fit => sx.min(sy),
        Fit::Fill => sx.max(sy),
    };
    ([layout[0] * 0.5, layout[1] * 0.5], [w * s, h * s])
}

impl Painter {
    pub fn new(
        gpu: Gpu,
        size: [u32; 2],
        icons: Icons,
        font: Option<Font>,
        tick: u32,
        seed: u32,
        atlas: Option<Atlas>,
    ) -> Result<Painter, String> {
        let renderer = Renderer::new(gpu.clone(), size[0], size[1]).map_err(|e| e.to_string())?;
        let target = gpu.offscreen(size[0], size[1], FORMAT)?;
        let upscale = Upscale::new(&gpu.device);
        let icons = FlatIcons::new(&gpu.device, &gpu.queue, icons);
        let images = atlas
            .map(|a| -> Result<FlatSprites, String> {
                let image = SpriteImage::new(a.size[0], a.size[1], a.rgba, SpriteFilter::Linear)?;
                Ok(FlatSprites::new(&gpu.device, &gpu.queue, image))
            })
            .transpose()?;
        Ok(Painter {
            gpu,
            renderer,
            target,
            upscale,
            icons,
            font,
            pages: GlyphPages::new(),
            sprite: None,
            images,
            tokens: TokenSet::new(),
            curve: ShadowCurve::none(),
            size,
            tick,
            seed,
        })
    }

    fn sprite(
        &mut self,
        picture: &Picture,
        key: (u64, u64),
        crop: Option<[f32; 4]>,
    ) -> Result<[u32; 2], String> {
        if let Some((k, size, _)) = &self.sprite
            && *k == key
        {
            return Ok(*size);
        }
        let (size, rgba) = cropped(picture, crop)?;
        let image = SpriteImage::new(size[0], size[1], rgba, SpriteFilter::Linear)?;
        self.sprite = Some((
            key,
            size,
            FlatSprites::new(&self.gpu.device, &self.gpu.queue, image),
        ));
        Ok(size)
    }

    fn picture(
        &mut self,
        frame: &FilmFrame<'_, Shown>,
        picture: Option<(&Picture, u64, Option<[f32; 4]>)>,
        fit: Fit,
        backdrop: Backdrop,
    ) -> Result<(), String> {
        let wide = frame.cut.grade.camera.wide;
        let mid = [wide[0] * 0.5, wide[1] * 0.5];
        let mut draws = Vec::with_capacity(2);
        let back = Draw::rect(mid, wide, 0.0).shadow(Shadow::None);
        draws.push(match backdrop {
            Backdrop::Solid(c) => back.fill(c),
            Backdrop::Fall(top, bottom) => back.vertical(top, bottom),
        });
        if let Some((picture, serial, crop)) = picture {
            let shown = self.sprite(picture, (serial, picture.stamp), crop)?;
            let (centre, size) = placed(fit, shown, wide);
            draws.push(Draw::sprite(centre, size, [0.0, 0.0, 1.0, 1.0]).shadow(Shadow::None));
        }
        let sprites = picture.and(self.sprite.as_ref().map(|(_, _, s)| s));
        let scene = FlatScene {
            layout: wide,
            clear: Some(backdrop.clear()),
            curve: &self.curve,
            light: Light::default(),
            draws: &draws,
            groups: &[],
            text: &[],
            icons: None,
            sprites,
            environment: None,
            post: false,
            frame: frame.index as u32,
            seed: self.seed,
        };
        let look = frame.game_look(self.size);
        self.renderer.set_flat_view(frame.view(self.size));
        self.renderer.render_flat_look(
            &scene,
            &LookFrame::still(&look, &self.tokens),
            &self.target.view,
        )?;
        Ok(())
    }

    fn overlay(
        &mut self,
        frame: &FilmFrame<'_, Shown>,
        overlay: &mut Overlay,
    ) -> Result<(), String> {
        let size = layout_in(frame.cut.grade.camera.wide, self.size);
        let scale = pfx_live::flat::Fit::new(size, self.size)?.scale;
        if let Some(font) = &mut self.font {
            font.engine.begin_frame();
        }
        let mut page = Page {
            size,
            draws: Vec::new(),
            groups: Vec::new(),
            runs: Vec::new(),
            font: self.font.as_mut(),
            scale,
            tick: self.tick,
            placed: HashMap::new(),
            widths: HashMap::new(),
            failed: None,
        };
        overlay.paint(&mut page, &frame.cut.pieces, &frame.cut.grade, true);
        let Page {
            draws,
            groups,
            runs,
            failed,
            ..
        } = page;
        if let Some(e) = failed {
            return Err(e);
        }
        if let Some(font) = &mut self.font {
            font.engine.end_frame();
            let changes = font.engine.take_atlas_changes();
            self.pages
                .sync(&self.gpu.device, &self.gpu.queue, &changes)
                .map_err(|e| format!("the film's glyph pages: {e:?}"))?;
        }
        let mut texts = Vec::with_capacity(runs.len());
        for run in &runs {
            let atlas = self
                .pages
                .page(run.page)
                .ok_or_else(|| format!("glyph page {} is missing", run.page))?;
            texts.push(FlatText {
                atlas,
                glyphs: Glyphs::Quads(&run.quads),
                colour: run.colour,
            });
        }
        let scene = FlatScene {
            layout: size,
            clear: None,
            curve: &self.curve,
            light: Light::default(),
            draws: &draws,
            groups: &groups,
            text: &texts,
            icons: Some(&self.icons),
            sprites: self.images.as_ref(),
            environment: None,
            post: false,
            frame: frame.index as u32,
            seed: self.seed,
        };
        let look = frame.over_look(self.size);
        let still = LookFrame::still(&look, &self.tokens);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx film overlay"),
            });
        self.upscale.set_view(None);
        self.upscale.encode_overlay(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &upscale::Overlay {
                scene: &scene,
                look: Some(&still),
                output: &self.target.view,
                output_format: FORMAT,
                output_size: self.size,
                content: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: self.size[0] as f32,
                    height: self.size[1] as f32,
                },
            },
            None,
        )?;
        self.gpu.queue.submit([encoder.finish()]);
        Ok(())
    }

    pub fn paint(
        &mut self,
        frame: &FilmFrame<'_, Shown>,
        overlay: &mut Overlay,
        picture: Option<(&Picture, u64, Option<[f32; 4]>)>,
        fit: Fit,
        backdrop: Backdrop,
    ) -> Result<Vec<u8>, String> {
        self.picture(frame, picture, fit, backdrop)?;
        self.overlay(frame, overlay)?;
        Ok(bytes(&self.gpu.readback_rgba16(&self.target)?))
    }
}
