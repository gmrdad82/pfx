use std::path::{Path, PathBuf};

use egui::{Color32, Context, Rect, TextureId, Vec2};
use egui_wgpu::Renderer;
use pfx_editor_shell::paint::clear;
use pfx_editor_shell::{App, Script, Theme, Timing, context, drive, gpu_turn, shot};
use pfx_editor_style::fixture;
use pfx_gpu::{Gpu, wgpu};

#[derive(Default)]
struct Form {
    clicks: u32,
    name: String,
    level: f32,
    button: Option<Rect>,
    field: Option<Rect>,
    slider: Option<Rect>,
}

impl App for Form {
    fn ui(&mut self, ctx: &Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("form");
            let button = ui.button(format!("pressed {}", self.clicks));
            if button.clicked() {
                self.clicks += 1;
            }
            self.button = Some(button.rect);
            let field = ui.text_edit_singleline(&mut self.name);
            self.field = Some(field.rect);
            let slider = ui.add(egui::Slider::new(&mut self.level, 0.0..=100.0).show_value(false));
            self.slider = Some(slider.rect);
        });
    }

    fn theme(&self) -> Theme {
        fixture::theme()
    }
}

fn centre(rect: Option<Rect>) -> String {
    let at = rect.unwrap().center();
    format!("{} {}", at.x, at.y)
}

fn form_script(form: &Form) -> Script {
    let slider = form.slider.unwrap();
    let from = slider.center();
    let to = from + Vec2::new(slider.width() / 4.0, 0.0);
    Script::parse(&format!(
        "click {button}\nframe\nclick {button}\nclick {field}\ntext typed words\nframe\ndrag {} {} {} {}\nframe\n",
        from.x,
        from.y,
        to.x,
        to.y,
        button = centre(form.button),
        field = centre(form.field),
    ))
    .unwrap()
}

fn laid_out() -> Form {
    let ctx = context(&fixture::theme());
    let mut form = Form::default();
    drive(&ctx, SIZE, &Script::default(), |ctx| form.ui(ctx));
    form
}

const SIZE: [u32; 2] = [320, 200];

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/shell-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn a_script_clicks_types_and_drags_into_the_app() {
    let script = form_script(&laid_out());
    let ctx = context(&fixture::theme());
    let mut form = Form::default();
    drive(&ctx, SIZE, &script, |ctx| form.ui(ctx));
    assert_eq!(form.clicks, 2);
    assert_eq!(form.name, "typed words");
    assert!(form.level > 55.0 && form.level < 95.0, "{}", form.level);
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn a_form_shot_is_the_same_bytes_every_run() {
    let since = std::time::Instant::now();
    let script = form_script(&laid_out());
    let mut first = Form::default();
    let one = shot(&mut first, SIZE, &script).unwrap();
    let mut second = Form::default();
    let two = shot(&mut second, SIZE, &script).unwrap();
    assert_eq!(first.clicks, 2);
    assert_eq!(first.name, "typed words");
    assert_eq!(one, two);
    std::fs::write(scratch("form.png"), &one).unwrap();
    gpu_turn(since);
}

const VIEW: Color32 = Color32::from_rgb(0xC0, 0x40, 0x20);

#[derive(Default)]
struct Viewport {
    rect: Option<Rect>,
    texture: Option<(wgpu::Texture, TextureId, [u32; 2])>,
    forgotten: u32,
    frames: u32,
}

impl App for Viewport {
    fn ui(&mut self, ctx: &Context) {
        egui::TopBottomPanel::top("bar").show(ctx, |ui| ui.label("viewport"));
        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            self.rect = Some(rect);
            if let Some((_, id, _)) = &self.texture {
                ui.painter().image(
                    *id,
                    rect,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        });
    }

    fn theme(&self) -> Theme {
        fixture::theme()
    }

    fn viewport(&self) -> bool {
        true
    }

    fn frame(&mut self, gpu: &Gpu, renderer: &mut Renderer, pixels_per_point: f32) -> bool {
        self.frames += 1;
        let Some(rect) = self.rect else {
            return false;
        };
        let size = [
            ((rect.width() * pixels_per_point).round() as u32).max(1),
            ((rect.height() * pixels_per_point).round() as u32).max(1),
        ];
        if self.texture.as_ref().is_none_or(|(.., at)| *at != size) {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("viewport"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
            });
            let shown = texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(wgpu::TextureFormat::Rgba8Unorm),
                ..wgpu::TextureViewDescriptor::default()
            });
            let id =
                renderer.register_native_texture(&gpu.device, &shown, wgpu::FilterMode::Nearest);
            self.texture = Some((texture, id, size));
        }
        let Some((texture, ..)) = &self.texture else {
            return false;
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("viewport"),
            });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewport"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear(VIEW)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        gpu.queue.submit([encoder.finish()]);
        false
    }

    fn forget(&mut self) {
        self.texture = None;
        self.forgotten += 1;
    }

    fn timing(&self) -> Timing {
        Timing {
            drawn: self.texture.is_some(),
            ..Timing::default()
        }
    }
}

fn pixel(png: &[u8], at: [u32; 2]) -> [u8; 4] {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).unwrap();
    let start = ((at[1] * info.width + at[0]) * 4) as usize;
    rgba[start..start + 4].try_into().unwrap()
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn a_viewport_apps_frame_shows_in_the_shot() {
    let since = std::time::Instant::now();
    let mut app = Viewport::default();
    let png = shot(&mut app, SIZE, &Script::default()).unwrap();
    std::fs::write(scratch("viewport.png"), &png).unwrap();
    assert_eq!(app.forgotten, 1);
    assert!(app.frames >= 3, "{}", app.frames);
    let rect = app.rect.unwrap();
    let inside = pixel(&png, [rect.center().x as u32, rect.center().y as u32]);
    let want = [VIEW.r(), VIEW.g(), VIEW.b(), 255];
    assert!(
        inside
            .iter()
            .zip(want)
            .all(|(got, want)| got.abs_diff(want) <= 2),
        "{inside:?}"
    );
    let again = shot(&mut app, SIZE, &Script::default()).unwrap();
    assert_eq!(app.forgotten, 2);
    assert_eq!(png, again);
    gpu_turn(since);
}

const GROUND: Color32 = Color32::from_rgb(0x12, 0x5A, 0x3C);
const INK: Color32 = Color32::from_rgb(0xF2, 0xD0, 0x4A);

fn test_theme() -> Theme {
    let mut theme = fixture::theme();
    theme.name = "test".into();
    theme.bg = GROUND;
    theme.text = INK;
    theme
}

#[derive(Default)]
struct Themed {
    first: Option<(Color32, Color32)>,
}

impl App for Themed {
    fn ui(&mut self, ctx: &Context) {
        let seen = ctx.style().visuals.clone();
        self.first
            .get_or_insert((seen.panel_fill, seen.text_color()));
        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            let swatch = Rect::from_min_size(rect.min + Vec2::splat(8.0), Vec2::splat(24.0));
            ui.painter().rect_filled(swatch, 0.0, seen.text_color());
        });
    }

    fn theme(&self) -> Theme {
        test_theme()
    }
}

#[test]
fn the_shells_context_paints_only_the_apps_colours_and_faces() {
    let theme = test_theme();
    let ctx = context(&theme);
    assert_eq!(*Theme::of(&ctx), theme);
    let mut app = Themed::default();
    let output = drive(&ctx, SIZE, &Script::default(), |ctx| app.ui(ctx));
    assert_eq!(app.first, Some((GROUND, INK)));
    let allowed = fixture::colours(&theme);
    let mut seen = Vec::new();
    for clipped in &output.shapes {
        if let egui::Shape::Rect(rect) = &clipped.shape {
            seen.push(rect.fill);
        }
    }
    assert!(seen.contains(&GROUND), "{seen:?}");
    assert!(seen.contains(&INK), "{seen:?}");
    assert!(
        seen.iter().all(|colour| allowed.contains(colour)),
        "{seen:?}"
    );
    let ctx = context(&theme);
    let mut form = Form::default();
    let output = drive(&ctx, SIZE, &form_script(&laid_out()), |ctx| form.ui(ctx));
    assert!(fixture::assert_faces(&ctx, &theme, &output.shapes) >= 3);
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn an_apps_theme_paints_its_ground_and_text_from_the_first_frame() {
    let since = std::time::Instant::now();
    let mut app = Themed::default();
    let png = shot(&mut app, SIZE, &Script::default()).unwrap();
    std::fs::write(scratch("themed.png"), &png).unwrap();
    assert_eq!(app.first, Some((GROUND, INK)));
    let want = |colour: Color32| [colour.r(), colour.g(), colour.b(), 255];
    assert_eq!(pixel(&png, [SIZE[0] - 2, SIZE[1] - 2]), want(GROUND));
    assert_eq!(pixel(&png, [20, 20]), want(INK));
    gpu_turn(since);
}
