use std::path::Path;

use pfx_core::clock::Tick;
use pfx_gpu::Gpu;
use pfx_gpu::screens::{Budget, Device};
use pfx_gpu::window::{Attention, Size};
use pfx_input::Input;
use pfx_live::flat::{Draw, Srgba};
use pfx_play::{Cap, Game, Label, Settings, Ui, UiFrame, World};

use crate::tests::{capped, world};
use crate::{Config, Count, Event, Headless};

const FONT: &[u8] = pfx_text::fixture::DM_SANS;

struct Board {
    settings: Settings,
}

impl Game for Board {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        let mut ui = Ui::new();
        ui.draw(Draw::rect([240.0, 180.0], [240.0, 120.0], 12.0).fill(Srgba([0.9, 0.9, 0.9, 1.0])));
        ui.label(
            Label::new("board", "DM Sans", 96.0, [1200.0, 120.0])
                .colour(Srgba([0.05, 0.05, 0.05, 1.0])),
        );
        ui
    }

    fn settings(&self) -> Option<&Settings> {
        Some(&self.settings)
    }
}

fn run(cap: Cap, window: Size, config: impl FnOnce(Config) -> Config) -> Headless {
    let settings = capped(cap, Cap::Fps30);
    let config = config(
        Config::new("board", "0.0.0", world())
            .device(Device::Desktop)
            .font(FONT),
    );
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    Headless::with_gpu(Board { settings }, config, gpu, window).unwrap()
}

fn shot(name: &str, width: u32, height: u32, pixels: &[u8]) {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/game");
    std::fs::create_dir_all(&folder).unwrap();
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    bytes.extend(
        pixels
            .chunks(4)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]]),
    );
    std::fs::write(folder.join(name), bytes).unwrap();
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_loop_draws_the_world_and_its_ui_offscreen_at_each_cap() {
    let window = Size {
        width: 640,
        height: 360,
    };
    for (cap, frames) in [(Cap::Fps30, 15), (Cap::Fps60, 30), (Cap::Fps120, 60)] {
        let mut run = run(cap, window, |config| config);
        run.run_for(0.5).unwrap();
        let counted = run.run_for(0.5).unwrap();
        assert_eq!(counted, Count { frames, ticks: 30 }, "{cap:?}");
        let drawn = run.drawn.unwrap();
        assert_eq!(drawn.render, [640, 360]);
        assert_eq!(drawn.output, [640, 360]);
        assert_eq!(drawn.labels, 1);
        if cap == Cap::Fps120 {
            run.feed(Event::Attention(Attention::Focused(false)));
            run.run_for(0.5).unwrap();
            assert_eq!(run.run_for(0.5).unwrap().frames, 15);
        }
    }
    let mut run = run(Cap::Fps60, window, |config| config);
    run.run_for(0.2).unwrap();
    let pixels = run.readback().unwrap();
    shot("board.ppm", 640, 360, &pixels);
    let card = pixel(&pixels, 640, 80, 60);
    assert!(
        card[..3].iter().all(|&c| (226..=234).contains(&c)),
        "the UI card at native size: {card:?}"
    );
    let world = pixel(&pixels, 640, 320, 300);
    assert!(
        world[..3].iter().any(|&c| c > 8) && world != card,
        "the 3D world behind: {world:?}"
    );
    let ink = (36..76)
        .flat_map(|y| (400..600).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(&pixels, 640, x, y)[0] < 40)
        .count();
    assert!(ink > 200, "the label's ink: {ink}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_wide_window_gets_bars_and_a_lower_render_scale_keeps_the_ui_native() {
    let wide = Size {
        width: 960,
        height: 270,
    };
    let mut run = run(Cap::Fps60, wide, |config| config.render_scale(0.5));
    run.run_for(0.2).unwrap();
    let drawn = run.drawn.unwrap();
    assert_eq!(drawn.render, [240, 135]);
    assert_eq!(drawn.output, [960, 270]);
    let pixels = run.readback().unwrap();
    shot("wide.ppm", 960, 270, &pixels);
    assert_eq!(
        pixel(&pixels, 960, 10, 135),
        [0, 0, 0, 255],
        "the pillarbox bar"
    );
    assert_eq!(
        pixel(&pixels, 960, 950, 135),
        [0, 0, 0, 255],
        "the pillarbox bar"
    );
    let card = pixel(&pixels, 960, 240 + 40, 45);
    assert!(
        card[..3].iter().all(|&c| (226..=234).contains(&c)),
        "{card:?}"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn dynamic_resolution_drops_the_render_size_under_a_tight_budget() {
    let window = Size {
        width: 640,
        height: 360,
    };
    let mut run = run(Cap::Fps60, window, |config| {
        config.budget(Budget::new(0.0005))
    });
    run.run_for(0.5).unwrap();
    let drawn = run.drawn.unwrap();
    assert!(drawn.render[0] < 640 && drawn.render[1] < 360, "{drawn:?}");
    assert_eq!(drawn.output, [640, 360]);
}

struct Azerty;

impl pfx_input::LayoutSource for Azerty {
    fn layout(&mut self) -> u64 {
        1
    }

    fn label(&mut self, key: pfx_input::Key) -> Option<String> {
        (key == pfx_input::Key::Q).then(|| "a".to_string())
    }
}

struct Panel;

const INK: Srgba = Srgba([0.05, 0.05, 0.05, 1.0]);

impl Game for Panel {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        use pfx_input::{Button, Control, GlyphStyle, Key, glyph_for};
        use pfx_live::flat::{Icon, Material, Shape, multiply, scaling, translation};
        use pfx_play::Prompt;

        let mut ui = Ui::new();
        ui.prompt(
            Prompt::control(Control::Key(Key::Q), [120.0, 600.0], 96.0)
                .font("DM Sans", 600)
                .colour(INK),
        );
        ui.prompt(
            Prompt::glyph(
                glyph_for(GlyphStyle::PlayStation, Control::Button(Button::South)),
                [360.0, 600.0],
                96.0,
            )
            .colour(INK),
        );
        ui.draw(
            Draw::new(Shape::Icon(Icon(0)))
                .transform(multiply(translation(600.0, 560.0), scaling(2.0, 2.0)))
                .fill(INK),
        );
        let group = ui.group(0.5);
        ui.draw(
            Draw::rect([1300.0, 250.0], [200.0, 200.0], 0.0)
                .fill(INK)
                .group(group),
        );
        ui.draw(
            Draw::rect([1400.0, 250.0], [200.0, 200.0], 0.0)
                .fill(INK)
                .group(group),
        );
        ui.draw(
            Draw::rect([1500.0, 600.0], [300.0, 200.0], 24.0)
                .fill(Srgba([0.5, 0.5, 0.5, 1.0]))
                .material(Material {
                    thickness: 18.0,
                    bevel: 16.0,
                    gloss: 0.0,
                    roughness: 0.2,
                    reflection: 1.0,
                }),
        );
        ui
    }
}

fn panel(environment: bool) -> Headless {
    let icons = pfx_live::flat::Icons::bake(&[pfx_live::flat::icons::pointer()]).unwrap();
    let mut config = Config::new("panel", "0.0.0", world())
        .device(Device::Desktop)
        .font(FONT)
        .icons(icons)
        .layout(Box::new(Azerty));
    if environment {
        config = config.environment(crate::EnvironmentTexels {
            width: 8,
            height: 4,
            linear: vec![[6.0, 0.0, 0.0, 1.0]; 32],
            intensity: 1.0,
        });
    }
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let window = Size {
        width: 640,
        height: 360,
    };
    let mut run = Headless::with_gpu(Panel, config, gpu, window).unwrap();
    run.run_for(0.2).unwrap();
    run
}

fn inked(pixels: &[u8], x: std::ops::Range<u32>, y: std::ops::Range<u32>) -> usize {
    y.flat_map(|y| x.clone().map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(pixels, 640, x, y)[0] < 60)
        .count()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn prompts_icons_groups_and_the_environment_draw_in_the_ui() {
    let run = panel(true);
    assert_eq!(run.drawn.unwrap().prompts, 2);
    assert_eq!(run.driver.labels().label(pfx_input::Key::Q), "A");
    let pixels = run.readback().unwrap();
    shot("panel.ppm", 640, 360, &pixels);
    let cap = inked(&pixels, 40..72, 200..232);
    let letter = inked(&pixels, 50..62, 206..226);
    assert!(cap > 60, "the keycap: {cap}");
    assert!(
        letter > 8,
        "the keycap's label in the player's layout: {letter}"
    );
    let cross = inked(&pixels, 120..152, 200..232);
    assert!(cross > 150, "the PlayStation glyph: {cross}");
    let pointer = inked(&pixels, 200..226, 187..228);
    assert!(pointer > 200, "the pointer icon: {pointer}");
    let alone = pixel(&pixels, 640, 415, 83);
    let overlap = pixel(&pixels, 640, 450, 83);
    assert!(alone[0] > 60 && alone[0] < 180, "half opacity: {alone:?}");
    for channel in 0..3 {
        assert!(
            alone[channel].abs_diff(overlap[channel]) <= 3,
            "{alone:?} {overlap:?}"
        );
    }
    let red = |pixels: &[u8]| {
        (466..534u32)
            .flat_map(|x| (166..234u32).map(move |y| (x, y)))
            .map(|(x, y)| {
                let [r, g, _, _] = pixel(pixels, 640, x, y);
                f64::from(r) - f64::from(g)
            })
            .sum::<f64>()
    };
    let plain = panel(false).readback().unwrap();
    assert!(
        red(&pixels) > red(&plain) + 500.0,
        "{} {}",
        red(&pixels),
        red(&plain)
    );
}

#[cfg(feature = "dev-hud")]
#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn f3_toggles_the_dev_hud_over_the_game() {
    use pfx_input::{InputEvent, Key};

    let window = Size {
        width: 640,
        height: 360,
    };
    let mut run = run(Cap::Fps60, window, |config| {
        config.exposure(crate::Exposure::Fixed(1.0))
    });
    let f3 = |pressed| {
        Event::Input(InputEvent::Key {
            key: Key::F3,
            pressed,
        })
    };
    run.run_for(0.3).unwrap();
    let before = run.readback().unwrap();
    shot("hud-before.ppm", 640, 360, &before);
    assert!(!run.driver.session().hud().visible());
    run.feed(f3(true));
    run.run_for(0.05).unwrap();
    run.feed(f3(false));
    run.run_for(0.3).unwrap();
    assert!(run.driver.session().hud().visible());
    let [x, y, width, height] = run.driver.session().hud().rect().expect("the HUD's rect");
    assert!(
        x + width <= 640 && x > 320 && y < 40,
        "top right: {x} {y} {width} {height}"
    );
    let shown = run.readback().unwrap();
    shot("hud.ppm", 640, 360, &shown);
    let changed = (x..x + width)
        .flat_map(|px| (y..y + height).map(move |py| (px, py)))
        .filter(|&(px, py)| {
            let a = pixel(&before, 640, px, py);
            let b = pixel(&shown, 640, px, py);
            (0..3).any(|c| a[c].abs_diff(b[c]) > 20)
        })
        .count();
    assert!(
        changed * 2 > (width * height) as usize,
        "{changed} of {}",
        width * height
    );
    let outside = pixel(&shown, 640, 80, 300);
    let was = pixel(&before, 640, 80, 300);
    assert!(
        (0..3).all(|c| outside[c].abs_diff(was[c]) <= 8),
        "{outside:?} {was:?}"
    );
    run.feed(f3(true));
    run.run_for(0.05).unwrap();
    run.feed(f3(false));
    run.run_for(0.3).unwrap();
    assert!(!run.driver.session().hud().visible());
    let hidden = run.readback().unwrap();
    shot("hud-hidden.ppm", 640, 360, &hidden);
    let dark = |pixels: &[u8]| {
        (x..x + width)
            .flat_map(|px| (y..y + height).map(move |py| (px, py)))
            .filter(|&(px, py)| pixel(pixels, 640, px, py)[..3].iter().all(|&c| c < 40))
            .count()
    };
    let area = (width * height) as usize;
    assert!(
        dark(&shown) * 2 > area,
        "the panel: {} of {area}",
        dark(&shown)
    );
    assert!(
        dark(&hidden).abs_diff(dark(&before)) * 200 < area,
        "hidden again: {} against {} before",
        dark(&hidden),
        dark(&before)
    );
}

const STRIPE: f32 = 120.0;
const STRIPES: [Srgba; 2] = [Srgba([0.8, 0.2, 0.2, 1.0]), Srgba([0.2, 0.2, 0.8, 1.0])];

const SPILL: Srgba = Srgba([0.1, 0.9, 0.1, 1.0]);

#[derive(Default)]
struct Seen {
    frames: std::cell::RefCell<Vec<UiFrame>>,
    fired: std::cell::Cell<u32>,
    outside: std::cell::Cell<bool>,
}

struct Backdropped {
    backdrop: bool,
    seen: std::rc::Rc<Seen>,
}

impl Game for Backdropped {
    fn actions(&self) -> Vec<pfx_input::ActionSpec> {
        vec![
            pfx_input::ActionSpec::digital("fire")
                .key(pfx_input::Binding::mouse(pfx_input::MouseButton::Left)),
        ]
    }

    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, input: &Input) {
        if input.action("fire").pressed {
            self.seen.fired.set(self.seen.fired.get() + 1);
        }
        if input.mouse_outside(pfx_input::MouseButton::Left) {
            self.seen.outside.set(true);
        }
    }

    fn ui(&mut self, _world: &World, frame: &UiFrame) -> Ui {
        self.seen.frames.borrow_mut().push(*frame);
        let mut ui = Ui::new();
        if self.backdrop {
            let [width, height] = frame.window_units;
            let count = (width / STRIPE).ceil() as usize;
            for stripe in 0..count {
                let x = stripe as f32 * STRIPE;
                ui.backdrop(
                    Draw::rect([x + STRIPE / 2.0, height / 2.0], [STRIPE, height], 0.0)
                        .fill(STRIPES[stripe % 2]),
                );
            }
        }
        ui.draw(Draw::rect([240.0, 180.0], [240.0, 120.0], 12.0).fill(Srgba([0.9, 0.9, 0.9, 1.0])));
        ui.draw(Draw::rect([960.0, 1000.0], [2920.0, 60.0], 0.0).fill(SPILL));
        ui.label(
            Label::new("framed", "DM Sans", 96.0, [1200.0, 120.0])
                .colour(Srgba([0.05, 0.05, 0.05, 1.0])),
        );
        ui
    }
}

fn backdropped(
    window: Size,
    declared: bool,
    backdrop: bool,
) -> (Headless, Vec<UiFrame>, std::rc::Rc<Seen>) {
    let seen = std::rc::Rc::new(Seen::default());
    let bars = if declared { "backdrop" } else { "#000000" };
    let screen =
        pfx_gpu::screens::ScreenPolicy::from_toml(&format!("[screen]\nbars = \"{bars}\"")).unwrap();
    let config = Config::new("backdrop", "0.0.0", world())
        .device(Device::Desktop)
        .screen(screen)
        .exposure(crate::Exposure::Fixed(1.0))
        .font(FONT);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let game = Backdropped {
        backdrop,
        seen: seen.clone(),
    };
    let mut run = Headless::with_gpu(game, config, gpu, window).unwrap();
    run.painter_mut()
        .unwrap()
        .renderer_mut()
        .wait_sky()
        .unwrap();
    run.run_for(0.2).unwrap();
    let frames = seen.frames.borrow().clone();
    (run, frames, seen)
}

fn mean(pixels: &[u8]) -> f64 {
    let total: u64 = pixels
        .chunks_exact(4)
        .map(|pixel| u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]))
        .sum();
    total as f64 / (pixels.len() / 4 * 3) as f64
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_backdrop_fills_a_wide_window_beyond_an_unchanged_frame() {
    let narrow = Size {
        width: 640,
        height: 360,
    };
    let wide = Size {
        width: 1280,
        height: 360,
    };
    let (framed, _, _) = backdropped(narrow, true, true);
    let framed = framed.readback().unwrap();
    shot("backdrop-narrow.ppm", 640, 360, &framed);
    let (mut run, frames, seen) = backdropped(wide, true, true);
    let pixels = run.readback().unwrap();
    shot("backdrop.ppm", 1280, 360, &pixels);
    let frame = *frames.last().unwrap();
    assert!(frame.backdrop);
    assert!((frame.window_units[0] - 3840.0).abs() < 1e-2, "{frame:?}");
    assert!((frame.window_units[1] - 1080.0).abs() < 1e-2, "{frame:?}");
    assert!((frame.content[0] - 960.0).abs() < 1e-2, "{frame:?}");
    assert_eq!(frame.content[2..], [1920.0, 1080.0]);
    let mut differing = 0;
    for y in 0..360 {
        for x in 0..640 {
            let inside = pixel(&pixels, 1280, x + 320, y);
            let alone = pixel(&framed, 640, x, y);
            if inside != alone {
                differing += 1;
            }
        }
    }
    assert_eq!(differing, 0, "the frame differs from a 16:9 window's");
    let stripe = |x: u32| {
        let units = (x as f32 + 0.5) * 3.0;
        let colour = STRIPES[(units / STRIPE) as usize % 2].0;
        colour.map(|c| (c * 255.0).round() as u8)
    };
    for x in (0..320).chain(960..1280) {
        let units = (x as f32 + 0.5) * 3.0;
        let edge = units % STRIPE;
        if !(3.0..STRIPE - 3.0).contains(&edge) {
            continue;
        }
        for y in [0, 180, 333, 359] {
            let got = pixel(&pixels, 1280, x, y);
            let want = stripe(x);
            assert!(
                (0..3).all(|k| got[k].abs_diff(want[k]) <= 2) && got[3] == 255,
                "the backdrop at {x} {y}: {got:?} against {want:?}"
            );
        }
    }
    let bar = pixel(&pixels, 1280, 640, 333);
    assert!(
        bar[1] > 200 && bar[0] < 80 && bar[2] < 80,
        "the UI bar shows inside the frame: {bar:?}"
    );
    run.feed(Event::Input(pfx_input::InputEvent::PointerMoved {
        window: [100.0, 180.0],
    }));
    run.run_for(0.05).unwrap();
    let pointer = run.driver.session().input().pointer();
    assert!(!pointer.inside, "{pointer:?}");
    assert!(pointer.layout.is_some_and(|[x, _]| x < 0.0), "{pointer:?}");
    run.feed(Event::Input(pfx_input::InputEvent::PointerMoved {
        window: [1200.0, 180.0],
    }));
    run.run_for(0.05).unwrap();
    let pointer = run.driver.session().input().pointer();
    assert!(!pointer.inside, "{pointer:?}");
    assert!(
        pointer.layout.is_some_and(|[x, _]| x > 1920.0),
        "{pointer:?}"
    );
    let left = |pressed: bool| {
        Event::Input(pfx_input::InputEvent::MouseButton {
            button: pfx_input::MouseButton::Left,
            pressed,
        })
    };
    run.feed(left(true));
    run.run_for(0.1).unwrap();
    assert_eq!(
        seen.fired.get(),
        0,
        "a click over the backdrop fires nothing"
    );
    assert!(seen.outside.get(), "the game still sees the press, outside");
    run.feed(left(false));
    run.feed(Event::Input(pfx_input::InputEvent::PointerMoved {
        window: [640.0, 180.0],
    }));
    run.feed(left(true));
    run.run_for(0.1).unwrap();
    assert_eq!(seen.fired.get(), 1, "a click inside the frame fires");
    run.feed(left(false));

    for (declared, backdrop) in [(true, false), (false, true)] {
        let (run, frames, _) = backdropped(wide, declared, backdrop);
        assert_eq!(frames.last().unwrap().backdrop, declared);
        let bars = run.readback().unwrap();
        for x in [10, 300, 970, 1270] {
            for y in [180, 333] {
                assert_eq!(
                    pixel(&bars, 1280, x, y),
                    [0, 0, 0, 255],
                    "declared {declared}, backdrop {backdrop}: the bar at {x} {y}"
                );
            }
        }
        for y in 0..360 {
            for x in 320..960 {
                assert_eq!(
                    pixel(&bars, 1280, x, y),
                    pixel(&pixels, 1280, x, y),
                    "declared {declared}, backdrop {backdrop}: the frame at {x} {y}"
                );
            }
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_fixed_exposure_holds_the_frame_and_auto_is_the_default() {
    use crate::Exposure;
    let window = Size {
        width: 320,
        height: 180,
    };
    let frame = |exposure: Option<Exposure>| {
        let mut run = run(Cap::Fps60, window, |config| match exposure {
            Some(exposure) => config.exposure(exposure),
            None => config,
        });
        run.run_for(1.0).unwrap();
        let pixels = run.readback().unwrap();
        shot(
            &format!("exposure-{exposure:?}.ppm").replace([' ', '{', '}', ':', '(', ')'], ""),
            window.width,
            window.height,
            &pixels,
        );
        mean(&pixels)
    };
    let auto = frame(None);
    let one = frame(Some(Exposure::Fixed(1.0)));
    let again = frame(Some(Exposure::Fixed(1.0)));
    let two = frame(Some(Exposure::Fixed(2.0)));
    let half = frame(Some(Exposure::Fixed(0.5)));
    assert!(
        (one - again).abs() < 0.5,
        "a fixed exposure is steady: {one} and {again}"
    );
    assert!(
        two > one + 5.0 && one > half + 5.0,
        "{half} < {one} < {two}"
    );
    assert!(
        (auto - one).abs() > 1.0,
        "auto meters the frame: {auto} against {one}"
    );
    let error = Headless::with_gpu(
        Board {
            settings: Settings::default(),
        },
        Config::new("board", "0.0.0", world())
            .device(Device::Desktop)
            .font(FONT)
            .exposure(Exposure::Fixed(0.0)),
        pollster::block_on(Gpu::headless()).unwrap(),
        window,
    )
    .err()
    .unwrap();
    assert!(error.contains("exposure"), "{error}");
}
