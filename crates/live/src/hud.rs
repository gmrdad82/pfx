use egui::{
    Align, Area, Color32, Context, Frame, FullOutput, Id, Layout, Margin, Order, Pos2, RawInput,
    Rect, Sense, Stroke, TexturesDelta, ViewportId, ViewportInfo, epaint::ClippedPrimitive, vec2,
};
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use pfx_gpu::Gpu;
use pfx_input::device::Key;
use pfx_input::event::InputEvent;
use pfx_theme::style::Theme;
use pfx_theme::style::theme::apply_theme;

use crate::renderer::Renderer;
use crate::stats::{FrameKind, FrameStats, Sample};

pub const TOGGLE: Key = Key::F3;
pub const BUDGET_MS: f64 = 1000.0 / 60.0;
pub const WINDOW_MS: f64 = 4000.0;
pub const KEPT: usize = 512;
pub const WIDTH: f32 = 216.0;
pub const MARGIN: f32 = 12.0;
pub const GRAPH_HEIGHT: f32 = 48.0;
const CEILING_MS: f64 = 2.0 * BUDGET_MS;
const CANVAS: f32 = 512.0;
const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Readout {
    pub sim_ms: Option<f64>,
    pub submit_ms: Option<f64>,
    pub gpu: Option<(u64, f64)>,
    pub kind: Option<FrameKind>,
    pub hud_ms: Option<f64>,
    pub frames: Vec<(f64, f64)>,
    pub over: usize,
}

impl Readout {
    pub fn from_samples(samples: &[Sample]) -> Self {
        let Some(latest) = samples.last() else {
            return Self::default();
        };
        let start = latest.t_ms - WINDOW_MS;
        let frames: Vec<(f64, f64)> = samples
            .iter()
            .filter(|sample| sample.t_ms >= start)
            .filter_map(|sample| {
                sample
                    .interval_ms
                    .map(|interval| (sample.t_ms - start, interval))
            })
            .collect();
        Self {
            sim_ms: latest.sim_ms,
            submit_ms: Some(latest.submit_ms),
            gpu: samples
                .iter()
                .rev()
                .find_map(|sample| sample.gpu_ms.map(|ms| (sample.tick, ms))),
            kind: Some(latest.kind),
            hud_ms: samples.iter().rev().find_map(|sample| sample.hud_ms),
            over: frames
                .iter()
                .filter(|(_, interval)| *interval > BUDGET_MS)
                .count(),
            frames,
        }
    }
}

fn millis(value: Option<f64>) -> String {
    value.map_or_else(|| "-".into(), |ms| format!("{ms:.2} ms"))
}

fn kind_colour(theme: &Theme, kind: FrameKind) -> Color32 {
    match kind {
        FrameKind::Full => theme.text,
        FrameKind::Partial => theme.secondary,
        FrameKind::Reproject => theme.secondary_soft,
        FrameKind::Idle => theme.muted,
    }
}

fn row(ui: &mut egui::Ui, theme: &Theme, name: &str, value: String, colour: Color32) {
    ui.horizontal(|ui| {
        ui.label(pfx_theme::label(theme, name, theme.text2));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                egui::RichText::new(value)
                    .font(theme.font(theme.size_body))
                    .color(colour),
            );
        });
    });
}

fn graph(ui: &mut egui::Ui, theme: &Theme, readout: &Readout) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), GRAPH_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme.well);
    let y = |ms: f64| rect.bottom() - (ms.min(CEILING_MS) / CEILING_MS) as f32 * rect.height();
    let x = |ms: f64| rect.left() + (ms / WINDOW_MS).clamp(0.0, 1.0) as f32 * rect.width();
    for &(end, interval) in &readout.frames {
        let colour = if interval > CEILING_MS {
            theme.error
        } else if interval > BUDGET_MS {
            theme.warning
        } else {
            theme.secondary
        };
        let left = x(end - interval);
        let right = x(end).max(left + 1.0);
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(left, y(interval)),
                Pos2::new(right, rect.bottom()),
            ),
            0.0,
            colour,
        );
    }
    let budget = y(BUDGET_MS).round() + 0.5;
    painter.hline(rect.x_range(), budget, Stroke::new(theme.stroke, theme.hot));
}

pub fn ui(ctx: &Context, readout: &Readout) -> Rect {
    let theme = Theme::of(ctx);
    let theme = theme.as_ref();
    Area::new(Id::new("pfx dev hud"))
        .fixed_pos(Pos2::ZERO)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            Frame::new()
                .fill(theme.bg)
                .stroke(Stroke::new(theme.stroke, theme.line))
                .inner_margin(Margin::same(theme.panel_padding))
                .corner_radius(theme.radius)
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    ui.label(pfx_theme::title(
                        theme,
                        "frame",
                        theme.size_body,
                        theme.text,
                    ));
                    row(ui, theme, "sim", millis(readout.sim_ms), theme.text);
                    row(ui, theme, "submit", millis(readout.submit_ms), theme.text);
                    row(
                        ui,
                        theme,
                        "gpu",
                        millis(readout.gpu.map(|(_, ms)| ms)),
                        theme.text,
                    );
                    let (kind, colour) = match readout.kind {
                        Some(kind) => (kind.name().to_uppercase(), kind_colour(theme, kind)),
                        None => ("-".into(), theme.muted),
                    };
                    row(ui, theme, "kind", kind, colour);
                    row(ui, theme, "hud", millis(readout.hud_ms), theme.text2);
                    graph(ui, theme, readout);
                    let over = if readout.over > 0 {
                        theme.warning
                    } else {
                        theme.muted
                    };
                    row(ui, theme, "over 16.67", readout.over.to_string(), over);
                });
        })
        .response
        .rect
}

pub fn layout(
    ctx: &Context,
    pixels_per_point: f32,
    readout: &Readout,
    passes: usize,
) -> (Rect, FullOutput) {
    let mut rect = Rect::NOTHING;
    let mut textures = TexturesDelta::default();
    let mut full = FullOutput::default();
    for _ in 0..passes.max(1) {
        full = ctx.run(input(pixels_per_point), |ctx| rect = ui(ctx, readout));
        textures.append(std::mem::take(&mut full.textures_delta));
    }
    full.textures_delta = textures;
    (rect, full)
}

pub fn context() -> Context {
    let ctx = Context::default();
    apply_theme(&ctx, &pfx_theme::pfx());
    ctx
}

pub fn input(pixels_per_point: f32) -> RawInput {
    let mut input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(CANVAS, CANVAS))),
        max_texture_side: Some(2048),
        ..RawInput::default()
    };
    input.viewports.insert(
        ViewportId::ROOT,
        ViewportInfo {
            native_pixels_per_point: Some(pixels_per_point),
            ..ViewportInfo::default()
        },
    );
    input
}

pub fn default_scale(height: u32) -> f32 {
    (height as f32 / 1080.0).max(1.0)
}

pub fn placement(size: [u32; 2], panel: [u32; 2], pixels_per_point: f32) -> Option<[u32; 4]> {
    let margin = (MARGIN * pixels_per_point).round() as u32;
    let width = panel[0].min(size[0].saturating_sub(2 * margin));
    let height = panel[1].min(size[1].saturating_sub(2 * margin));
    if width == 0 || height == 0 {
        return None;
    }
    Some([size[0] - margin - width, margin, width, height])
}

fn encodes(format: wgpu::TextureFormat) -> bool {
    !format.is_srgb()
        && !matches!(
            format,
            wgpu::TextureFormat::Rgba16Float
                | wgpu::TextureFormat::Rgba32Float
                | wgpu::TextureFormat::Rg11b10Ufloat
        )
}

const COMPOSITE: &str = r#"
struct Place {
    origin: vec2<i32>,
    encode: u32,
    pad: u32,
}

@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var<uniform> place: Place;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn gamma(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = textureLoad(canvas, vec2<i32>(at.xy) - place.origin, 0);
    if place.encode == 1u && colour.a > 0.0 {
        return vec4<f32>(gamma(colour.rgb / colour.a) * colour.a, colour.a);
    }
    return colour;
}
"#;

struct Canvas {
    size: [u32; 2],
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

struct Paint {
    format: wgpu::TextureFormat,
    egui: egui_wgpu::Renderer,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    place: wgpu::Buffer,
    canvas: Option<Canvas>,
}

impl Paint {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pfx dev hud composite"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pfx dev hud composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pfx dev hud composite"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pfx dev hud composite"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let place = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pfx dev hud place"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            format,
            egui: egui_wgpu::Renderer::new(device, CANVAS_FORMAT, RendererOptions::PREDICTABLE),
            pipeline,
            layout,
            place,
            canvas: None,
        }
    }

    fn canvas(&mut self, gpu: &Gpu, size: [u32; 2]) -> &Canvas {
        if self
            .canvas
            .as_ref()
            .is_none_or(|canvas| canvas.size != size)
        {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pfx dev hud canvas"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: CANVAS_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pfx dev hud composite"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.place.as_entire_binding(),
                    },
                ],
            });
            self.canvas = Some(Canvas { size, view, bind });
        }
        self.canvas.as_ref().unwrap()
    }

    fn upload(&mut self, gpu: &Gpu, textures: &TexturesDelta) {
        for (id, delta) in &textures.set {
            self.egui
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        gpu: &Gpu,
        output: &wgpu::TextureView,
        textures: &TexturesDelta,
        primitives: &[ClippedPrimitive],
        pixels_per_point: f32,
        panel: [u32; 2],
        at: [u32; 4],
    ) {
        self.upload(gpu, textures);
        let mut place = [0u8; 16];
        place[0..4].copy_from_slice(&(at[0] as i32).to_le_bytes());
        place[4..8].copy_from_slice(&(at[1] as i32).to_le_bytes());
        place[8..12].copy_from_slice(&u32::from(encodes(self.format)).to_le_bytes());
        gpu.queue.write_buffer(&self.place, 0, &place);
        let screen = ScreenDescriptor {
            size_in_pixels: panel,
            pixels_per_point,
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx dev hud"),
            });
        let mut commands =
            self.egui
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, primitives, &screen);
        let canvas = self.canvas(gpu, panel);
        let (view, bind) = (canvas.view.clone(), canvas.bind.clone());
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pfx dev hud ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut pass = pass.forget_lifetime();
            self.egui.render(&mut pass, primitives, &screen);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pfx dev hud composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_viewport(
                at[0] as f32,
                at[1] as f32,
                at[2] as f32,
                at[3] as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(at[0], at[1], at[2], at[3]);
            pass.draw(0..3, 0..1);
        }
        commands.push(encoder.finish());
        gpu.queue.submit(commands);
        for id in &textures.free {
            self.egui.free_texture(id);
        }
    }
}

#[derive(Default)]
pub struct Hud {
    visible: bool,
    held: bool,
    ctx: Option<Context>,
    paint: Option<Paint>,
    rect: Option<[u32; 4]>,
}

impl Hud {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn rect(&self) -> Option<[u32; 4]> {
        self.rect
    }

    pub fn input(&mut self, event: &InputEvent) -> bool {
        match event {
            InputEvent::Key { key, pressed } if *key == TOGGLE => {
                if *pressed && !self.held {
                    self.toggle();
                }
                self.held = *pressed;
                true
            }
            InputEvent::FocusLost => {
                self.held = false;
                false
            }
            _ => false,
        }
    }

    pub fn attach(renderer: &mut Renderer) -> FrameStats {
        let mut stats = renderer.frame_stats();
        if !stats.enabled() {
            stats = FrameStats::measure();
            renderer.set_frame_stats(stats.clone());
        }
        stats.keep_recent(KEPT);
        stats
    }

    pub fn draw(&mut self, renderer: &mut Renderer, output: &wgpu::TextureView) {
        let format = renderer.output_format();
        let (width, height) = renderer.size();
        self.draw_on(renderer, output, format, [width, height]);
    }

    pub fn draw_on(
        &mut self,
        renderer: &mut Renderer,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
    ) {
        let stats = Self::attach(renderer);
        if !self.visible {
            self.rect = None;
            return;
        }
        stats.hud_begin();
        self.paint(renderer, output, format, size, &stats);
        stats.hud_end();
    }

    fn paint(
        &mut self,
        renderer: &Renderer,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        stats: &FrameStats,
    ) {
        let [width, height] = size;
        let scale = default_scale(height);
        let readout = stats.with_recent(Readout::from_samples);
        let passes = if self.ctx.is_none() { 2 } else { 1 };
        let ctx = self.ctx.get_or_insert_with(context);
        let (rect, full) = layout(ctx, scale, &readout, passes);
        let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
        let panel = [
            ((rect.max.x * scale).ceil() as u32).clamp(1, (CANVAS * scale) as u32),
            ((rect.max.y * scale).ceil() as u32).clamp(1, (CANVAS * scale) as u32),
        ];
        let gpu = renderer.gpu();
        if self
            .paint
            .as_ref()
            .is_none_or(|paint| paint.format != format)
        {
            self.paint = Some(Paint::new(gpu, format));
        }
        let Some(paint) = self.paint.as_mut() else {
            return;
        };
        let Some(at) = placement([width, height], panel, scale) else {
            paint.upload(gpu, &full.textures_delta);
            self.rect = None;
            return;
        };
        paint.draw(
            gpu,
            output,
            &full.textures_delta,
            &primitives,
            scale,
            panel,
            at,
        );
        self.rect = Some(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tick: u64, t_ms: f64, interval_ms: Option<f64>) -> Sample {
        Sample {
            tick,
            t_ms,
            sim_ms: Some(0.5),
            submit_ms: 2.0,
            gpu_ms: None,
            interval_ms,
            kind: FrameKind::Full,
            size: [320, 180],
            hud_ms: None,
        }
    }

    #[test]
    fn f3_toggles_once_per_press_and_other_input_passes_through() {
        let mut hud = Hud::new();
        assert!(!hud.visible());
        let press = InputEvent::Key {
            key: Key::F3,
            pressed: true,
        };
        let release = InputEvent::Key {
            key: Key::F3,
            pressed: false,
        };
        assert!(hud.input(&press));
        assert!(hud.visible());
        assert!(hud.input(&press));
        assert!(hud.visible());
        assert!(hud.input(&release));
        assert!(hud.input(&press));
        assert!(!hud.visible());
        assert!(!hud.input(&InputEvent::FocusLost));
        assert!(hud.input(&press));
        assert!(hud.visible());
        let other = InputEvent::Key {
            key: Key::F4,
            pressed: true,
        };
        assert!(!hud.input(&other));
        assert!(hud.visible());
    }

    #[test]
    fn the_readout_takes_the_newest_numbers_and_the_last_four_seconds() {
        assert_eq!(Readout::from_samples(&[]), Readout::default());
        let mut samples: Vec<Sample> = (0..300)
            .map(|tick| {
                let interval = if tick % 100 == 0 { 40.0 } else { 16.0 };
                sample(tick, tick as f64 * 16.0, (tick > 0).then_some(interval))
            })
            .collect();
        samples[297].gpu_ms = Some(3.25);
        samples[296].hud_ms = Some(0.75);
        samples[299].kind = FrameKind::Partial;
        samples[299].sim_ms = None;
        let readout = Readout::from_samples(&samples);
        assert_eq!(readout.gpu, Some((297, 3.25)));
        assert_eq!(readout.hud_ms, Some(0.75));
        assert_eq!(readout.kind, Some(FrameKind::Partial));
        assert_eq!(readout.sim_ms, None);
        assert_eq!(readout.submit_ms, Some(2.0));
        assert_eq!(readout.frames.len(), 251);
        assert!(
            readout
                .frames
                .iter()
                .all(|(end, _)| (0.0..=WINDOW_MS).contains(end))
        );
        assert_eq!(readout.over, 2);
    }

    #[test]
    fn the_panel_lays_out_the_same_in_pfx_theme_and_sits_top_right() {
        let readout = Readout::from_samples(&[sample(0, 0.0, None), sample(1, 16.0, Some(16.0))]);
        let ctx = context();
        let mut rects = Vec::new();
        let mut texts = Vec::new();
        for passes in [2, 1] {
            let (rect, full) = layout(&ctx, 1.0, &readout, passes);
            rects.push(rect);
            texts = full
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                    _ => None,
                })
                .collect();
        }
        assert_eq!(rects[0], rects[1]);
        assert_eq!(rects[0].min, Pos2::ZERO);
        assert!(rects[0].width() > WIDTH && rects[0].width() < WIDTH + 40.0);
        for want in [
            "FRAME", "SIM", "SUBMIT", "GPU", "KIND", "HUD", "FULL", "2.00 ms", "0.50 ms",
        ] {
            assert!(texts.iter().any(|text| text == want), "{want} in {texts:?}");
        }
        assert_eq!(Theme::of(&ctx).bg, pfx_theme::BG);
        assert_eq!(
            placement([1920, 1080], [240, 200], 1.0),
            Some([1920 - 12 - 240, 12, 240, 200])
        );
        assert_eq!(
            placement([3840, 2160], [480, 400], 2.0),
            Some([3840 - 24 - 480, 24, 480, 400])
        );
        assert_eq!(
            placement([100, 50], [240, 200], 1.0),
            Some([12, 12, 76, 26])
        );
        assert_eq!(placement([20, 20], [240, 200], 1.0), None);
        assert_eq!(default_scale(800), 1.0);
        assert_eq!(default_scale(2160), 2.0);
        assert!(encodes(wgpu::TextureFormat::Rgba8Unorm));
        assert!(!encodes(wgpu::TextureFormat::Rgba8UnormSrgb));
        assert!(!encodes(wgpu::TextureFormat::Rgba16Float));
    }
}
