use egui::{
    Context, Event, FullOutput, Id, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput,
    Rect, TouchPhase, Ui, Vec2, ViewportId, ViewportInfo,
};
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use pfx_gpu::{Gpu, wgpu};

use crate::mean::Mean;
use crate::paint::Painter;
use crate::shutter::{self, Shutter};
use crate::{App, Error, Mods, Script, Step, Theme, block_on, context};

pub const MAX_SIDE: u32 = 8192;
const DT: f32 = 1.0 / 60.0;
const DRAG_STEPS: u32 = 4;
const SETTLE: usize = 2;
const SETTLE_MOST: usize = 64;
const CLOCK: &str = "pfx-editor-shell/drive-passes";
const RATE: [u32; 2] = [60, 1];

struct Driver<'a> {
    ctx: &'a Context,
    screen: Rect,
    passes: u64,
    modifiers: Modifiers,
    textures: egui::TexturesDelta,
}

impl Driver<'_> {
    fn pass(&mut self, events: Vec<Event>, ui: &mut impl FnMut(&mut Ui)) -> FullOutput {
        let time = self.passes as f64 * f64::from(DT);
        self.pass_at(time, DT, events, ui)
    }

    fn pass_at(
        &mut self,
        time: f64,
        dt: f32,
        events: Vec<Event>,
        ui: &mut impl FnMut(&mut Ui),
    ) -> FullOutput {
        let events: Vec<Event> = std::iter::once(Event::ModifiersChanged(self.modifiers))
            .chain(events)
            .collect();
        let mut input = RawInput {
            screen_rect: Some(self.screen),
            max_texture_side: Some(MAX_SIDE as usize),
            time: Some(time),
            predicted_dt: dt,
            focused: true,
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
        let mut output = self.ctx.run_ui(input, |root| ui(root));
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
    ui: impl FnMut(&mut Ui),
) -> FullOutput {
    let mut output = drive_painted(ctx, size, script, ui, || false);
    output.textures_delta.clear();
    output
}

fn drive_painted(
    ctx: &Context,
    size: [u32; 2],
    script: &Script,
    mut ui: impl FnMut(&mut Ui),
    settling: impl Fn() -> bool,
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
                    phase: TouchPhase::Move,
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
    let mut extra = 0;
    while extra < SETTLE_MOST && settling() {
        last = driver.pass(Vec::new(), &mut ui);
        extra += 1;
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
    shoot(app, size, script, None)
}

pub fn pixels_shuttered(
    app: &mut impl App,
    size: [u32; 2],
    script: &Script,
    shutter: &Shutter,
) -> Result<Vec<u8>, Error> {
    shoot(app, size, script, Some(shutter))
}

fn shoot(
    app: &mut impl App,
    size: [u32; 2],
    script: &Script,
    shutter: Option<&Shutter>,
) -> Result<Vec<u8>, Error> {
    check(size)?;
    let theme = app.theme();
    let gpu = block_on(Gpu::headless()).map_err(Error)?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut painter = Painter::new(&gpu, format, RendererOptions::PREDICTABLE);
    if !app.viewport() {
        return finish(
            (&gpu, &mut painter),
            size,
            (script, shutter),
            &theme,
            |ui, _| app.ui(ui),
            || false,
        );
    }
    app.forget();
    let mut settled = script.clone();
    settled
        .steps
        .extend(std::iter::repeat_n(Step::Frame, SETTLE));
    let settling = std::cell::Cell::new(false);
    finish(
        (&gpu, &mut painter),
        size,
        (&settled, shutter),
        &theme,
        |ui, painter| {
            app.ui(ui);
            app.frame(&gpu, painter.renderer(), ui.ctx().pixels_per_point());
            let commands = app.commands();
            if !commands.is_empty() {
                gpu.queue.submit(commands);
            }
            settling.set(app.settling());
        },
        || settling.get(),
    )
}

fn finish(
    (gpu, painter): (&Gpu, &mut Painter),
    size: [u32; 2],
    (script, shutter): (&Script, Option<&Shutter>),
    app_theme: &Theme,
    mut ui: impl FnMut(&mut Ui, &mut Painter),
    settling: impl Fn() -> bool,
) -> Result<Vec<u8>, Error> {
    let ctx = context(app_theme);
    let mut output = {
        let painter = &mut *painter;
        drive_painted(&ctx, size, script, |root| ui(root, painter), &settling)
    };
    let target = gpu
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba8UnormSrgb)
        .map_err(Error)?;
    let Some(shutter) = shutter else {
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
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
        output.textures_delta.clear();
        return gpu.readback_rgba8(&target).map_err(Error);
    };
    let mut mean = Mean::new(gpu, size)?;
    let mut driver = Driver {
        ctx: &ctx,
        screen: Rect::from_min_size(Pos2::ZERO, Vec2::new(size[0] as f32, size[1] as f32)),
        passes: ctx.data(|data| data.get_temp(Id::new(CLOCK))).unwrap_or(0),
        modifiers: Modifiers::NONE,
        textures: std::mem::take(&mut output.textures_delta),
    };
    let frame = driver.passes;
    let ticks = shutter.ticks();
    let dt = (1.0 / (f64::from(RATE[0]) * f64::from(ticks))) as f32;
    for offset in shutter.offsets() {
        let tick = (frame * u64::from(ticks)).saturating_add_signed(offset);
        let time = shutter::seconds(shutter::tick_time(RATE, ticks, tick));
        let mut last = {
            let painter = &mut *painter;
            let mut each = |root: &mut Ui| ui(root, painter);
            let mut last = driver.pass_at(time, dt, Vec::new(), &mut each);
            let mut extra = 0;
            while extra < SETTLE_MOST && settling() {
                last = driver.pass_at(time, dt, Vec::new(), &mut each);
                extra += 1;
            }
            last
        };
        let primitives = ctx.tessellate(std::mem::take(&mut last.shapes), last.pixels_per_point);
        let screen = ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: last.pixels_per_point,
        };
        let mut textures = std::mem::take(&mut driver.textures);
        painter.paint(
            gpu,
            &target.view,
            &screen,
            &textures,
            &primitives,
            app_theme.bg,
        );
        textures.clear();
        mean.add(gpu, &target.view, 1);
    }
    ctx.data_mut(|data| data.insert_temp(Id::new(CLOCK), frame + 1));
    mean.resolve(gpu)
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
        drive(&ctx, [64, 32], &script, |ui| {
            seen.push(ui.input(|input| input.pointer.primary_down()));
        });
        assert_eq!(seen, vec![false, true, false]);
    }

    #[test]
    fn two_drives_on_one_context_never_move_the_clock_back() {
        let ctx = context(&fixture::theme());
        let script = Script::parse("click 5 5\nframe").unwrap();
        let mut times = Vec::new();
        for _ in 0..2 {
            drive(&ctx, [64, 32], &script, |ui| {
                times.push(ui.input(|input| input.time));
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
        fn ui(&mut self, _: &mut Ui) {}

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

    struct Sweep {
        at: Option<f64>,
        times: Vec<f64>,
    }

    impl App for Sweep {
        fn ui(&mut self, ui: &mut Ui) {
            let time = self.at.unwrap_or_else(|| ui.input(|input| input.time));
            self.times.push(time);
            let x = 10.0 + time as f32 * 1200.0;
            ui.painter().rect_filled(
                Rect::from_min_size(Pos2::new(x, 4.0), Vec2::new(8.0, 8.0)),
                0.0,
                egui::Color32::WHITE,
            );
        }

        fn theme(&self) -> Theme {
            fixture::theme()
        }
    }

    #[test]
    #[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -p pfx-editor-shell -- --ignored"]
    fn a_shuttered_shot_is_the_mean_of_its_subframes_at_their_exact_times() {
        let since = std::time::Instant::now();
        let size = [96, 16];
        let shutter = Shutter::new([1, 2], crate::shutter::Position::Centre, 8).unwrap();
        let mut sweep = Sweep {
            at: None,
            times: Vec::new(),
        };
        let blurred = pixels_shuttered(&mut sweep, size, &Script::default(), &shutter).unwrap();
        assert_eq!(sweep.times.len(), 2 + 8, "{:?}", sweep.times);
        let expected: Vec<f64> = shutter
            .offsets()
            .iter()
            .map(|offset| {
                crate::shutter::seconds(crate::shutter::tick_time(
                    RATE,
                    shutter.ticks(),
                    (2 * 32 + offset) as u64,
                ))
            })
            .collect();
        assert_eq!(&sweep.times[2..], expected.as_slice());
        assert!(expected.iter().all(|time| *time > sweep.times[1]));
        let again = pixels_shuttered(
            &mut Sweep {
                at: None,
                times: Vec::new(),
            },
            size,
            &Script::default(),
            &shutter,
        )
        .unwrap();
        assert_eq!(blurred, again, "a shuttered shot repeats its bytes");
        let apart: Vec<Vec<u8>> = expected
            .iter()
            .map(|time| {
                let mut fixed = Sweep {
                    at: Some(*time),
                    times: Vec::new(),
                };
                pixels(&mut fixed, size, &Script::default()).unwrap()
            })
            .collect();
        let parts: Vec<(&[u8], u32)> = apart.iter().map(|rgba| (rgba.as_slice(), 1)).collect();
        let mean = crate::mean::mean_of(&parts);
        let worst = blurred
            .iter()
            .zip(&mean)
            .map(|(got, want)| (f64::from(*got) - want).abs())
            .fold(0.0, f64::max);
        assert!(worst <= 1.0, "{worst:.3} codes from the mean of the subframes");
        let row: Vec<u8> = blurred[(8 * 96) * 4..(9 * 96) * 4].chunks(4).map(|pixel| pixel[0]).collect();
        let ground = fixture::theme().bg.r();
        let between = row.iter().filter(|value| **value != ground && **value != 255).count();
        assert!(between >= 6, "the moving square smears: {row:?}");
        let plain = pixels(
            &mut Sweep {
                at: None,
                times: Vec::new(),
            },
            size,
            &Script::default(),
        )
        .unwrap();
        assert_ne!(plain, blurred);
        crate::gpu_turn(since);
    }

    #[test]
    #[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -p pfx-editor-shell -- --ignored"]
    fn a_shuttered_shot_of_a_still_window_is_the_unshuttered_shot() {
        let since = std::time::Instant::now();
        let size = [48, 30];
        let shutter = Shutter::new([1, 1], crate::shutter::Position::End, 5).unwrap();
        let still = pixels_shuttered(&mut Blank, size, &Script::default(), &shutter).unwrap();
        let plain = pixels(&mut Blank, size, &Script::default()).unwrap();
        assert!(
            still.iter().zip(&plain).all(|(a, b)| a.abs_diff(*b) <= 1),
            "nothing moves, so the mean is the frame"
        );
        assert!(check([0, 1]).is_err());
        assert!(pixels_shuttered(&mut Blank, [0, 1], &Script::default(), &shutter).is_err());
        crate::gpu_turn(since);
    }
}
