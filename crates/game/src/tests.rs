use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_gpu::screens::{Bars, Device, WindowMode};
use pfx_gpu::window::{Attention, FullscreenPlan, Size};
use pfx_input::{ActionSpec, Binding, BindingMap, Input, InputEvent, Key};
use pfx_live::flat::{Draw, Srgba};
use pfx_play::{Cap, Game, Label, Settings, Ui, UiFrame, World};
use pfx_sound::Clip;

use crate::{Config, Count, Event, Headless, WindowCall};

pub(crate) const WINDOW: Size = Size {
    width: 1920,
    height: 1080,
};

pub(crate) fn world() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/empty/world/empty.scene.toml")
}

#[derive(Default)]
struct Log {
    alphas: Vec<f32>,
    frames: Vec<UiFrame>,
    jump: Vec<bool>,
}

type Next = Rc<RefCell<Option<Settings>>>;
type Shared = Rc<RefCell<Log>>;

struct Probe {
    settings: Settings,
    next: Next,
    log: Shared,
}

impl Probe {
    fn new(settings: Settings) -> (Self, Next, Shared) {
        let next = Rc::new(RefCell::new(None));
        let log = Rc::new(RefCell::new(Log::default()));
        (
            Self {
                settings,
                next: next.clone(),
                log: log.clone(),
            },
            next,
            log,
        )
    }
}

impl Game for Probe {
    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec::digital("jump").key(Binding::key(Key::Space))]
    }

    fn start(&mut self, world: &mut World) {
        world.play_sound("hum");
    }

    fn tick(&mut self, _world: &mut World, _tick: &Tick, input: &Input) {
        self.log.borrow_mut().jump.push(input.action("jump").held);
    }

    fn frame(&mut self, _world: &World, alpha: f32) {
        if let Some(settings) = self.next.borrow_mut().take() {
            self.settings = settings;
        }
        self.log.borrow_mut().alphas.push(alpha);
    }

    fn ui(&mut self, _world: &World, frame: &UiFrame) -> Ui {
        self.log.borrow_mut().frames.push(*frame);
        let [x, y, _, _] = frame.safe;
        let mut ui = Ui::new();
        ui.draw(
            Draw::rect([x + 60.0, y + 40.0], [80.0, 40.0], 6.0).fill(Srgba([0.9, 0.9, 0.9, 1.0])),
        );
        ui.label(Label::new(
            "probe",
            "DM Sans",
            24.0 * frame.ui_scale,
            [x + 24.0, y + 24.0],
        ));
        ui
    }

    fn settings(&self) -> Option<&Settings> {
        Some(&self.settings)
    }
}

pub(crate) fn capped(focused: Cap, background: Cap) -> Settings {
    let mut settings = Settings::default();
    settings.pacing.focused = focused;
    settings.pacing.background = background;
    settings
}

fn headless(settings: Settings) -> (Headless, Next, Shared) {
    let (game, next, log) = Probe::new(settings);
    let config = Config::new("probe", "0.0.0", world()).device(Device::Desktop);
    (Headless::new(game, config, WINDOW).unwrap(), next, log)
}

#[test]
fn each_cap_paces_the_frames_while_the_tick_stays_at_its_rate() {
    for (cap, fps) in [
        (Cap::Fps30, 30),
        (Cap::Fps60, 60),
        (Cap::Fps90, 90),
        (Cap::Fps120, 120),
    ] {
        let (mut run, _, _) = headless(capped(cap, Cap::Fps30));
        run.run_for(1.0).unwrap();
        let second = run.run_for(1.0).unwrap();
        assert_eq!(second.frames, fps, "{cap:?}");
        assert_eq!(second.ticks, 60, "{cap:?}");
        assert_eq!(run.driver.effective_fps(), Some(fps as u32));
    }
    let (mut run, _, _) = headless(capped(Cap::Uncapped, Cap::Fps30));
    run.frame_cost_ns = 2_000_000;
    run.run_for(1.0).unwrap();
    let second = run.run_for(1.0).unwrap();
    assert_eq!(second.frames, 500);
    assert_eq!(second.ticks, 60);
    assert_eq!(run.driver.effective_fps(), None);
}

#[test]
fn focus_loss_applies_the_background_cap_and_minimizing_pauses_without_a_burst() {
    let (mut run, _, _) = headless(capped(Cap::Fps120, Cap::Fps30));
    run.run_for(1.0).unwrap();
    assert_eq!(run.run_for(1.0).unwrap().frames, 120);
    run.feed(Event::Attention(Attention::Focused(false)));
    assert_eq!(run.run_for(1.0).unwrap().frames, 30);
    let away = run.run_for(1.0).unwrap();
    assert_eq!(
        away,
        Count {
            frames: 30,
            ticks: 60
        }
    );
    run.feed(Event::Attention(Attention::Occluded(true)));
    run.feed(Event::Attention(Attention::Focused(true)));
    assert_eq!(run.run_for(1.0).unwrap().frames, 30);
    run.feed(Event::Attention(Attention::Occluded(false)));
    run.run_for(0.1).unwrap();
    assert_eq!(run.run_for(1.0).unwrap().frames, 120);
    run.feed(Event::Attention(Attention::Minimized(true)));
    assert_eq!(run.frame().unwrap(), None);
    let ticks = run.driver.ticks();
    run.clock.now_ns += 5_000_000_000;
    assert_eq!(run.frame().unwrap(), None);
    assert_eq!(run.driver.ticks(), ticks);
    run.feed(Event::Attention(Attention::Minimized(false)));
    let back = run.frame().unwrap().unwrap();
    assert_eq!(back.ticks, 0, "the first frame back runs no catch-up ticks");
    run.run_for(1.0).unwrap();
    let resumed = run.run_for(1.0).unwrap();
    assert_eq!(
        resumed,
        Count {
            frames: 120,
            ticks: 60
        }
    );
}

#[test]
fn the_interpolation_alpha_reaches_the_game_and_its_ui() {
    for (cap, step) in [
        (Cap::Fps120, 0.5),
        (Cap::Fps90, 60.0 / 90.0),
        (Cap::Fps30, 2.0),
    ] {
        let (mut run, _, log) = headless(capped(cap, Cap::Fps30));
        run.run_for(1.0).unwrap();
        let mut last: Option<f64> = None;
        for _ in 0..24 {
            let tick = run.frame().unwrap().unwrap();
            assert!((0.0..1.0).contains(&tick.alpha), "{cap:?} {}", tick.alpha);
            let log = log.borrow();
            assert_eq!(*log.alphas.last().unwrap(), tick.alpha);
            assert_eq!(log.frames.last().unwrap().alpha, tick.alpha);
            let at = run.driver.ticks() as f64 + f64::from(tick.alpha);
            if let Some(last) = last {
                assert!((at - last - step).abs() < 1e-3, "{cap:?}: {last} then {at}");
            }
            last = Some(at);
        }
    }
}

fn tone() -> Clip {
    Clip {
        rate: 48_000,
        channels: 1,
        samples: (0..4_800)
            .map(|i| (i as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin() * 0.5)
            .collect(),
    }
}

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str, extra: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/game-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = world();
        for entry in std::fs::read_dir(source.parent().unwrap()).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        std::fs::write(root.join("tone.wav"), tone().to_wav()).unwrap();
        let scene = root.join("empty.scene.toml");
        let text = std::fs::read_to_string(&scene).unwrap() + extra;
        std::fs::write(&scene, text).unwrap();
        Self { root }
    }

    fn scene(&self) -> PathBuf {
        self.root.join("empty.scene.toml")
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

#[test]
fn a_volume_change_in_the_settings_changes_the_mix_live() {
    let folder = Folder::new(
        "volume",
        "\n[sound.hum]\nfile = \"tone.wav\"\nloop = true\n",
    );
    let (game, next, _) = Probe::new(capped(Cap::Fps60, Cap::Fps30));
    let config = Config::new("probe", "0.0.0", folder.scene()).device(Device::Desktop);
    let mut run = Headless::new(game, config, WINDOW).unwrap();
    run.run_for(0.5).unwrap();
    run.audio.clear();
    run.run_for(0.5).unwrap();
    let full = rms(&run.audio);
    assert!(full > 0.2, "{full}");

    let mut quieter = run.driver.settings().clone();
    quieter.audio.effects = 0.0;
    *next.borrow_mut() = Some(quieter.clone());
    run.run_for(0.25).unwrap();
    run.audio.clear();
    run.run_for(0.5).unwrap();
    let other = rms(&run.audio);
    assert!((other - full).abs() < full * 0.01, "{other} {full}");

    quieter.audio.music = 0.25;
    *next.borrow_mut() = Some(quieter.clone());
    run.run_for(0.25).unwrap();
    run.audio.clear();
    run.run_for(0.5).unwrap();
    let quarter = rms(&run.audio);
    assert!((quarter / full - 0.25).abs() < 0.01, "{quarter} {full}");
    assert_eq!(run.driver.settings().audio.music, 0.25);

    quieter.audio.muted = true;
    *next.borrow_mut() = Some(quieter);
    run.run_for(0.25).unwrap();
    run.audio.clear();
    run.run_for(0.5).unwrap();
    assert_eq!(rms(&run.audio), 0.0);
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Input(InputEvent::Key { key, pressed })
}

#[test]
fn a_bind_change_in_the_settings_remaps_the_action_live() {
    let (mut run, next, log) = headless(capped(Cap::Fps60, Cap::Fps30));
    run.run_for(0.1).unwrap();
    run.feed(key(Key::Space, true));
    run.run_for(0.1).unwrap();
    assert_eq!(log.borrow().jump.last(), Some(&true));
    run.feed(key(Key::Space, false));
    run.run_for(0.1).unwrap();
    assert_eq!(log.borrow().jump.last(), Some(&false));

    let mut settings = run.driver.settings().clone();
    let mut binds = BindingMap::default();
    binds
        .keyboard
        .insert("jump".into(), vec![Binding::key(Key::J)]);
    settings.binds = Some(binds);
    *next.borrow_mut() = Some(settings.clone());
    run.run_for(0.1).unwrap();
    assert_eq!(run.driver.settings().binds, settings.binds);

    run.feed(key(Key::Space, true));
    run.run_for(0.1).unwrap();
    assert_eq!(
        log.borrow().jump.last(),
        Some(&false),
        "space no longer jumps"
    );
    run.feed(key(Key::Space, false));
    run.feed(key(Key::J, true));
    run.run_for(0.1).unwrap();
    assert_eq!(log.borrow().jump.last(), Some(&true), "J jumps");
    run.feed(key(Key::J, false));

    settings.binds = None;
    *next.borrow_mut() = Some(settings);
    run.run_for(0.1).unwrap();
    run.feed(key(Key::Space, true));
    run.run_for(0.1).unwrap();
    assert_eq!(
        log.borrow().jump.last(),
        Some(&true),
        "the defaults are back"
    );
}

#[test]
fn display_and_pacing_changes_in_the_settings_apply_live() {
    let (mut run, next, _) = headless(capped(Cap::Fps60, Cap::Fps30));
    assert!(
        run.window.calls.iter().any(|call| matches!(
            call,
            WindowCall::Fullscreen(FullscreenPlan::Borderless { .. })
        )),
        "{:?}",
        run.window.calls
    );
    run.window.calls.clear();
    let mut settings = run.driver.settings().clone();
    settings.display.mode = WindowMode::Windowed;
    settings.display.size = Size {
        width: 1280,
        height: 720,
    };
    settings.pacing.focused = Cap::Fps90;
    *next.borrow_mut() = Some(settings);
    run.run_for(0.1).unwrap();
    let calls = &run.window.calls;
    assert!(
        calls.contains(&WindowCall::Fullscreen(FullscreenPlan::Windowed)),
        "{calls:?}"
    );
    assert!(calls.contains(&WindowCall::Decorations(true)), "{calls:?}");
    assert!(
        calls.contains(&WindowCall::Size(Size {
            width: 1280,
            height: 720
        })),
        "{calls:?}"
    );
    assert_eq!(run.driver.effective_fps(), Some(90));
    run.run_for(0.5).unwrap();
    assert_eq!(run.run_for(1.0).unwrap().frames, 90);
}

#[test]
fn the_deck_keeps_its_mode_whatever_the_settings_say() {
    let mut settings = capped(Cap::Fps60, Cap::Fps30);
    settings.display.mode = WindowMode::Windowed;
    let (game, next, _) = Probe::new(settings.clone());
    let config = Config::new("probe", "0.0.0", world()).device(Device::SteamDeck {
        model: pfx_gpu::screens::DeckModel::Oled,
    });
    let mut run = Headless::new(
        game,
        config,
        Size {
            width: 1280,
            height: 800,
        },
    )
    .unwrap();
    assert!(!run.window.calls.contains(&WindowCall::Decorations(true)));
    run.window.calls.clear();
    settings.display.mode = WindowMode::Fullscreen;
    *next.borrow_mut() = Some(settings);
    run.run_for(0.1).unwrap();
    assert!(!run.window.calls.contains(&WindowCall::Decorations(true)));
    assert!(run.driver.ui_frame(0.0).deck);
}

#[test]
fn the_ui_frame_follows_the_aspect_fit_and_the_safe_area() {
    let (mut run, _, log) = headless(capped(Cap::Fps60, Cap::Fps30));
    run.feed(Event::Resized(Size {
        width: 2560,
        height: 1080,
    }));
    run.frame().unwrap();
    let report = *run.driver.report().unwrap();
    assert_eq!(report.bars, Bars::Pillarbox);
    let frame = *log.borrow().frames.last().unwrap();
    assert_eq!(frame.layout, [1920.0, 1080.0]);
    assert_eq!(frame.window, [2560, 1080]);
    assert_eq!(frame.safe, [0.0, 0.0, 1920.0, 1080.0]);
    assert_eq!(frame.ui_scale, 1.0);
    assert!(!frame.deck);
    assert!((frame.window_units[0] - 2560.0).abs() < 1e-3, "{frame:?}");
    assert_eq!(frame.window_units[1], 1080.0);
    assert!((frame.content[0] - 320.0).abs() < 1e-3, "{frame:?}");
    assert_eq!(frame.content[1..], [0.0, 1920.0, 1080.0]);
    assert!(!frame.backdrop);
    let ui = run.driver.ui();
    assert_eq!(ui.draws.len(), 1);
    assert_eq!(ui.labels[0].text, "probe");

    let (game, _, log) = Probe::new(capped(Cap::Fps60, Cap::Fps30));
    let config = Config::new("probe", "0.0.0", world()).device(Device::SteamDeck {
        model: pfx_gpu::screens::DeckModel::Lcd,
    });
    let mut run = Headless::new(
        game,
        config,
        Size {
            width: 1280,
            height: 800,
        },
    )
    .unwrap();
    run.frame().unwrap();
    let frame = *log.borrow().frames.last().unwrap();
    assert_eq!(frame.layout, [1728.0, 1080.0]);
    assert!(
        frame.safe[0] > 30.0 && frame.safe[1] > 30.0,
        "{:?}",
        frame.safe
    );
    assert_eq!(frame.ui_scale, 1.4);
    assert!(frame.deck);

    let (game, _, log) = Probe::new(capped(Cap::Fps60, Cap::Fps30));
    let screen =
        pfx_gpu::screens::ScreenPolicy::from_toml("[screen]\nbars = \"backdrop\"").unwrap();
    let config = Config::new("probe", "0.0.0", world())
        .device(Device::Desktop)
        .screen(screen);
    let mut run = Headless::new(
        game,
        config,
        Size {
            width: 1280,
            height: 1024,
        },
    )
    .unwrap();
    run.frame().unwrap();
    let frame = *log.borrow().frames.last().unwrap();
    assert!(frame.backdrop);
    assert_eq!(frame.window_units[0], 1920.0);
    assert!((frame.window_units[1] - 1536.0).abs() < 1e-3, "{frame:?}");
    assert_eq!(frame.content[0], 0.0);
    assert!((frame.content[1] - 228.0).abs() < 1e-3, "{frame:?}");
}

#[test]
fn the_pointer_lands_in_layout_units_through_the_bars() {
    let (mut run, _, _) = headless(capped(Cap::Fps60, Cap::Fps30));
    run.feed(Event::Resized(Size {
        width: 2560,
        height: 1080,
    }));
    run.feed(Event::Input(InputEvent::PointerMoved {
        window: [1280.0, 540.0],
    }));
    run.run_for(0.05).unwrap();
    let pointer = run.driver.session().input().pointer();
    assert_eq!(pointer.layout, Some([960.0, 540.0]));
    assert!(pointer.inside);
}

#[test]
fn settings_round_trip_through_toml_and_fill_what_is_missing() {
    let mut settings = capped(Cap::Fps120, Cap::Uncapped);
    settings.audio.music = 0.5;
    settings.audio.custom.insert(2, 0.75);
    settings.pacing.vrr = true;
    let text = toml::to_string(&settings).unwrap();
    assert!(text.contains("focused = \"120\""), "{text}");
    assert!(text.contains("background = \"uncapped\""), "{text}");
    let back: Settings = toml::from_str(&text).unwrap();
    assert_eq!(back, settings);
    let partial: Settings = toml::from_str("[audio]\nmusic = 0.5\n").unwrap();
    assert_eq!(partial.audio.music, 0.5);
    assert_eq!(partial.audio.effects, 1.0);
    assert_eq!(partial.pacing, Settings::default().pacing);
    assert_eq!(partial.display, Settings::default().display);
    assert_eq!(partial.binds, None);
    let json = serde_json::to_string(&settings).unwrap();
    assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
}

#[test]
fn a_missing_scene_is_refused_before_anything_opens() {
    let (game, _, _) = Probe::new(Settings::default());
    let config = Config::new("probe", "0.0.0", "no/such.scene.toml");
    let error = Headless::new(game, config, WINDOW).err().unwrap();
    assert!(error.contains("no such scene file"), "{error}");
}

#[test]
fn exposure_is_auto_unless_the_game_fixes_it() {
    use crate::Exposure;
    let config = Config::new("probe", "0.0.0", "a.scene.toml");
    assert_eq!(config.exposure, Exposure::Auto { bias: 0.0 });
    let config = config.exposure(Exposure::Fixed(1.0));
    assert_eq!(config.exposure, Exposure::Fixed(1.0));
}

fn cyclic(root: &std::path::Path, kind: &str) -> std::path::PathBuf {
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root.join("content")).unwrap();
    let write = |relative: &str, text: &str| std::fs::write(root.join(relative), text).unwrap();
    write(
        "project.toml",
        "format = 1\n\n[project]\nname = \"loop\"\nscene = \"content/main.scene.toml\"\n",
    );
    match kind {
        "prefab" => {
            write(
                "content/main.scene.toml",
                "format = 1\n\n[[object]]\nname = \"group\"\nprefab = \"content/loop.prefab.toml\"\n",
            );
            write(
                "content/loop.prefab.toml",
                "format = 1\n\n[[object]]\nname = \"again\"\nprefab = \"content/loop.prefab.toml\"\n",
            );
        }
        "include" => {
            write(
                "content/main.scene.toml",
                "format = 1\n\ninclude = [\"content/other.scene.toml\"]\n",
            );
            write(
                "content/other.scene.toml",
                "format = 1\n\ninclude = [\"content/main.scene.toml\"]\n",
            );
        }
        _ => {
            write(
                "content/main.scene.toml",
                "format = 1\n\n[[object]]\nname = \"group\"\nprefab = \"content/p0.prefab.toml\"\n",
            );
            for at in 0..70 {
                write(
                    &format!("content/p{at}.prefab.toml"),
                    &format!(
                        "format = 1\n\n[[object]]\nname = \"next\"\nprefab = \"content/p{}.prefab.toml\"\n",
                        at + 1
                    ),
                );
            }
            write("content/p70.prefab.toml", "format = 1\n");
        }
    }
    root.join("content/main.scene.toml")
}

const FINDINGS: [(&str, &str); 3] = [
    ("prefab", "which places itself"),
    ("include", "includes itself"),
    ("deep", "nests prefabs more than 64 deep"),
];

#[test]
fn a_game_on_a_cyclic_or_too_deep_project_fails_to_start_with_the_finding() {
    for (kind, says) in FINDINGS {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/game-tests")
            .join(format!("cyclic-{kind}"));
        let scene = cyclic(&root, kind);
        let (game, _, _) = Probe::new(Settings::default());
        let config = Config::new("probe", "0.0.0", scene);
        let error = Headless::new(game, config, WINDOW).err().unwrap();
        assert!(error.contains(says), "{kind}: {error}");
    }
}

mod launcher {
    use std::cell::Cell;
    use std::rc::Rc;

    use std::process::ExitCode;

    #[cfg(feature = "tools")]
    use crate::{CommandError, Unmatched};
    use crate::{Commands, Decision, decide};

    fn done(decision: &Decision, code: u8) -> bool {
        matches!(decision, Decision::Done(exit) if *exit == ExitCode::from(code))
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    fn probe(_args: &[String]) -> Result<(), String> {
        println!("pfx-game-link-probe-6c1d");
        Ok(())
    }

    fn register(commands: &mut Commands) {
        commands.add("probe", "prints the link probe", probe);
    }

    fn decided(list: &[&str], register: impl FnOnce(&mut Commands)) -> (Decision, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let decision = decide(&args(list), "game", "1.2.3", register, &mut out, &mut err);
        (
            decision,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn no_arguments_run_the_game_without_registering() {
        let called = Rc::new(Cell::new(false));
        let seen = called.clone();
        let (decision, out, err) = decided(&[], move |_| seen.set(true));
        assert!(matches!(decision, Decision::Run));
        assert!(!called.get());
        assert!(out.is_empty() && err.is_empty());
    }

    #[cfg(feature = "tools")]
    struct Numbered {
        number: u32,
        stopped: Rc<Cell<u32>>,
    }

    #[cfg(feature = "tools")]
    impl crate::Game for Numbered {
        fn start(&mut self, _world: &mut crate::World) {}

        fn stop(&mut self) {
            self.stopped.set(self.stopped.get() + self.number);
        }
    }

    #[cfg(feature = "tools")]
    #[test]
    fn the_edit_hook_gets_a_factory_that_builds_a_fresh_game_for_each_play() {
        let made = Rc::new(Cell::new(0u32));
        let stopped = Rc::new(Cell::new(0u32));
        let (decision, _, _) = decided(&["edit", "--level", "two"], |commands| {
            commands.edit("edits", |factory, args| {
                if args != ["--level", "two"] {
                    return Err(format!("the arguments: {args:?}"));
                }
                for _ in 0..3 {
                    factory().stop();
                }
                Ok(())
            });
        });
        let Decision::Edit(hook, rest) = decision else {
            panic!("{decision:?}");
        };
        assert_eq!(made.get(), 0, "deciding builds no game");
        let count = made.clone();
        let seen = stopped.clone();
        let factory = crate::factory(move || {
            count.set(count.get() + 1);
            Numbered {
                number: count.get(),
                stopped: seen.clone(),
            }
        });
        let code = crate::launcher::edit("game", hook, factory, &rest);
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(made.get(), 3, "one game for each play");
        assert_eq!(stopped.get(), 1 + 2 + 3, "each play stopped its own game");
    }

    #[cfg(not(feature = "tools"))]
    #[test]
    fn without_tools_every_argument_is_ignored_and_nothing_registers() {
        for list in [
            &["help"][..],
            &["version"],
            &["edit"],
            &["probe", "x"],
            &["nonsense"],
        ] {
            let called = Rc::new(Cell::new(false));
            let seen = called.clone();
            let (decision, out, err) = decided(list, move |_| seen.set(true));
            assert!(matches!(decision, Decision::Run), "{list:?}");
            assert!(!called.get(), "{list:?}");
            assert!(out.is_empty() && err.is_empty());
        }
    }

    #[cfg(feature = "tools")]
    #[test]
    fn help_lists_the_built_ins_and_every_registered_command() {
        let (decision, out, _) = decided(&["help"], |commands| {
            commands.add("run", "runs the rules headless", |_| ()).add(
                "batch",
                "plays a batch of seeds",
                |_| (),
            );
        });
        assert!(done(&decision, 0));
        assert_eq!(
            out,
            "game 1.2.3\n\nUsage: game [COMMAND]\n\nWith no command, game runs the game.\n\nCommands:\n  help     lists these commands\n  version  prints the game's version\n  run      runs the rules headless\n  batch    plays a batch of seeds\n"
        );
        let (_, again, _) = decided(&["--help"], |_| {});
        assert!(again.contains("  help     lists these commands"), "{again}");
        let (_, edit, _) = decided(&["-h"], |commands| {
            commands.edit("opens the game in pfx's editor", |_, _| Ok(()));
        });
        assert!(
            edit.contains("  edit     opens the game in pfx's editor\n"),
            "{edit}"
        );
    }

    #[cfg(feature = "tools")]
    #[test]
    fn version_commands_and_refusals_answer_with_their_exit_codes() {
        let (decision, out, _) = decided(&["version"], |_| {});
        assert!(done(&decision, 0));
        assert_eq!(out, "game 1.2.3\n");
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = seen.clone();
        let (decision, _, _) = decided(&["run", "--seed", "4"], move |commands| {
            commands.add("run", "runs", move |args: &[String]| {
                log.borrow_mut().extend(args.iter().cloned());
            });
        });
        assert!(done(&decision, 0));
        assert_eq!(*seen.borrow(), vec!["--seed".to_string(), "4".to_string()]);
        let (decision, _, err) = decided(&["run"], |commands| {
            commands.add("run", "runs", |_| Err::<(), _>("no seeds"));
        });
        assert!(done(&decision, 1));
        assert_eq!(err, "game run: no seeds\n");
        let hint = "\n\nFor more information, try '--help'.\n";
        let (decision, _, err) = decided(&["nonsense"], |_| {});
        assert!(done(&decision, 2));
        assert_eq!(
            err,
            format!("error: unrecognized subcommand 'nonsense'\n\nUsage: game [COMMAND]{hint}")
        );
        let (decision, _, err) = decided(&["--x"], |_| {});
        assert!(done(&decision, 2));
        assert_eq!(
            err,
            format!("error: unexpected argument '--x' found\n\nUsage: game [COMMAND]{hint}")
        );
        let (decision, _, err) = decided(&["edit"], |_| {});
        assert!(done(&decision, 2));
        assert!(err.contains("unrecognized subcommand 'edit'"), "{err}");
        let (decision, _, err) = decided(&["edit", "x"], |commands| {
            commands
                .edit("edits", |_, _| Ok(()))
                .usage("edit", "")
                .add("scan", "scans", |_| ExitCode::SUCCESS)
                .usage("scan", "<SEED>");
        });
        assert!(done(&decision, 2));
        assert_eq!(
            err,
            format!("error: unexpected argument 'x' found\n\nUsage: game edit{hint}")
        );
        let (decision, _, err) = decided(&["help"], |commands| {
            commands.usage("nope", "");
        });
        assert!(done(&decision, 2));
        assert!(err.contains("usage \"nope\": no such command"), "{err}");
        for name in ["help", "edit", "-x", "two words", ""] {
            let (decision, _, err) = decided(&["help"], |commands| {
                commands.add(name, "clashes", |_| ());
            });
            assert!(done(&decision, 2), "{name:?}");
            assert!(!err.is_empty());
        }
        let (decision, _, err) = decided(&["help"], |commands| {
            commands.add("run", "a", |_| ()).add("run", "b", |_| ());
        });
        assert!(done(&decision, 2));
        assert!(err.contains("registered twice"), "{err}");
        let (decision, _, _) = decided(&["edit", "level.scene.toml"], |commands| {
            commands.edit("edits", |_, _| Ok(()));
        });
        match decision {
            Decision::Edit(_, rest) => assert_eq!(rest, vec!["level.scene.toml".to_string()]),
            other => panic!("{other:?}"),
        }
    }

    #[cfg(feature = "tools")]
    #[test]
    fn a_command_returns_its_own_exit_code_and_usage_errors_are_two() {
        let (decision, _, err) = decided(&["scan"], |commands| {
            commands.add("scan", "scans", |_| ExitCode::from(3));
        });
        assert!(done(&decision, 3) && err.is_empty());
        let (decision, _, _) = decided(&["scan"], |commands| {
            commands.add("scan", "scans", |_| 4u8);
        });
        assert!(done(&decision, 4));
        let (decision, _, _) = decided(&["scan"], |commands| {
            commands.add("scan", "scans", |_| {
                Ok::<_, CommandError>(ExitCode::from(5))
            });
        });
        assert!(done(&decision, 5));
        let (decision, _, err) = decided(&["scan"], |commands| {
            commands.add("scan", "scans", |args: &[String]| {
                if args.is_empty() {
                    return Err(CommandError::usage("scan needs a seed"));
                }
                Ok(ExitCode::SUCCESS)
            });
        });
        assert!(done(&decision, 2));
        assert_eq!(
            err,
            "error: scan needs a seed\n\nUsage: game scan [ARGS]...\n\nFor more information, try '--help'.\n"
        );
        let (_, _, err) = decided(&["scan"], |commands| {
            commands
                .add("scan", "scans", |_| {
                    Err::<(), _>(CommandError::usage("unexpected argument 'x' found"))
                })
                .usage("scan", "<SEED>");
        });
        assert!(err.contains("\n\nUsage: game scan <SEED>\n\n"), "{err}");
        let (decision, _, err) = decided(&["scan"], |commands| {
            commands.add("scan", "scans", |_| {
                std::fs::read("no/such/file").map(|_| ExitCode::SUCCESS)
            });
        });
        assert!(done(&decision, 1));
        assert!(err.starts_with("game scan: "), "{err}");
        let (decision, _, err) = decided(&["scan"], |commands| {
            commands.add("scan", "scans", |_| {
                Err::<ExitCode, _>(CommandError::new(7, "seven"))
            });
        });
        assert!(done(&decision, 7));
        assert_eq!(err, "game scan: seven\n");
    }

    #[cfg(feature = "tools")]
    #[test]
    fn arguments_that_are_not_a_command_reach_the_unmatched_hook() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let hook = |seen: Rc<std::cell::RefCell<Vec<Vec<String>>>>| {
            move |commands: &mut Commands| {
                commands
                    .add("scan", "scans", |_| ExitCode::from(6))
                    .unmatched(move |args| {
                        seen.borrow_mut().push(args.to_vec());
                        match args.first().map(String::as_str) {
                            Some("--windowed") => Ok(Unmatched::Run),
                            Some("--quiet") => Ok(Unmatched::Command(args[1..].to_vec())),
                            Some("--json") => Ok(Unmatched::Exit(ExitCode::from(4))),
                            _ => Err(CommandError::usage(format!(
                                "{{\"error\":\"unknown {}\"}}",
                                args[0]
                            ))),
                        }
                    });
            }
        };
        let (decision, _, _) = decided(&["--windowed"], hook(seen.clone()));
        assert!(matches!(decision, Decision::Run));
        let (decision, out, _) = decided(&["--quiet", "version"], hook(seen.clone()));
        assert!(done(&decision, 0));
        assert_eq!(out, "game 1.2.3\n");
        let (decision, _, _) = decided(&["--quiet", "scan"], hook(seen.clone()));
        assert!(done(&decision, 6));
        let (decision, _, _) = decided(&["--quiet"], hook(seen.clone()));
        assert!(matches!(decision, Decision::Run));
        let (decision, _, _) = decided(&["--json", "x"], hook(seen.clone()));
        assert!(done(&decision, 4));
        let (decision, _, err) = decided(&["nonsense"], hook(seen.clone()));
        assert!(done(&decision, 2));
        assert_eq!(err, "game: {\"error\":\"unknown nonsense\"}\n");
        let (decision, _, err) = decided(&["--quiet", "--quiet"], hook(seen.clone()));
        assert!(done(&decision, 2));
        assert!(
            err.contains("unexpected argument '--quiet' found"),
            "the hook runs once: {err}"
        );
        let before = seen.borrow().len();
        let (decision, _, _) = decided(&["scan"], hook(seen.clone()));
        assert!(done(&decision, 6));
        let (decision, _, _) = decided(&["help"], hook(seen.clone()));
        assert!(done(&decision, 0));
        assert_eq!(
            seen.borrow().len(),
            before,
            "known commands never reach the hook"
        );
        assert_eq!(seen.borrow()[0], vec!["--windowed".to_string()]);
    }

    #[cfg(not(windows))]
    #[test]
    fn command_code_links_only_with_the_tools_feature() {
        let (decision, _, _) = decided(&["probe"], register);
        let binary = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let found = |reversed: &str| {
            let needle: String = reversed.chars().rev().collect();
            binary
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
        };
        let code = found("eborp5rehcnual8");
        let text = found("d1c6-eborp-knil-emag-xfp");
        let reversed =
            "ni deknil era rotide s'xfp dna sdnammoc s'rehcnual eht :dliub sloot emag-xfp";
        #[cfg(feature = "tools")]
        assert_eq!(
            reversed.chars().rev().collect::<String>(),
            crate::launcher::TOOLS_MARKER
        );
        let marker = found(reversed);
        let linked = code || text;
        if cfg!(feature = "tools") {
            assert!(done(&decision, 0));
            assert!(code && text, "the tools build carries the command");
            assert!(
                marker,
                "the tools build carries the marker steam stage refuses"
            );
        } else {
            assert!(matches!(decision, Decision::Run));
            assert!(
                !linked,
                "the command's code links into a build without tools"
            );
            assert!(!marker, "the marker links into a build without tools");
        }
    }
}

#[cfg(feature = "steam")]
#[test]
fn steam_input_drives_the_actions_and_hands_its_events_back() {
    use pfx_input::family::STEAM_INPUT_PS5;
    use pfx_steam::{Fake, SteamPads};

    use crate::Pads;
    use crate::steam::SteamInput;

    let mut steam = Fake::new();
    steam.plug_pad(7, STEAM_INPUT_PS5);
    steam.press(7, "jump", true);
    steam.push_overlay(true);
    let input = SteamInput::new(steam, SteamPads::new("play", &["jump"], &[]));
    let events = input.events();
    let (game, _, log) = Probe::new(capped(Cap::Fps60, Cap::Fps30));
    let config = Config::new("probe", "0.0.0", world())
        .device(Device::Desktop)
        .pads(Pads::Backend(Box::new(input)));
    let mut run = Headless::new(game, config, WINDOW).unwrap();
    run.run_for(0.1).unwrap();
    assert_eq!(log.borrow().jump.last(), Some(&true));
    assert_eq!(
        run.driver.session().input().glyph_style(),
        pfx_input::GlyphStyle::PlayStation
    );
    assert_eq!(
        events.take(),
        vec![pfx_steam::Event::Overlay(pfx_steam::Overlay::Opened)]
    );
    assert!(events.take().is_empty());
}

#[derive(Default)]
struct Noisy {
    ticks: u64,
    said: Vec<pfx_play::Notice>,
}

impl pfx_play::Reload for Noisy {
    fn files(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn changed(&mut self, _file: &Path) {}

    fn notices(&mut self) -> Vec<pfx_play::Notice> {
        std::mem::take(&mut self.said)
    }
}

impl Game for Noisy {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {
        self.ticks += 1;
        if self.ticks == 3 {
            self.said.push(pfx_play::Notice {
                level: pfx_play::Level::Error,
                text: "module `brain` failed".to_string(),
            });
        }
    }

    fn modules(&mut self) -> Option<&mut dyn pfx_play::Reload> {
        Some(self)
    }
}

#[test]
fn the_runtime_collects_the_notices_of_a_games_modules_and_keeps_running() {
    let config = Config::new("probe", "0.0.0", world()).device(Device::Desktop);
    let mut run = Headless::new(Noisy::default(), config, WINDOW).unwrap();
    let ran = run.run_for(0.5).unwrap();
    let notices = run.driver.take_notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].level, pfx_play::Level::Error);
    assert!(run.driver.take_notices().is_empty());
    assert!(ran.ticks >= 20, "{ran:?}");
}
