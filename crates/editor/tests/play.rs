use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_editor::editor::Play;
use pfx_editor::script::frames;
use pfx_editor::session::{Session, Setup};
use pfx_editor_shell::{App, Script, Theme, Watch, shot};
use pfx_game::flat::{Draw, FlatScene, Light, ShadowCurve, Srgba};
use pfx_game::{Anchor, Config, Device, Label, Renderer};
use pfx_gpu::Gpu;
use pfx_gpu::wgpu;
use pfx_gpu::window::Rect;
use pfx_input::Input;
use pfx_live::upscale::{Overlay, Upscale};
use pfx_play::{Game, GameError, PaintFrame, Ui, UiFrame, Warm, Warming, World};

const SIZE: [u32; 2] = [1280, 800];
const FONT: &[u8] = pfx_text::fixture::DM_SANS;
const SHEET: Srgba = Srgba([0.2, 0.2, 0.2, 1.0]);
const CARD: Srgba = Srgba([0.9, 0.9, 0.9, 1.0]);
const PAINTED: Srgba = Srgba([0.8, 0.1, 0.1, 1.0]);
const INK: Srgba = Srgba([0.02, 0.02, 0.02, 1.0]);

#[derive(Default)]
struct Seen {
    made: Cell<u32>,
    started: Cell<u32>,
    stopped: Cell<u32>,
    warms: Cell<u32>,
    uis: Cell<u32>,
    paints: Cell<u32>,
    early: Cell<bool>,
    layout: Cell<[f32; 2]>,
}

#[derive(Clone, Copy, Default)]
struct Faults {
    fail_at: Option<u64>,
    paint: bool,
}

struct Board {
    seen: Rc<Seen>,
    faults: Faults,
    overlay: Option<Upscale>,
}

impl Game for Board {
    fn start(&mut self, _world: &mut World) {
        self.seen.started.set(self.seen.started.get() + 1);
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        if self.faults.fail_at.is_some_and(|at| world.ticks() >= at) {
            world.fail("the board broke");
        }
    }

    fn warm(&mut self, warm: &mut Warm<'_>) -> Result<Warming, GameError> {
        self.seen.warms.set(self.seen.warms.get() + 1);
        warm.glyphs("DM Sans", 400, &[160.0], "PLAY")?;
        Ok(Warming::Done)
    }

    fn ui(&mut self, _world: &World, frame: &UiFrame) -> Ui {
        if self.seen.warms.get() < self.seen.started.get() {
            self.seen.early.set(true);
        }
        self.seen.uis.set(self.seen.uis.get() + 1);
        self.seen.layout.set(frame.layout);
        let mut ui = Ui::new();
        ui.clear(SHEET);
        ui.draw(Draw::rect([1260.0, 540.0], [900.0, 600.0], 0.0).fill(CARD));
        ui.label(
            Label::new("PLAY", "DM Sans", 160.0, [1260.0, 400.0])
                .anchor(Anchor::Center)
                .colour(INK),
        );
        ui
    }

    fn paint(
        &mut self,
        renderer: &mut Renderer,
        target: &wgpu::TextureView,
        frame: &PaintFrame,
    ) -> Result<(), GameError> {
        self.seen.paints.set(self.seen.paints.get() + 1);
        if self.faults.paint {
            return Err("the paint failed".into());
        }
        let gpu = renderer.gpu().clone();
        let overlay = self
            .overlay
            .get_or_insert_with(|| Upscale::new(&gpu.device));
        let draws = [Draw::rect([400.0, 540.0], [400.0, 400.0], 0.0).fill(PAINTED)];
        let none = ShadowCurve::none();
        let scene = FlatScene {
            layout: frame.layout,
            clear: None,
            curve: &none,
            light: Light::default(),
            draws: &draws,
            groups: &[],
            text: &[],
            icons: None,
            sprites: None,
            environment: None,
            post: false,
            frame: frame.frame as u32,
            seed: frame.seed,
        };
        let [x, y, width, height] = frame.content;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("the board's own painter"),
            });
        overlay.encode_overlay(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &Overlay {
                scene: &scene,
                look: None,
                output: target,
                output_format: frame.format,
                output_size: frame.output,
                content: Rect {
                    x,
                    y,
                    width,
                    height,
                },
            },
            None,
        )?;
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn stop(&mut self) {
        self.seen.stopped.set(self.seen.stopped.get() + 1);
    }
}

fn flat_project(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("editor-play")
        .join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("project.toml"),
        "format = 1\n\n[project]\nname = \"board\"\n\n[authoring.pfx]\nflat = true\n",
    )
    .unwrap();
    root
}

fn setup() -> Setup {
    Setup {
        pgpu: false,
        audio: false,
    }
}

fn config() -> Config {
    Config::flat("board", "0.0.0")
        .device(Device::Desktop)
        .font(FONT)
}

fn board(root: &Path, faults: Faults) -> (Session, Rc<Seen>) {
    let seen = Rc::new(Seen::default());
    let made = seen.clone();
    let factory = pfx_editor::factory(move || {
        made.made.set(made.made.get() + 1);
        Board {
            seen: made.clone(),
            faults,
            overlay: None,
        }
    });
    let session = Session::open_game(root, setup(), factory, config()).unwrap();
    (session, seen)
}

fn save(name: &str, png: &[u8]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/editor-shots");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{name}.png")), png).unwrap();
}

fn take(session: &mut Session, name: &str, script: &str) {
    let png = shot(session, SIZE, &Script::parse(script).unwrap()).unwrap();
    save(name, &png);
}

fn logged(session: &Session, wanted: &str) -> bool {
    session
        .editor
        .log
        .entries
        .iter()
        .any(|entry| entry.text.contains(wanted))
}

fn near(pixel: &[u8], colour: Srgba, slack: u8) -> bool {
    (0..3).all(|i| pixel[i].abs_diff((colour.0[i] * 255.0).round() as u8) <= slack)
}

#[derive(Debug)]
struct Counted {
    size: [u32; 2],
    painted: usize,
    card: usize,
    ink: usize,
    sheet: usize,
    corner: [u8; 3],
}

fn counted(session: &Session) -> Counted {
    let size = session.view.as_ref().unwrap().size();
    let pixels = session.pixels().unwrap();
    let width = size[0] as usize;
    let at = |x: usize, y: usize| &pixels[(y * width + x) * 4..(y * width + x) * 4 + 4];
    let mut box_min = [usize::MAX; 2];
    let mut box_max = [0usize; 2];
    let mut out = Counted {
        size,
        painted: 0,
        card: 0,
        ink: 0,
        sheet: 0,
        corner: [pixels[0], pixels[1], pixels[2]],
    };
    for y in 0..size[1] as usize {
        for x in 0..width {
            let pixel = at(x, y);
            if near(pixel, PAINTED, 8) {
                out.painted += 1;
            }
            if near(pixel, SHEET, 6) {
                out.sheet += 1;
            }
            if near(pixel, CARD, 6) {
                out.card += 1;
                box_min = [box_min[0].min(x), box_min[1].min(y)];
                box_max = [box_max[0].max(x), box_max[1].max(y)];
            }
        }
    }
    if out.card > 0 {
        for y in box_min[1]..=box_max[1] {
            for x in box_min[0]..=box_max[0] {
                if at(x, y)[..3].iter().all(|&channel| channel < 60) {
                    out.ink += 1;
                }
            }
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_flat_game_plays_its_ui_and_its_own_paint_in_the_play_view_one_game_per_play() {
    let root = flat_project("board");
    let (mut session, seen) = board(&root, Faults::default());
    assert!(session.flat() && session.plays_a_game());
    let script = format!(
        "{}key F5\n{}key F8\nframe\nkey F5\n{}",
        frames(10),
        frames(30),
        frames(30)
    );
    take(&mut session, "play-flat-game", &script);
    assert_eq!(session.editor.play, Play::Playing);
    assert_eq!(seen.made.get(), 2, "one game for each play");
    assert_eq!(seen.started.get(), 2);
    assert_eq!(seen.stopped.get(), 1, "F8 stopped the first game");
    assert_eq!(
        seen.warms.get(),
        2,
        "each play warms once before its frames"
    );
    assert!(!seen.early.get(), "no UI before the warm-up");
    assert!(seen.uis.get() > 40, "{} UI frames", seen.uis.get());
    assert!(seen.paints.get() > 40, "{} paints", seen.paints.get());
    let [width, height] = seen.layout.get();
    assert!(
        height == 1080.0 && (width / 1920.0 - 1.0).abs() < 0.05,
        "laid out for the 16:9 frame: {width} × {height}"
    );
    let shown = counted(&session);
    assert!(
        shown.painted > 2_000,
        "the game's own paint shows: {shown:?}"
    );
    assert!(shown.card > 10_000, "the UI's card shows: {shown:?}");
    assert!(
        shown.ink > 1_000,
        "the label's ink shows on the card: {shown:?}"
    );
    assert!(shown.sheet > 10_000, "the clear colour shows: {shown:?}");
    let aspect = shown.size[0] as f32 / shown.size[1] as f32;
    if (aspect / (16.0 / 9.0) - 1.0).abs() > 0.02 {
        assert_eq!(shown.corner, [0, 0, 0], "the bars around the 16:9 frame");
    }
    assert!(logged(
        &session,
        "play: the game's own logic runs on an empty world"
    ));
    drop(session);
    assert_eq!(seen.stopped.get(), 2, "closing the editor stops the game");
    assert_eq!(
        fs::read_to_string(root.join("tmp/pfx/flat.scene.toml")).unwrap(),
        "format = 1\n",
        "play writes no file"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_flat_project_opens_on_an_empty_world_and_f5_plays_it() {
    let root = flat_project("empty");
    let mut session = Session::open_project(&root, setup(), None).unwrap();
    assert!(session.flat() && !session.plays_a_game());
    take(
        &mut session,
        "play-flat-project",
        &format!("{}key F5\n{}", frames(10), frames(20)),
    );
    assert_eq!(session.editor.play, Play::Playing);
    assert!(logged(&session, "play: the empty world runs as a game"));
    let pixels = session.pixels().unwrap();
    assert!(
        pixels.chunks_exact(4).all(|pixel| pixel[..3] == [0, 0, 0]),
        "a game with no UI shows the black clear colour"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_games_fatal_error_stops_play_and_reaches_the_log() {
    let root = flat_project("fail");
    let (mut session, seen) = board(
        &root,
        Faults {
            fail_at: Some(5),
            paint: false,
        },
    );
    take(
        &mut session,
        "play-failed",
        &format!("{}key F5\n{}", frames(10), frames(30)),
    );
    assert_eq!(session.editor.play, Play::Stopped);
    assert!(
        logged(
            &session,
            "play: the game failed: the board broke; play stopped"
        ),
        "{:#?}",
        session.editor.log.entries
    );
    assert_eq!(seen.started.get(), 1);
    assert_eq!(seen.stopped.get(), 1, "the failed game stopped once");
    assert!(session.pixels().is_ok(), "the editor stays open");

    let (mut painted, seen) = board(
        &root,
        Faults {
            fail_at: None,
            paint: true,
        },
    );
    take(
        &mut painted,
        "play-paint-failed",
        &format!("{}key F5\n{}", frames(10), frames(10)),
    );
    assert_eq!(painted.editor.play, Play::Stopped);
    assert!(
        logged(&painted, "play: the paint failed; play stopped"),
        "{:#?}",
        painted.editor.log.entries
    );
    assert_eq!(seen.paints.get(), 1);
    assert_eq!(seen.stopped.get(), 1);
    let _ = fs::remove_dir_all(&root);
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

struct Caps;

impl Game for Caps {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        use pfx_input::{Control, Key};
        let mut ui = Ui::new();
        ui.clear(CARD);
        ui.prompt(
            pfx_play::Prompt::control(Control::Key(Key::Q), [800.0, 400.0], 300.0)
                .font("DM Sans", 600)
                .colour(INK),
        );
        ui
    }
}

fn caps(root: &Path, layout: Option<Box<dyn pfx_input::LayoutSource>>) -> Session {
    let factory = pfx_editor::factory(|| Caps);
    let mut config = config();
    if let Some(layout) = layout {
        config = config.layout(layout);
    }
    Session::open_game(root, setup(), factory, config).unwrap()
}

struct Typing<'a> {
    session: &'a mut Session,
    passes: u32,
}

impl App for Typing<'_> {
    fn ui(&mut self, ctx: &egui::Context) {
        self.passes += 1;
        if self.passes >= 20 {
            ctx.input_mut(|input| {
                input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: Some(egui::Key::Q),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
                input.events.push(egui::Event::Text("a".into()));
            });
        }
        self.session.ui(ctx);
    }

    fn viewport(&self) -> bool {
        self.session.viewport()
    }

    fn theme(&self) -> Theme {
        self.session.theme()
    }

    fn frame(&mut self, gpu: &Gpu, renderer: &mut egui_wgpu::Renderer, ppp: f32) -> bool {
        self.session.frame(gpu, renderer, ppp)
    }

    fn forget(&mut self) {
        self.session.forget();
    }

    fn name(&self) -> &str {
        self.session.name()
    }

    fn label(&self) -> Option<String> {
        self.session.label()
    }

    fn watched(&self) -> Watch {
        self.session.watched()
    }

    fn changed(&mut self, files: &[PathBuf]) {
        self.session.changed(files);
    }
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 24))
        .count()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_prompt_in_the_play_view_shows_the_players_layout_configured_or_learned() {
    let root = flat_project("caps");
    let play = format!("{}key F5\n{}", frames(10), frames(30));

    let mut plain = caps(&root, None);
    take(&mut plain, "play-keycap-us", &play);
    let us = plain.pixels().unwrap();

    let mut configured = caps(&root, Some(Box::new(Azerty)));
    take(&mut configured, "play-keycap-azerty", &play);
    let azerty = configured.pixels().unwrap();
    let configured_diff = differing(&us, &azerty);
    assert!(
        configured_diff > 1_000,
        "the configured layout changes the cap's label: {configured_diff} pixels"
    );

    let mut taught = caps(&root, None);
    let mut typing = Typing {
        session: &mut taught,
        passes: 0,
    };
    let png = shot(&mut typing, SIZE, &Script::parse(&play).unwrap()).unwrap();
    save("play-keycap-learned", &png);
    let learned = taught.pixels().unwrap();
    assert!(
        differing(&us, &learned) > 1_000,
        "the learned label changes the cap"
    );
    assert_eq!(
        differing(&azerty, &learned),
        0,
        "a learned A draws as a configured A"
    );
    let _ = fs::remove_dir_all(&root);
}
