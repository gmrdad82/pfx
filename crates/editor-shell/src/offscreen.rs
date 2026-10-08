use egui::{
    Context, Event, FullOutput, Id, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput,
    Rect, Vec2, ViewportId, ViewportInfo,
};
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use pfx_gpu::{Gpu, wgpu};

use crate::paint::Painter;
use crate::{App, Error, Mods, Script, Step, Theme, block_on, context};

pub const MAX_SIDE: u32 = 8192;
const DT: f32 = 1.0 / 60.0;
const DRAG_STEPS: u32 = 4;
const SETTLE: usize = 2;
const CLOCK: &str = "pfx-editor-shell/drive-passes";

struct Driver<'a> {
    ctx: &'a Context,
    screen: Rect,
    passes: u64,
    modifiers: Modifiers,
    textures: egui::TexturesDelta,
}

impl Driver<'_> {
    fn pass(&mut self, events: Vec<Event>, ui: &mut impl FnMut(&Context)) -> FullOutput {
        let mut input = RawInput {
            screen_rect: Some(self.screen),
            max_texture_side: Some(MAX_SIDE as usize),
            time: Some(self.passes as f64 * f64::from(DT)),
            predicted_dt: DT,
            focused: true,
            modifiers: self.modifiers,
            events,
            ..RawInput::default()
        };
        input.viewports.insert(
            ViewportId::ROOT,
            ViewportInfo {
                native_pixels_per_point: Some(1.0),
                focused: Some(true),
                ..ViewportInfo::default()
            },
        );
        self.passes += 1;
        let mut output = self.ctx.run(input, |ctx| ui(ctx));
        self.textures
            .append(std::mem::take(&mut output.textures_delta));
        output
    }
}

fn modifiers_of(mods: Mods) -> Modifiers {
    Modifiers {
        alt: mods.alt,
        ctrl: mods.ctrl,
        shift: mods.shift,
        mac_cmd: false,
        command: mods.ctrl,
    }
}

fn key(name: &str, pressed: bool, modifiers: Modifiers) -> Option<Event> {
    Some(Event::Key {
        key: Key::from_name(name)?,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    })
}

fn button(at: Pos2, pressed: bool) -> Event {
    press(at, PointerButton::Primary, pressed, Modifiers::NONE)
}

fn press(at: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers,
    }
}

pub fn drive(
    ctx: &Context,
    size: [u32; 2],
    script: &Script,
    mut ui: impl FnMut(&Context),
) -> FullOutput {
    let mut driver = Driver {
        ctx,
        screen: Rect::from_min_size(Pos2::ZERO, Vec2::new(size[0] as f32, size[1] as f32)),
        passes: ctx.data(|data| data.get_temp(Id::new(CLOCK))).unwrap_or(0),
        modifiers: Modifiers::NONE,
        textures: egui::TexturesDelta::default(),
    };
    let mut last = driver.pass(Vec::new(), &mut ui);
    let mut events = Vec::new();
    let mut fresh = true;
    for step in &script.steps {
        match step {
            Step::Key(name, mods) if *mods == Mods::default() => {
                let modifiers = driver.modifiers;
                events.extend(
                    [key(name, true, modifiers), key(name, false, modifiers)]
                        .into_iter()
                        .flatten(),
                )
            }
            Step::Key(name, mods) => {
                if !events.is_empty() {
                    driver.pass(std::mem::take(&mut events), &mut ui);
                }
                let held = driver.modifiers;
                let modifiers = modifiers_of(*mods);
                driver.modifiers = held | modifiers;
                let chord = driver.modifiers;
                events.extend(
                    [key(name, true, chord), key(name, false, chord)]
                        .into_iter()
                        .flatten(),
                );
                last = driver.pass(std::mem::take(&mut events), &mut ui);
                driver.modifiers = held;
                fresh = false;
            }
            Step::Text(text) => events.push(Event::Text(text.clone())),
            Step::Move([x, y]) => events.push(Event::PointerMoved(Pos2::new(*x, *y))),
            Step::Click([x, y]) => {
                let at = Pos2::new(*x, *y);
                events.push(Event::PointerMoved(at));
                events.push(button(at, true));
                last = driver.pass(std::mem::take(&mut events), &mut ui);
                events.push(button(at, false));
                fresh = false;
            }
            Step::Press { at, mods } => {
                let at = Pos2::new(at[0], at[1]);
                let modifiers = modifiers_of(*mods);
                driver.modifiers = modifiers;
                events.push(Event::PointerMoved(at));
                events.push(press(at, PointerButton::Primary, true, modifiers));
                last = driver.pass(std::mem::take(&mut events), &mut ui);
                fresh = false;
            }
            Step::Release([x, y]) => {
                let at = Pos2::new(*x, *y);
                events.push(Event::PointerMoved(at));
                events.push(press(at, PointerButton::Primary, false, driver.modifiers));
                last = driver.pass(std::mem::take(&mut events), &mut ui);
                driver.modifiers = Modifiers::NONE;
                fresh = false;
            }
            Step::Drag {
                from,
                to,
                middle,
                mods,
            } => {
                let (from, to) = (Pos2::new(from[0], from[1]), Pos2::new(to[0], to[1]));
                let held = if *middle {
                    PointerButton::Middle
                } else {
                    PointerButton::Primary
                };
                let modifiers = modifiers_of(*mods);
                driver.modifiers = modifiers;
                events.push(Event::PointerMoved(from));
                events.push(press(from, held, true, modifiers));
                driver.pass(std::mem::take(&mut events), &mut ui);
                for step in 1..=DRAG_STEPS {
                    let at = from.lerp(to, step as f32 / DRAG_STEPS as f32);
                    events.push(Event::PointerMoved(at));
                    driver.pass(std::mem::take(&mut events), &mut ui);
                }
                events.push(press(to, held, false, modifiers));
                last = driver.pass(std::mem::take(&mut events), &mut ui);
                driver.modifiers = Modifiers::NONE;
                fresh = false;
            }
            Step::Scroll { at, lines } => {
                events.push(Event::PointerMoved(Pos2::new(at[0], at[1])));
                events.push(Event::MouseWheel {
                    unit: MouseWheelUnit::Line,
                    delta: Vec2::new(0.0, *lines),
                    modifiers: Modifiers::NONE,
                });
                last = driver.pass(std::mem::take(&mut events), &mut ui);
                fresh = false;
            }
            Step::Frame => {
                last = driver.pass(std::mem::take(&mut events), &mut ui);
                fresh = false;
            }
        }
    }
    if fresh || !events.is_empty() {
        last = driver.pass(events, &mut ui);
    }
    ctx.data_mut(|data| data.insert_temp(Id::new(CLOCK), driver.passes));
    last.textures_delta = driver.textures;
    last
}

pub fn check(size: [u32; 2]) -> Result<(), Error> {
    if size.iter().any(|side| *side == 0 || *side > MAX_SIDE) {
        return Err(Error(format!(
            "a shot is 1 to {MAX_SIDE} pixels a side, not {}x{}",
            size[0], size[1]
        )));
    }
    Ok(())
}

pub fn pixels(app: &mut impl App, size: [u32; 2], script: &Script) -> Result<Vec<u8>, Error> {
    check(size)?;
    let theme = app.theme();
    let gpu = block_on(Gpu::headless()).map_err(Error)?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut painter = Painter::new(&gpu, format, RendererOptions::PREDICTABLE);
    if !app.viewport() {
        return finish(&gpu, &mut painter, size, script, &theme, |ctx, _| {
            app.ui(ctx)
        });
    }
    app.forget();
    let mut settled = script.clone();
    settled
        .steps
        .extend(std::iter::repeat_n(Step::Frame, SETTLE));
    finish(
        &gpu,
        &mut painter,
        size,
        &settled,
        &theme,
        |ctx, painter| {
            app.ui(ctx);
            app.frame(&gpu, painter.renderer(), ctx.pixels_per_point());
        },
    )
}

fn finish(
    gpu: &Gpu,
    painter: &mut Painter,
    size: [u32; 2],
    script: &Script,
    app_theme: &Theme,
    mut ui: impl FnMut(&Context, &mut Painter),
) -> Result<Vec<u8>, Error> {
    let ctx = context(app_theme);
    let output = {
        let painter = &mut *painter;
        drive(&ctx, size, script, |ctx| ui(ctx, painter))
    };
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    let target = gpu
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba8UnormSrgb)
        .map_err(Error)?;
    let screen = ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: output.pixels_per_point,
    };
    painter.paint(
        gpu,
        &target.view,
        &screen,
        &output.textures_delta,
        &primitives,
        app_theme.bg,
    );
    gpu.readback_rgba8(&target).map_err(Error)
}

pub fn encode_png(size: [u32; 2], rgba: &[u8]) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| Error(error.to_string()))?;
    writer
        .write_image_data(rgba)
        .map_err(|error| Error(error.to_string()))?;
    writer.finish().map_err(|error| Error(error.to_string()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_editor_style::fixture;

    #[test]
    fn sizes_outside_the_limit_are_refused() {
        assert!(check([0, 10]).is_err());
        assert!(check([10, MAX_SIDE + 1]).is_err());
        assert!(check([1, MAX_SIDE]).is_ok());
    }

    #[test]
    fn a_click_spends_a_pass_and_releases_on_the_next() {
        let ctx = context(&fixture::theme());
        let script = Script::parse("click 5 5\nframe").unwrap();
        let mut seen = Vec::new();
        drive(&ctx, [64, 32], &script, |ctx| {
            seen.push(ctx.input(|input| input.pointer.primary_down()));
        });
        assert_eq!(seen, vec![false, true, false]);
    }

    #[test]
    fn two_drives_on_one_context_never_move_the_clock_back() {
        let ctx = context(&fixture::theme());
        let script = Script::parse("click 5 5\nframe").unwrap();
        let mut times = Vec::new();
        for _ in 0..2 {
            drive(&ctx, [64, 32], &script, |ctx| {
                times.push(ctx.input(|input| input.time));
            });
        }
        assert_eq!(times.len(), 6);
        assert!(times.windows(2).all(|pair| pair[0] < pair[1]), "{times:?}");
    }

    #[test]
    fn a_script_without_frames_still_draws_once() {
        let ctx = context(&fixture::theme());
        let mut passes = 0;
        drive(&ctx, [64, 32], &Script::default(), |_| passes += 1);
        assert_eq!(passes, 2);
    }

    #[test]
    fn the_png_round_trips() {
        let rgba: Vec<u8> = (0..4 * 3 * 2).map(|i| i as u8).collect();
        let bytes = encode_png([3, 2], &rgba).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut back = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut back).unwrap();
        assert_eq!(back, rgba);
    }

    struct Blank;

    impl App for Blank {
        fn ui(&mut self, _: &Context) {}

        fn theme(&self) -> Theme {
            fixture::theme()
        }
    }

    #[test]
    #[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
    fn an_empty_context_shoots_the_themes_solid_ground() {
        let since = std::time::Instant::now();
        let size = [96, 60];
        let rgba = pixels(&mut Blank, size, &Script::default()).unwrap();
        assert_eq!(rgba.len(), 96 * 60 * 4);
        let ground = fixture::theme().bg;
        let base = [ground.r(), ground.g(), ground.b(), 255];
        assert!(rgba.chunks_exact(4).all(|pixel| pixel == base));
        crate::gpu_turn(since);
    }
}
