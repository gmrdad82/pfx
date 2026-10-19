use super::footage::{EPSILON, Picture, Source};
use super::{Atlas, DropShadow, Fit, Playing, Shown};
use crate::camera::layout_in;
use crate::overlay::Overlay;
use crate::sheet::{Letter, Sheet};
use crate::show::FilmFrame;
use pfx_gpu::window::Rect;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::flat::look::{LookFrame, TokenSet};
use pfx_live::flat::{
    Draw, FlatIcons, FlatScene, FlatSprites, FlatText, Glow, Glyphs, Group, Icon, IconShape, Icons,
    Light, Shadow, ShadowCurve, Shape, SpriteFilter, SpriteImage, Srgba, multiply, scaling,
    translation,
};
use pfx_live::renderer::Renderer;
use pfx_live::text::{GlyphPages, RichQuad, page_quads};
use pfx_live::upscale::{self, Upscale};
use pfx_post::quantise::{self, Dither};
use pfx_text::{Anchor, Face, Placement, Representation, Span, TextEngine};
use std::collections::HashMap;
use std::path::PathBuf;
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
const CLIP: u32 = 0x4000_0000;
const PAGE: u32 = 0x2000_0000;
const SHARP: f32 = 0.5;
const SILENT: usize = usize::MAX;

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

struct Reader {
    source: Box<dyn Source>,
    last: f64,
    sprite: Option<(u64, FlatSprites)>,
}

pub struct Painter {
    readers: HashMap<(PathBuf, u64), Reader>,
    gpu: Gpu,
    renderer: Renderer,
    target: OffscreenTarget,
    upscale: Upscale,
    icons: FlatIcons,
    font: Option<Font>,
    pages: GlyphPages,
    sprite: Option<((u64, u64), [u32; 2], FlatSprites)>,
    images: Vec<FlatSprites>,
    tokens: TokenSet,
    curve: ShadowCurve,
    size: [u32; 2],
    tick: u32,
    seed: u32,
    dither: Option<quantise::Tile>,
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
    clips: Vec<Playing>,
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
        let id = match &art.clip {
            Some(playing) => {
                self.clips.push(playing.clone());
                CLIP | (self.clips.len() as u32 - 1)
            }
            None if art.page > 0 => PAGE | art.page as u32,
            None => 0,
        };
        let half = [art.size[0] * 0.5, art.size[1] * 0.5];
        let radius = art.radius.min(half[0]).min(half[1]).max(0.0);
        if let Some(d) = art.shadow {
            self.draws.push(shadow(art, d, radius).id(id));
        }
        let mut draw = Draw::sprite(art.centre, art.size, art.uv)
            .shadow(Shadow::None)
            .id(id);
        draw.shape = Shape::rounded(half, radius);
        self.draws.push(draw);
    }
}

fn shadow(art: &Shown, d: DropShadow, radius: f32) -> Draw {
    let centre = [art.centre[0] + d.offset[0], art.centre[1] + d.offset[1]];
    let colour = Srgba([0.0, 0.0, 0.0, d.alpha]);
    let draw = Draw::rect(centre, art.size, radius);
    if d.blur < SHARP {
        return draw.fill(colour).shadow(Shadow::None);
    }
    draw.fill(Srgba([0.0; 4])).shadow(Shadow::Glow(Glow {
        colour,
        sigma: d.blur,
    }))
}

fn layer<'a>(
    frame: &FilmFrame<'_, Shown>,
    size: [f32; 2],
    draws: &'a [Draw],
    sprites: Option<&'a FlatSprites>,
    curve: &'a ShadowCurve,
    seed: u32,
) -> FlatScene<'a> {
    FlatScene {
        layout: size,
        clear: None,
        curve,
        light: Light::default(),
        draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites,
        environment: None,
        post: false,
        frame: frame.index as u32,
        seed,
    }
}

fn pass(
    gpu: &Gpu,
    upscale: &mut Upscale,
    target: &OffscreenTarget,
    out: [u32; 2],
    scene: FlatScene<'_>,
) -> Result<(), String> {
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pfx film layer"),
        });
    upscale.set_view(None);
    upscale.encode_overlay(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &upscale::Overlay {
            scene: &scene,
            look: None,
            output: &target.view,
            output_format: FORMAT,
            output_size: out,
            content: Rect {
                x: 0.0,
                y: 0.0,
                width: out[0] as f32,
                height: out[1] as f32,
            },
        },
        None,
    )?;
    gpu.queue.submit([encoder.finish()]);
    Ok(())
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

fn table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| (0..=u16::MAX).map(quantise::half).collect())
}

fn bytes(halves: &[u16], width: u32, dither: Option<&quantise::Tile>) -> Vec<u8> {
    let table = table();
    let width = width.max(1);
    let mut out = Vec::with_capacity(halves.len());
    for (at, pixel) in halves.chunks_exact(4).enumerate() {
        let [r, g, b, a] = [0, 1, 2, 3].map(|k| table[usize::from(pixel[k])]);
        match dither {
            Some(tile) => {
                let at = at as u32;
                let offset = tile.at(at % width, at / width);
                out.extend([r, g, b].map(|v| quantise::dithered(v, offset)));
            }
            None => out.extend([r, g, b].map(quantise::code)),
        }
        out.push(quantise::code(a));
    }
    out
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
            .map(|a| a.pages)
            .unwrap_or_default()
            .into_iter()
            .map(|page| -> Result<FlatSprites, String> {
                let image =
                    SpriteImage::new(page.size[0], page.size[1], page.rgba, SpriteFilter::Linear)?;
                Ok(FlatSprites::new(&gpu.device, &gpu.queue, image))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Painter {
            readers: HashMap::new(),
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
            dither: Some(Dither::seeded(u64::from(seed)).tile()),
        })
    }

    pub fn set_dither(&mut self, dither: Option<Dither>) {
        self.dither = dither.map(|d| d.tile());
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
            clips: Vec::new(),
        };
        overlay.paint(&mut page, &frame.cut.pieces, &frame.cut.grade, true);
        let Page {
            draws,
            groups,
            runs,
            failed,
            clips,
            ..
        } = page;
        if let Some(e) = failed {
            return Err(e);
        }
        let (apart, draws): (Vec<Draw>, Vec<Draw>) =
            draws.into_iter().partition(|d| d.id & (CLIP | PAGE) != 0);
        self.pages(frame, size, &apart)?;
        self.clips(frame, size, &clips, &apart)?;
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
            sprites: self.images.first(),
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

    fn read(&mut self, playing: &Playing) -> Result<Option<&FlatSprites>, String> {
        let key = (playing.path.clone(), playing.from.to_bits());
        let stale = self
            .readers
            .get(&key)
            .is_some_and(|r| playing.at + EPSILON < r.last);
        if stale || !self.readers.contains_key(&key) {
            let source = super::clip(&playing.path, playing.from, SILENT)?;
            self.readers.insert(
                key.clone(),
                Reader {
                    source,
                    last: playing.at,
                    sprite: None,
                },
            );
        }
        let Some(reader) = self.readers.get_mut(&key) else {
            return Ok(None);
        };
        reader.last = playing.at;
        let Some(picture) = reader.source.picture(playing.at)? else {
            return Ok(None);
        };
        if reader
            .sprite
            .as_ref()
            .is_none_or(|(stamp, _)| *stamp != picture.stamp)
        {
            let image = SpriteImage::new(
                picture.size[0],
                picture.size[1],
                picture.rgba.clone(),
                SpriteFilter::Linear,
            )?;
            let sprites = FlatSprites::new(&self.gpu.device, &self.gpu.queue, image);
            reader.sprite = Some((picture.stamp, sprites));
        }
        Ok(reader.sprite.as_ref().map(|(_, s)| s))
    }

    fn clips(
        &mut self,
        frame: &FilmFrame<'_, Shown>,
        size: [f32; 2],
        clips: &[Playing],
        played: &[Draw],
    ) -> Result<(), String> {
        let keep: Vec<(PathBuf, u64)> = clips
            .iter()
            .map(|p| (p.path.clone(), p.from.to_bits()))
            .collect();
        self.readers.retain(|key, _| keep.contains(key));
        for (i, playing) in clips.iter().enumerate() {
            let id = CLIP | i as u32;
            let draws: Vec<Draw> = played.iter().filter(|d| d.id == id).copied().collect();
            if self.read(playing)?.is_none() {
                continue;
            }
            let key = (playing.path.clone(), playing.from.to_bits());
            let sprites = self
                .readers
                .get(&key)
                .and_then(|r| r.sprite.as_ref())
                .map(|(_, s)| s);
            pass(
                &self.gpu,
                &mut self.upscale,
                &self.target,
                self.size,
                layer(frame, size, &draws, sprites, &self.curve, self.seed),
            )?;
        }
        Ok(())
    }

    fn pages(
        &mut self,
        frame: &FilmFrame<'_, Shown>,
        size: [f32; 2],
        apart: &[Draw],
    ) -> Result<(), String> {
        for (p, sprites) in self.images.iter().enumerate().skip(1) {
            let id = PAGE | p as u32;
            let draws: Vec<Draw> = apart.iter().filter(|d| d.id == id).copied().collect();
            if draws.is_empty() {
                continue;
            }
            pass(
                &self.gpu,
                &mut self.upscale,
                &self.target,
                self.size,
                layer(frame, size, &draws, Some(sprites), &self.curve, self.seed),
            )?;
        }
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
        let halves = self.gpu.readback_rgba16(&self.target)?;
        Ok(bytes(&halves, self.size[0], self.dither.as_ref()))
    }
}
