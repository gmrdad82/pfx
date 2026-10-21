use std::collections::BTreeMap;
use std::rc::Rc;

use pfx_gpu::screens::{Budget, ScreenReport};
use pfx_gpu::window::Size;
use pfx_gpu::{Gpu, wgpu};
use pfx_input::{GlyphAtlas, KeyLabels};
use pfx_live::demand::{Next, SETTLE};
use pfx_live::flat::look::Look;
use pfx_live::flat::{
    Draw, Environment, FlatIcons, FlatScene, FlatSprites, FlatText, Glyphs, IDENTITY, Matrix,
    ShadowCurve, Shape, Srgba, multiply, translation,
};
use pfx_live::renderer::Renderer;
use pfx_live::scene::Staged;
use pfx_live::text::{GlyphPages, GpuAtlas, RichQuad, page_quads};
use pfx_live::upscale::{Compose, Filter, Overlay, Upscale, clip};
use pfx_load::scene::Scene;
use pfx_play::{Anchor, PaintFrame, Ui, Warmer, Warming};
use pfx_text::{Face, Fallback, Placement, Prefill, Representation, Span, TextEngine};

use crate::config::{Config, Redraw};
use crate::driver::Driver;
use crate::prompt::{self, Part, PromptQuads, Sources, Style};

pub struct Painter {
    renderer: Renderer,
    staged: Option<Staged>,
    upscale: Upscale,
    text: Option<TextEngine>,
    pages: GlyphPages,
    filter: Filter,
    render_scale: f32,
    seed: u32,
    none: ShadowCurve,
    glyphs: Option<(GlyphAtlas, GpuAtlas)>,
    icons: Option<FlatIcons>,
    sprites: Option<FlatSprites>,
    environment: Option<Environment>,
    fallbacks: BTreeMap<String, Vec<String>>,
    prompts: Vec<PromptQuads>,
    draws: Vec<Draw>,
    span: Span,
    uploads: usize,
    redraw: Redraw,
    source: Option<(wgpu::TextureView, [u32; 2])>,
    shown: Option<Shown>,
    budget: Option<Budget>,
    graphics: Option<pfx_settings::Graphics>,
}

struct Shown {
    ui: Rc<Ui>,
    parts: Vec<Vec<Part>>,
    labels: Option<KeyLabels>,
    report: ScreenReport,
    window: Size,
    hud: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drawn {
    pub render: [u32; 2],
    pub output: [u32; 2],
    pub labels: usize,
    pub prompts: usize,
    pub uploads: usize,
}

const BLACK: Srgba = Srgba([0.0, 0.0, 0.0, 1.0]);
pub const FRAMES_IN_FLIGHT: u64 = 2;

enum Sheet {
    Page(u32),
    Glyphs,
}

enum Quads {
    Own(Vec<RichQuad>),
    Glyphs(usize),
    Label(usize, usize),
}

struct Run {
    sheet: Sheet,
    quads: Quads,
    colour: Srgba,
    elevation: f32,
    id: u32,
    transform: Matrix,
}

fn share(anchor: Anchor) -> f32 {
    match anchor {
        Anchor::Start => 0.0,
        Anchor::Center => 0.5,
        Anchor::End => 1.0,
    }
}

fn anchor(anchor: Anchor) -> pfx_text::Anchor {
    match anchor {
        Anchor::Start => pfx_text::Anchor::Start,
        Anchor::Center => pfx_text::Anchor::Center,
        Anchor::End => pfx_text::Anchor::End,
    }
}

impl Painter {
    pub fn new(
        gpu: Gpu,
        scene: Option<&Scene>,
        config: &Config,
        window: Size,
    ) -> Result<Self, String> {
        Self::open(gpu, scene, config, window, false)
    }

    pub fn live(
        gpu: Gpu,
        scene: Option<&Scene>,
        config: &Config,
        window: Size,
    ) -> Result<Self, String> {
        Self::open(gpu, scene, config, window, true)
    }

    fn open(
        gpu: Gpu,
        scene: Option<&Scene>,
        config: &Config,
        window: Size,
        background: bool,
    ) -> Result<Self, String> {
        let width = window.width.max(1);
        let height = window.height.max(1);
        let mut renderer = Renderer::new(gpu, width, height).map_err(|error| error.to_string())?;
        renderer.set_background_reflections(background);
        renderer.set_exposure(config.exposure)?;
        if config.redraw == Redraw::OnDemand {
            renderer.watch_redraws(Some(SETTLE));
        }
        let mut staged = scene.map(|scene| renderer.stage(scene)).transpose()?;
        if background && let Some(staged) = &mut staged {
            let frame = staged.frame(width as f32 / height as f32, 0.0, config.seed as u32);
            renderer.warm_frame(&frame.scene, &frame.effects)?;
        }
        if let (Some(budget), Some(_)) = (config.budget, &staged) {
            renderer.set_dynamic_resolution(Some(budget))?;
        }
        let upscale = Upscale::new(&renderer.gpu().device);
        let text = match config.fonts.split_first() {
            Some((first, rest)) => {
                let mut engine = TextEngine::new(first)
                    .map_err(|error| format!("the game's first font: {error:?}"))?;
                for font in rest {
                    engine
                        .register_font(font)
                        .map_err(|error| format!("the game's font: {error:?}"))?;
                }
                Some(engine)
            }
            None => None,
        };
        let gpu = renderer.gpu().clone();
        let icons = config
            .icons
            .clone()
            .map(|icons| FlatIcons::new(&gpu.device, &gpu.queue, icons));
        let sprites = config
            .sprites
            .clone()
            .map(|sprites| FlatSprites::new(&gpu.device, &gpu.queue, sprites));
        let environment = config
            .environment
            .as_ref()
            .map(|texels| {
                Environment::new(
                    &gpu.device,
                    &gpu.queue,
                    texels.width,
                    texels.height,
                    &texels.linear,
                    texels.intensity,
                )
            })
            .transpose()?;
        renderer.set_frames_in_flight(FRAMES_IN_FLIGHT);
        Ok(Self {
            renderer,
            staged,
            upscale,
            text,
            pages: GlyphPages::new(),
            filter: config.filter,
            render_scale: config.render_scale,
            seed: config.seed as u32,
            none: ShadowCurve::none(),
            glyphs: None,
            icons,
            sprites,
            environment,
            fallbacks: BTreeMap::new(),
            prompts: Vec::new(),
            draws: Vec::new(),
            span: Span {
                text: String::new(),
                face: Face {
                    family: String::new(),
                    size: 0.0,
                    line: 0.0,
                    weight: 400,
                    italic: false,
                    spacing: 0.0,
                },
                color: [1.0; 4],
            },
            uploads: 0,
            redraw: config.redraw,
            source: None,
            shown: None,
            budget: config.budget,
            graphics: None,
        })
    }

    pub fn follow_graphics(&mut self, graphics: &pfx_settings::Graphics) -> Result<(), String> {
        if self.graphics.as_ref() == Some(graphics) {
            return Ok(());
        }
        let budget = resolution(graphics.quality, self.budget);
        if self.staged.is_some()
            && budget.map(|budget| budget.gpu_ms)
                != self
                    .renderer
                    .dynamic_resolution()
                    .map(|resolution| resolution.budget().gpu_ms)
        {
            self.renderer.set_dynamic_resolution(budget)?;
        }
        self.renderer
            .set_shedding(graphics.quality == Some(pfx_settings::Quality::Performance))?;
        self.renderer.set_effects(graphics.effects)?;
        self.graphics = Some(*graphics);
        Ok(())
    }

    pub fn graphics(&self) -> Option<&pfx_settings::Graphics> {
        self.graphics.as_ref()
    }

    pub fn wait_for_frames(&mut self, waits: bool) {
        self.renderer
            .set_frames_in_flight(if waits { 0 } else { FRAMES_IN_FLIGHT });
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    pub fn staged(&self) -> Option<&Staged> {
        self.staged.as_ref()
    }

    pub fn text(&self) -> Option<&TextEngine> {
        self.text.as_ref()
    }

    pub fn glyph_pages(&self) -> &GlyphPages {
        &self.pages
    }

    pub fn prompt_quads(&self) -> &[PromptQuads] {
        &self.prompts
    }

    pub fn upscale(&self) -> &Upscale {
        &self.upscale
    }

    pub fn warm(
        &mut self,
        driver: &mut Driver,
        format: wgpu::TextureFormat,
        pass: u32,
        elapsed: f64,
        budget: f64,
    ) -> Result<Warming, String> {
        let window = driver.window();
        let size = [window.width.max(1), window.height.max(1)];
        let pixels_per_unit = driver
            .report()
            .map_or(1.0, |report| report.pixels_per_unit());
        if pass == 0 {
            self.upscale.warm(&self.renderer.gpu().device, format);
        }
        let mut warmer = PainterWarmer {
            painter: self,
            format,
            size,
            pixels_per_unit,
        };
        driver.warm(&mut warmer, pass, elapsed, budget)
    }

    fn sync_pages(&mut self) -> Result<usize, String> {
        let Some(engine) = &mut self.text else {
            return Ok(0);
        };
        let changes = engine.take_atlas_changes();
        let gpu = self.renderer.gpu();
        self.pages
            .sync(&gpu.device, &gpu.queue, &changes)
            .map_err(|error| format!("the UI's glyph pages: {error:?}"))?;
        Ok(changes.cells.len())
    }

    fn fallbacks(&mut self, family: &str, faces: &[String]) -> Result<(), String> {
        if faces.is_empty() || self.fallbacks.get(family).is_some_and(|set| set == faces) {
            return Ok(());
        }
        let Some(engine) = &mut self.text else {
            return Ok(());
        };
        let list: Vec<Fallback> = faces.iter().map(|face| Fallback::new(face)).collect();
        engine
            .set_fallbacks(family, None, &list)
            .map_err(|error| format!("the fallbacks of {family:?}: {error:?}"))?;
        self.fallbacks.insert(family.to_owned(), faces.to_vec());
        Ok(())
    }

    fn render_size(&mut self, content: [u32; 2], scale: Option<Size>) -> Result<[u32; 2], String> {
        if self.renderer.dynamic_resolution().is_some() {
            let (width, height) = self.renderer.apply_dynamic_resolution(content)?;
            return Ok([width, height]);
        }
        let size = scale.ok_or("the render scale must be a positive finite factor")?;
        if self.renderer.size() != (size.width, size.height) {
            self.renderer.resize(size.width, size.height)?;
        }
        Ok([size.width, size.height])
    }

    fn ensure_glyph_atlas(&mut self) -> Result<(), String> {
        if self.glyphs.is_none() {
            let atlas =
                GlyphAtlas::build().map_err(|error| format!("the input glyph atlas: {error:?}"))?;
            let gpu = self.renderer.gpu();
            let upload = GpuAtlas::new(&gpu.device, &gpu.queue, &atlas.atlas)
                .map_err(|error| format!("the input glyph atlas: {error:?}"))?;
            self.glyphs = Some((atlas, upload));
        }
        Ok(())
    }

    fn runs(
        &mut self,
        ui: &Ui,
        parts: &[Vec<Part>],
        labels: &KeyLabels,
        pixels_per_unit: f32,
    ) -> Result<Vec<Run>, String> {
        self.prompts.clear();
        self.uploads = 0;
        if ui.labels.is_empty() && ui.prompts.is_empty() {
            self.uploads = self.sync_pages()?;
            return Ok(Vec::new());
        }
        if !ui.prompts.is_empty() {
            self.ensure_glyph_atlas()?;
        }
        let placement = prompt::placement(pixels_per_unit);
        let mut runs = Vec::new();
        if let Some(engine) = &mut self.text {
            engine.begin_frame();
        }
        for label in &ui.labels {
            self.fallbacks(&label.family, &label.fallbacks)?;
            let Some(engine) = &mut self.text else {
                return Err(
                    "the game's UI has labels but the config names no font (Config::font)".into(),
                );
            };
            self.span.text.clone_from(&label.text);
            self.span.face.family.clone_from(&label.family);
            self.span.face.size = label.size;
            self.span.face.line = label.line_height();
            self.span.face.weight = label.weight;
            let spans = std::slice::from_ref(&self.span);
            let drop = match label.height {
                Some(height) => {
                    let block = engine
                        .layout_spans(spans, label.wrap, placement.scale, anchor(label.anchor))
                        .map_err(|error| format!("label {:?}: {error:?}", label.text))?;
                    (height - block.height) * share(label.vertical)
                }
                None => 0.0,
            };
            let at = Placement {
                origin: [label.at[0], label.at[1] + drop],
                ..placement
            };
            let placed = engine
                .place_spans(spans, label.wrap, anchor(label.anchor), &at)
                .map_err(|error| format!("label {:?}: {error:?}", label.text))?;
            let transform = label.transform.map_or(IDENTITY, |turn| {
                let [x, y] = label.at;
                multiply(multiply(translation(x, y), turn), translation(-x, -y))
            });
            for (page, quads) in page_quads(&placed.quads) {
                runs.push(Run {
                    sheet: Sheet::Page(page),
                    quads: Quads::Own(quads),
                    colour: label.colour,
                    elevation: label.elevation,
                    id: label.id,
                    transform,
                });
            }
        }
        if let Some((atlas, _)) = &self.glyphs {
            for (prompt, parts) in ui.prompts.iter().zip(parts) {
                let mut sources = Sources {
                    atlas,
                    text: self.text.as_mut(),
                    labels,
                };
                let quads = prompt::layout(
                    parts,
                    prompt.at,
                    prompt.height,
                    &Style::of(prompt, pixels_per_unit),
                    &mut sources,
                )?;
                let index = self.prompts.len();
                runs.push(Run {
                    sheet: Sheet::Glyphs,
                    quads: Quads::Glyphs(index),
                    colour: prompt.colour,
                    elevation: prompt.elevation,
                    id: prompt.id,
                    transform: IDENTITY,
                });
                for (part, (page, _)) in quads.labels.iter().enumerate() {
                    runs.push(Run {
                        sheet: Sheet::Page(*page),
                        quads: Quads::Label(index, part),
                        colour: prompt.colour,
                        elevation: prompt.elevation,
                        id: prompt.id,
                        transform: IDENTITY,
                    });
                }
                self.prompts.push(quads);
            }
        }
        if let Some(engine) = &mut self.text {
            engine.end_frame();
        }
        self.uploads = self.sync_pages()?;
        Ok(runs)
    }

    fn clear_source(&mut self, colour: Srgba) -> Result<(wgpu::TextureView, [u32; 2]), String> {
        let gpu = self.renderer.gpu();
        let size = [1, 1];
        let source =
            self.upscale
                .source_target(&gpu.device, size, self.renderer.output_format())?;
        let [r, g, b, a] = colour.0.map(f64::from);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx game clear"),
            });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("pfx game clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &source,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        gpu.queue.submit([encoder.finish()]);
        Ok((source, size))
    }

    fn scene_source(
        &mut self,
        driver: &mut Driver,
        report: &ScreenReport,
        always: bool,
    ) -> Result<bool, String> {
        let content = [
            (report.content.width.round() as u32).max(1),
            (report.content.height.round() as u32).max(1),
        ];
        let size = self.render_size(content, report.render_size(self.render_scale))?;
        let (capacity_width, capacity_height) = self.renderer.capacity();
        let source = self.upscale.source_target(
            &self.renderer.gpu().device,
            [capacity_width, capacity_height],
            self.renderer.output_format(),
        )?;
        let aspect = size[0] as f32 / size[1] as f32;
        let Some(staged) = &self.staged else {
            return Err("a 3D frame needs a scene".into());
        };
        let finish = staged.finish();
        let frame = driver
            .session_mut()
            .present(staged, &mut self.renderer, aspect, self.seed)
            .map_err(|error| error.to_string())?;
        let next = Next::new(&frame.scene, &frame.text, &frame.effects, finish);
        self.renderer.poll_reflections()?;
        let changed = self.renderer.needs_redraw(&next);
        let drew = changed || always || self.source.as_ref().is_none_or(|(_, at)| *at != size);
        if drew {
            self.renderer
                .render(&frame.scene, &frame.text, &frame.effects, finish, &source)?;
        }
        self.source = Some((source, size));
        Ok(drew)
    }

    pub fn redraw(&self) -> Redraw {
        self.redraw
    }

    pub fn prepare(&mut self, driver: &mut Driver, forced: bool) -> Result<bool, String> {
        self.follow_graphics(&driver.settings().graphics)?;
        let Some(report) = driver.report().copied() else {
            return Ok(false);
        };
        let always = self.redraw == Redraw::Always;
        let scene = self.staged.is_some() && self.scene_source(driver, &report, always)?;
        let requested = driver.session().world().take_frame_request();
        let hud = driver.session().hud_shown();
        Ok(always || forced || scene || requested || hud || self.changed(driver, &report, hud))
    }

    fn changed(&self, driver: &Driver, report: &ScreenReport, hud: bool) -> bool {
        let Some(shown) = &self.shown else {
            return true;
        };
        let ui = driver.ui();
        shown.report != *report
            || shown.window != driver.window()
            || shown.hud != hud
            || *shown.ui != *ui
            || shown.parts != driver.prompt_parts()
            || (!ui.prompts.is_empty() && shown.labels.as_ref() != Some(driver.labels()))
    }

    pub fn draw(
        &mut self,
        driver: &mut Driver,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> Result<Option<Drawn>, String> {
        let Some(report) = driver.report().copied() else {
            return Ok(None);
        };
        if self.staged.is_some() {
            self.scene_source(driver, &report, true)?;
        }
        self.compose(driver, output, format)
    }

    pub fn compose(
        &mut self,
        driver: &mut Driver,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> Result<Option<Drawn>, String> {
        let window = driver.window();
        let Some(report) = driver.report().copied() else {
            return Ok(None);
        };
        let ui = driver.shared_ui();
        let (source, size) = match (&self.staged, &self.source) {
            (Some(_), Some(source)) => source.clone(),
            (Some(_), None) => return Err("the 3D frame has not been drawn yet".into()),
            (None, _) => self.clear_source(ui.clear.unwrap_or(BLACK))?,
        };
        let gpu = self.renderer.gpu();
        let curve = ui.curve.as_ref().unwrap_or(&self.none);
        let backdrop = FlatScene {
            layout: report.window_units(),
            clear: None,
            curve,
            light: ui.light,
            draws: &ui.backdrop,
            groups: &ui.groups,
            text: &[],
            icons: self.icons.as_ref(),
            sprites: self.sprites.as_ref(),
            environment: self.environment.as_ref(),
            post: false,
            frame: driver.frames() as u32,
            seed: self.seed,
        };
        let backdrop = (report.backdrop && !ui.backdrop.is_empty()).then_some(&backdrop);
        let output_size = [window.width.max(1), window.height.max(1)];
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx game frame"),
            });
        self.upscale.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &Compose {
                source: &source,
                source_format: self.renderer.output_format(),
                source_size: size,
                output,
                output_format: format,
                output_size,
                content: report.content,
                bars: report.bar_colour,
                filter: self.filter,
                backdrop,
            },
            None,
        )?;
        gpu.queue.submit([encoder.finish()]);
        let content_units = report.content_units();
        let paint = PaintFrame {
            format,
            output: output_size,
            content: [
                report.content.x,
                report.content.y,
                report.content.width,
                report.content.height,
            ],
            clip: clip(report.content, output_size),
            render: self.staged.is_some().then_some(size),
            layout: report.layout_units(),
            content_units: [
                content_units.x,
                content_units.y,
                content_units.width,
                content_units.height,
            ],
            window_units: report.window_units(),
            pixels_per_unit: report.pixels_per_unit(),
            frame: driver.frames(),
            seed: self.seed,
        };
        driver
            .session_mut()
            .game_mut()
            .paint(&mut self.renderer, output, &paint)
            .map_err(|error| error.to_string())?;
        let runs = self.runs(
            &ui,
            driver.prompt_parts(),
            driver.labels(),
            report.pixels_per_unit(),
        )?;
        let mut draws = std::mem::take(&mut self.draws);
        draws.clear();
        draws.extend_from_slice(&ui.draws);
        let mut texts = Vec::with_capacity(runs.len());
        for run in &runs {
            let atlas = match run.sheet {
                Sheet::Page(page) => self
                    .pages
                    .page(page)
                    .ok_or_else(|| format!("the UI's glyph page {page} is missing"))?,
                Sheet::Glyphs => self
                    .glyphs
                    .as_ref()
                    .map(|(_, atlas)| atlas)
                    .ok_or("the input glyph atlas is missing")?,
            };
            let quads: &[RichQuad] = match &run.quads {
                Quads::Own(quads) => quads,
                Quads::Glyphs(index) => &self.prompts[*index].glyphs,
                Quads::Label(index, part) => &self.prompts[*index].labels[*part].1,
            };
            texts.push(FlatText {
                atlas,
                glyphs: Glyphs::Quads(quads),
                colour: run.colour,
            });
            draws.push(
                Draw::new(Shape::Text(texts.len() - 1))
                    .transform(run.transform)
                    .elevation(run.elevation)
                    .id(run.id),
            );
        }
        let look = ui.look.as_ref().map(|look| look.frame());
        if !draws.is_empty() || look.is_some() {
            let hud = FlatScene {
                layout: report.layout_units(),
                clear: self.staged.is_none().then(|| ui.clear.unwrap_or(BLACK)),
                curve: ui.curve.as_ref().unwrap_or(&self.none),
                light: ui.light,
                draws: &draws,
                groups: &ui.groups,
                text: &texts,
                icons: self.icons.as_ref(),
                sprites: self.sprites.as_ref(),
                environment: self.environment.as_ref(),
                post: false,
                frame: driver.frames() as u32,
                seed: self.seed,
            };
            let gpu = self.renderer.gpu();
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("pfx game ui"),
                });
            self.upscale.set_view(ui.view);
            self.upscale.encode_overlay(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &Overlay {
                    scene: &hud,
                    look: look.as_ref(),
                    output,
                    output_format: format,
                    output_size,
                    content: report.content,
                },
                None,
            )?;
            gpu.queue.submit([encoder.finish()]);
        }
        self.draws = draws;
        driver
            .session_mut()
            .draw_hud_on(&mut self.renderer, output, format, output_size);
        self.shown = Some(Shown {
            labels: (!ui.prompts.is_empty()).then(|| driver.labels().clone()),
            ui: Rc::clone(&ui),
            parts: driver.prompt_parts().to_vec(),
            report,
            window,
            hud: driver.session().hud_shown(),
        });
        Ok(Some(Drawn {
            render: if self.staged.is_some() { size } else { [0, 0] },
            output: output_size,
            labels: ui.labels.len(),
            prompts: ui.prompts.len(),
            uploads: self.uploads,
        }))
    }
}

pub struct Warmup {
    passes: u32,
    start_ns: Option<u64>,
    budget: f64,
}

impl Warmup {
    pub fn new(budget: f64) -> Self {
        Self {
            passes: 0,
            start_ns: None,
            budget,
        }
    }

    pub fn passes(&self) -> u32 {
        self.passes
    }

    pub fn pass(
        &mut self,
        painter: &mut Painter,
        driver: &mut Driver,
        format: wgpu::TextureFormat,
        now_ns: u64,
    ) -> Result<bool, String> {
        let start = *self.start_ns.get_or_insert(now_ns);
        let elapsed = now_ns.saturating_sub(start) as f64 / 1e9;
        let pass = self.passes;
        self.passes += 1;
        if pass > 0 && elapsed >= self.budget {
            return Ok(true);
        }
        Ok(painter.warm(driver, format, pass, elapsed, self.budget)? == Warming::Done)
    }
}

struct PainterWarmer<'a> {
    painter: &'a mut Painter,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    pixels_per_unit: f32,
}

impl Warmer for PainterWarmer<'_> {
    fn glyphs(
        &mut self,
        family: &str,
        weight: u16,
        sizes: &[f32],
        chars: &str,
    ) -> Result<usize, String> {
        let Some(engine) = &mut self.painter.text else {
            return Err("warming glyphs needs a font in the config (Config::font)".into());
        };
        let list: Vec<Prefill> = sizes
            .iter()
            .map(|&size| Prefill {
                face: Face {
                    family: family.to_owned(),
                    size,
                    line: size * 1.25,
                    weight,
                    italic: false,
                    spacing: 0.0,
                },
                chars: chars.to_owned(),
                scale: self.pixels_per_unit.max(f32::MIN_POSITIVE),
                representation: Representation::Msdf,
                bins: [true; 4],
                number: None,
            })
            .collect();
        let made = engine
            .prefill(&list)
            .map_err(|error| format!("warming the glyphs of {family:?}: {error:?}"))?;
        self.painter.sync_pages()?;
        Ok(made)
    }

    fn looks(&mut self, looks: &[Look]) -> Result<(), String> {
        let gpu = self.painter.renderer.gpu();
        self.painter
            .upscale
            .warm_overlay(&gpu.device, &gpu.queue, self.format, self.size, looks)
    }

    fn renderer(&mut self) -> Option<&mut Renderer> {
        Some(&mut self.painter.renderer)
    }

    fn format(&self) -> Option<wgpu::TextureFormat> {
        Some(self.format)
    }

    fn size(&self) -> Option<[u32; 2]> {
        Some(self.size)
    }
}

pub fn resolution(quality: Option<pfx_settings::Quality>, game: Option<Budget>) -> Option<Budget> {
    match quality {
        Some(pfx_settings::Quality::Appearance) => None,
        Some(pfx_settings::Quality::Performance) => {
            Some(game.unwrap_or_else(|| Budget::new(f64::from(pfx_settings::BUDGET_MS))))
        }
        None => game,
    }
}
