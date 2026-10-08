use std::cell::RefCell;
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_gpu::screens::{Device, ScreenPolicy, WindowMode};
use pfx_gpu::wgpu;
use pfx_gpu::window::{Host, Point, PresentPreference, Size, choose_present};
use pfx_input::Input;
use pfx_play::{Cap, Exit, FrameTime, Game, GameError, PcmStream, Settings, Ui, UiFrame, World};
use pfx_sound::{ChannelId, Level};

use crate::tests::{WINDOW, capped, rms, world};
use crate::{Config, Event, Headless, WindowCall, present_preference};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Way {
    Quit(u64, u8),
    FailTick(u64),
    FailFrame(u64),
    FailUi(u64),
    WorldFail(u64),
}

#[derive(Default)]
struct Trail {
    ticks: u64,
    frames: Vec<FrameTime>,
    uis: Vec<UiFrame>,
    stopped: bool,
    display: Vec<pfx_gpu::screens::DisplaySettings>,
}

struct Leaver {
    way: Option<Way>,
    settings: Settings,
    keep_display: bool,
    next: Rc<RefCell<Option<Settings>>>,
    trail: Rc<RefCell<Trail>>,
}

type Handles = (Rc<RefCell<Option<Settings>>>, Rc<RefCell<Trail>>);

impl Leaver {
    fn new(way: Option<Way>, settings: Settings) -> (Self, Handles) {
        let next = Rc::new(RefCell::new(None));
        let trail = Rc::new(RefCell::new(Trail::default()));
        (
            Self {
                way,
                settings,
                keep_display: false,
                next: next.clone(),
                trail: trail.clone(),
            },
            (next, trail),
        )
    }
}

impl Game for Leaver {
    fn start(&mut self, _world: &mut World) {}

    fn try_tick(
        &mut self,
        world: &mut World,
        _tick: &Tick,
        _input: &Input,
    ) -> Result<(), GameError> {
        let tick = world.ticks();
        self.trail.borrow_mut().ticks = tick;
        if self.keep_display && *world.display() != self.settings.display {
            self.settings.display = world.display().clone();
            self.trail
                .borrow_mut()
                .display
                .push(world.display().clone());
        }
        if let Some(settings) = self.next.borrow_mut().take() {
            self.settings = settings;
        }
        match self.way {
            Some(Way::Quit(at, code)) if tick == at => world.quit(code),
            Some(Way::FailTick(at)) if tick == at => {
                return Err(format!("the rules broke at tick {at}").into());
            }
            Some(Way::WorldFail(at)) if tick == at => world.fail("the world gave up"),
            _ => {}
        }
        Ok(())
    }

    fn try_frame(&mut self, world: &World, time: FrameTime) -> Result<(), GameError> {
        self.trail.borrow_mut().frames.push(time);
        match self.way {
            Some(Way::FailFrame(at)) if world.ticks() >= at => Err("a frame broke".into()),
            _ => Ok(()),
        }
    }

    fn try_ui(&mut self, world: &World, frame: &UiFrame) -> Result<Ui, GameError> {
        self.trail.borrow_mut().uis.push(*frame);
        match self.way {
            Some(Way::FailUi(at)) if world.ticks() >= at => Err("the menu broke".into()),
            _ => Ok(Ui::default()),
        }
    }

    fn settings(&self) -> Option<&Settings> {
        Some(&self.settings)
    }

    fn stop(&mut self) {
        self.trail.borrow_mut().stopped = true;
    }
}

fn leaving(way: Option<Way>, settings: Settings) -> (Headless, Handles, Rc<RefCell<Vec<String>>>) {
    let (game, handles) = Leaver::new(way, settings);
    let crashes = Rc::new(RefCell::new(Vec::new()));
    let seen = crashes.clone();
    let config = Config::new("leaver", "0.0.0", world())
        .device(Device::Desktop)
        .crash(move |error| seen.borrow_mut().push(error.to_string()));
    (
        Headless::new(game, config, WINDOW).unwrap(),
        handles,
        crashes,
    )
}

#[test]
fn a_quit_request_stops_the_game_cleanly_with_its_code() {
    let (mut run, (_, trail), crashes) =
        leaving(Some(Way::Quit(30, 3)), capped(Cap::Fps60, Cap::Fps30));
    let count = run.run_for(2.0).unwrap();
    assert_eq!(count.ticks, 30, "no tick runs after the quit");
    assert!(run.exited());
    assert_eq!(run.driver.exit(), Some(&Exit::Quit(3)));
    assert_eq!(run.frame().unwrap(), None);
    assert_eq!(run.run_for(1.0).unwrap().ticks, 0);
    let frames = trail.borrow().frames.len();
    assert_eq!(frames, 30, "the quitting frame calls no frame hook");
    assert!(!trail.borrow().stopped);
    assert_eq!(run.finish(), Ok(3));
    assert!(trail.borrow().stopped, "Game::stop ran");
    assert!(crashes.borrow().is_empty());

    let (mut run, (_, trail), _) = leaving(None, capped(Cap::Fps60, Cap::Fps30));
    run.run_for(0.5).unwrap();
    assert_eq!(run.finish(), Ok(0));
    assert!(trail.borrow().stopped);
}

#[test]
fn an_error_from_tick_frame_or_ui_stops_the_game_reports_it_and_exits_non_zero() {
    for (way, message) in [
        (Way::FailTick(12), "the rules broke at tick 12"),
        (Way::FailFrame(12), "a frame broke"),
        (Way::FailUi(12), "the menu broke"),
        (Way::WorldFail(12), "the world gave up"),
    ] {
        let (mut run, (_, trail), crashes) = leaving(Some(way), capped(Cap::Fps60, Cap::Fps30));
        run.run_for(1.0).unwrap();
        assert_eq!(
            run.driver.exit(),
            Some(&Exit::Failed(message.into())),
            "{way:?}"
        );
        assert_eq!(
            trail.borrow().ticks,
            12,
            "{way:?}: nothing ticks after the error"
        );
        assert_eq!(run.driver.exit().unwrap().code(), 1);
        assert_eq!(run.finish(), Err(message.to_string()), "{way:?}");
        assert!(trail.borrow().stopped, "{way:?}: Game::stop ran");
        assert_eq!(*crashes.borrow(), vec![message.to_string()], "{way:?}");
    }
}

#[test]
fn the_frame_and_the_ui_get_the_real_seconds_of_the_frame() {
    for (cap, seconds) in [(Cap::Fps60, 1.0 / 60.0), (Cap::Fps30, 1.0 / 30.0)] {
        let (mut run, (_, trail), _) = leaving(None, capped(cap, Cap::Fps30));
        run.run_for(0.5).unwrap();
        let trail = trail.borrow();
        let frame = *trail.frames.last().unwrap();
        let ui = *trail.uis.last().unwrap();
        assert!((frame.seconds - seconds).abs() < 1e-4, "{cap:?} {frame:?}");
        assert_eq!(ui.seconds, frame.seconds);
        assert_eq!(ui.alpha, frame.alpha);
        assert_eq!(run.driver.seconds(), frame.seconds);
        assert_eq!(
            trail.frames[0].seconds, 0.0,
            "the first frame has no last one"
        );
    }
    let (mut run, (_, trail), _) = leaving(None, capped(Cap::Fps60, Cap::Fps30));
    run.run_for(0.5).unwrap();
    run.feed(Event::Attention(pfx_gpu::window::Attention::Minimized(
        true,
    )));
    assert_eq!(run.frame().unwrap(), None);
    run.clock.now_ns += 3_000_000_000;
    run.feed(Event::Attention(pfx_gpu::window::Attention::Minimized(
        false,
    )));
    run.frame().unwrap();
    assert_eq!(
        trail.borrow().uis.last().unwrap().seconds,
        0.0,
        "a pause is not a long frame"
    );
}

#[test]
fn the_window_title_is_its_own_or_the_name() {
    let config = Config::new("game", "1.0.0", "a.scene.toml");
    assert_eq!(config.window_title(), "game");
    let config = config.title("A Game");
    assert_eq!(config.window_title(), "A Game");
    assert_eq!(config.name, "game");
}

fn windowed() -> Settings {
    let mut settings = capped(Cap::Fps60, Cap::Fps30);
    settings.display.mode = WindowMode::Windowed;
    settings.display.size = Size {
        width: 1280,
        height: 720,
    };
    settings
}

#[test]
fn display_changes_the_driver_learned_reach_the_game_and_store_without_a_reapply() {
    let (mut game, (next, trail)) = Leaver::new(None, windowed());
    game.keep_display = true;
    let config = Config::new("leaver", "0.0.0", world()).device(Device::Desktop);
    let mut run = Headless::new(game, config, WINDOW).unwrap();
    run.run_for(0.1).unwrap();
    assert!(trail.borrow().display.is_empty(), "nothing learned yet");
    run.window.calls.clear();

    let dragged = Size {
        width: 1500,
        height: 800,
    };
    run.feed(Event::Resized(dragged));
    run.feed(Event::Moved {
        monitor: Some("stand-in".into()),
        position: Point { x: 120, y: 80 },
    });
    run.run_for(0.1).unwrap();
    let learned = run.driver.session().world().display().clone();
    assert_eq!(learned.size, dragged);
    assert_eq!(learned.monitor.as_deref(), Some("stand-in"));
    assert_eq!(learned.position, Some(Point { x: 120, y: 80 }));
    assert_eq!(trail.borrow().display, vec![learned.clone()]);
    run.run_for(0.2).unwrap();
    assert_eq!(run.driver.settings().display, learned, "the game stored it");
    assert!(
        run.window.calls.is_empty(),
        "storing what the driver learned re-applies nothing: {:?}",
        run.window.calls
    );

    let mut bigger = run.driver.settings().clone();
    bigger.display.size = Size {
        width: 1024,
        height: 600,
    };
    *next.borrow_mut() = Some(bigger);
    run.run_for(0.1).unwrap();
    assert!(
        run.window.calls.contains(&WindowCall::Size(Size {
            width: 1024,
            height: 600
        })),
        "a real change still applies: {:?}",
        run.window.calls
    );
}

#[test]
fn the_windowed_position_is_restored_on_the_next_start() {
    let mut settings = windowed();
    settings.display.monitor = Some("stand-in".into());
    settings.display.position = Some(Point { x: 200, y: 100 });
    let (game, _) = Leaver::new(None, settings.clone());
    let config = Config::new("leaver", "0.0.0", world()).device(Device::Desktop);
    let run = Headless::new(game, config, WINDOW).unwrap();
    assert!(
        run.window
            .calls
            .contains(&WindowCall::Position(Point { x: 200, y: 100 })),
        "{:?}",
        run.window.calls
    );

    settings.display.monitor = Some("unplugged".into());
    let (game, _) = Leaver::new(None, settings);
    let config = Config::new("leaver", "0.0.0", world()).device(Device::Desktop);
    let run = Headless::new(game, config, WINDOW).unwrap();
    assert!(
        !run.window
            .calls
            .iter()
            .any(|call| matches!(call, WindowCall::Position(_))),
        "{:?}",
        run.window.calls
    );
}

fn presents(calls: &[WindowCall]) -> Vec<PresentPreference> {
    calls
        .iter()
        .filter_map(|call| match call {
            WindowCall::Present(preference) => Some(preference.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn vrr_toggles_live_by_asking_the_surface_for_its_present_mode_on_the_next_frame() {
    let (game, (next, _)) = Leaver::new(None, capped(Cap::Fps120, Cap::Fps30));
    let config = Config::new("leaver", "0.0.0", world()).device(Device::Desktop);
    let mut run = Headless::new(game, config, WINDOW).unwrap();
    run.feed(Event::Refresh(Some(144_000)));
    run.run_for(0.2).unwrap();
    assert!(presents(&run.window.calls).is_empty());

    let mut settings = run.driver.settings().clone();
    settings.pacing.vrr = true;
    *next.borrow_mut() = Some(settings.clone());
    run.run_for(0.05).unwrap();
    assert_eq!(
        presents(&run.window.calls),
        vec![PresentPreference::vrr(Host::Wayland)]
    );
    assert_eq!(run.driver.effective_fps(), Some(120));
    run.run_for(0.2).unwrap();
    assert_eq!(presents(&run.window.calls).len(), 1, "asked once");

    settings.pacing.focused = Cap::Fps60;
    *next.borrow_mut() = Some(settings.clone());
    run.run_for(0.2).unwrap();
    assert_eq!(
        presents(&run.window.calls).len(),
        1,
        "a cap change keeps the mode"
    );

    settings.pacing.vrr = false;
    *next.borrow_mut() = Some(settings);
    run.run_for(0.2).unwrap();
    assert_eq!(
        presents(&run.window.calls),
        vec![
            PresentPreference::vrr(Host::Wayland),
            PresentPreference::game()
        ]
    );

    use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
    let on = present_preference(true, Host::Wayland);
    let mailbox = choose_present(&on, &[Fifo, Mailbox, Immediate]);
    assert_eq!(mailbox.selected, Mailbox);
    assert_eq!(mailbox.desired_maximum_frame_latency, 1);
    let fifo = choose_present(&on, &[Fifo]);
    assert_eq!((fifo.selected, fifo.fell_back), (Fifo, false));
    let off = choose_present(
        &present_preference(false, Host::Wayland),
        &[Fifo, FifoRelaxed],
    );
    assert_eq!(off.selected, FifoRelaxed);
    assert_eq!(off.desired_maximum_frame_latency, 2);
    assert_eq!(
        choose_present(&present_preference(true, Host::X11), &[Fifo, FifoRelaxed]).selected,
        Fifo
    );
}

#[test]
fn a_width_anchored_layout_reaches_the_ui_frame() {
    let screen =
        ScreenPolicy::from_toml("[screen]\ndesktop = [\"16:9\", \"16:10\"]\nlayout_width = 1920")
            .unwrap();
    for (window, layout) in [
        (
            Size {
                width: 1280,
                height: 800,
            },
            [1920.0, 1200.0],
        ),
        (WINDOW, [1920.0, 1080.0]),
    ] {
        let (game, (_, trail)) = Leaver::new(None, capped(Cap::Fps60, Cap::Fps30));
        let config = Config::new("leaver", "0.0.0", world())
            .device(Device::Desktop)
            .screen(screen.clone());
        let mut run = Headless::new(game, config, window).unwrap();
        run.frame().unwrap();
        let frame = *trail.borrow().uis.last().unwrap();
        assert_eq!(frame.layout, layout, "{window:?}");
        assert_eq!(frame.content[2..], layout, "{window:?}");
    }
}

struct Singer {
    stream: Option<PcmStream>,
    phase: u64,
    pushed: Rc<RefCell<Vec<usize>>>,
}

impl Game for Singer {
    fn start(&mut self, world: &mut World) {
        let stream = world
            .sounds
            .stream(ChannelId::MUSIC, 1, 0.1, Level::default());
        assert_eq!(world.audio_due(), 0);
        self.stream = Some(stream);
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        let stream = self.stream.as_ref().unwrap();
        let due = world.audio_due();
        let rate = world.sounds.rate() as f32;
        let samples: Vec<f32> = (0..due)
            .map(|i| {
                let t = (self.phase + i as u64) as f32 / rate;
                (t * std::f32::consts::TAU * 330.0).sin() * 0.5
            })
            .collect();
        self.phase += due as u64;
        self.pushed.borrow_mut().push(stream.push(&samples));
        if world.ticks() == 30 {
            let flood = vec![0.0; stream.capacity() * 4];
            let taken = stream.push(&flood);
            assert!(taken <= stream.capacity());
            assert_eq!(stream.room(), 0);
        }
    }
}

fn sung(music: f32) -> (Vec<f32>, Vec<usize>) {
    let pushed = Rc::new(RefCell::new(Vec::new()));
    let game = Singer {
        stream: None,
        phase: 0,
        pushed: pushed.clone(),
    };
    let mut settings = capped(Cap::Fps60, Cap::Fps30);
    settings.audio.music = music;
    settings.audio.effects = 0.0;
    let config = Config::new("singer", "0.0.0", world())
        .device(Device::Desktop)
        .settings(settings);
    let mut run = Headless::new(game, config, WINDOW).unwrap();
    run.run_for(0.4).unwrap();
    let audio = std::mem::take(&mut run.audio);
    (audio, pushed.borrow().clone())
}

#[test]
fn a_game_streams_its_own_pcm_into_a_mixer_channel_deterministically() {
    let (full, pushed) = sung(1.0);
    assert_eq!(
        pushed[0], 800,
        "one tick of frames at 48 kHz and 60 ticks a second"
    );
    assert!(
        pushed.iter().take(29).all(|&frames| frames == 800),
        "{pushed:?}"
    );
    let (again, _) = sung(1.0);
    assert_eq!(full, again, "two headless runs mix the same samples");
    let window = 4_800..28_800;
    let level = rms(&full[window.clone()]);
    assert!(level > 0.1, "{level}");
    let (half, _) = sung(0.5);
    let quieter = rms(&half[window]);
    assert!((quieter / level - 0.5).abs() < 0.01, "{quieter} {level}");
}
