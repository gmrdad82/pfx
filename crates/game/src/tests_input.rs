use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_gpu::screens::Device;
use pfx_input::{
    ActionSpec, Backend, Binding, Button, Control, Family, GlyphStyle, Input, InputEvent, Key,
    LayoutSource, MotorCommand, Motors, PadId, PadInfo, Rumble, glyph_for,
};
use pfx_play::{Cap, Game, Prompt, Settings, Ui, UiFrame, World};

use crate::tests::{WINDOW, capped, world};
use crate::{Config, Event, Headless, Pads, Part};

type Sent = Rc<RefCell<Vec<MotorCommand>>>;
type Queue = Rc<RefCell<VecDeque<InputEvent>>>;

struct Pad {
    events: Queue,
    sent: Sent,
}

impl Backend for Pad {
    fn poll(&mut self, events: &mut Vec<InputEvent>) {
        events.extend(self.events.borrow_mut().drain(..));
    }

    fn set_motors(&mut self, command: &MotorCommand) -> bool {
        self.sent.borrow_mut().push(*command);
        true
    }
}

fn pad(family: Family) -> InputEvent {
    InputEvent::PadConnected {
        pad: PadId(1),
        info: PadInfo {
            name: "stand-in pad".into(),
            vendor: None,
            product: None,
            family,
            motors: Motors::Two,
            resolves_actions: false,
        },
    }
}

fn south(value: f32) -> InputEvent {
    InputEvent::PadButton {
        pad: PadId(1),
        button: Button::South,
        value,
    }
}

struct Rumbler {
    settings: Settings,
    next: Rc<RefCell<Option<Settings>>>,
}

impl Game for Rumbler {
    fn actions(&self) -> Vec<ActionSpec> {
        vec![
            ActionSpec::digital("fire")
                .key(Binding::key(Key::Q))
                .pad(Binding::button(Button::South)),
        ]
    }

    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, world: &mut World, _tick: &Tick, input: &Input) {
        if input.action("fire").pressed {
            world.rumble(Rumble::motors(0.8, 0.4, 250));
        }
    }

    fn frame(&mut self, _world: &World, _alpha: f32) {
        if let Some(settings) = self.next.borrow_mut().take() {
            self.settings = settings;
        }
    }

    fn ui(&mut self, _world: &World, _frame: &UiFrame) -> Ui {
        let mut ui = Ui::new();
        ui.prompt(Prompt::action("fire", [40.0, 40.0], 48.0));
        ui.prompt(Prompt::control(
            Control::Button(Button::East),
            [40.0, 120.0],
            48.0,
        ));
        ui
    }

    fn settings(&self) -> Option<&Settings> {
        Some(&self.settings)
    }
}

fn rumbling(
    layout: Option<Box<dyn LayoutSource>>,
) -> (Headless, Queue, Sent, Rc<RefCell<Option<Settings>>>) {
    let events: Queue = Rc::default();
    let sent: Sent = Rc::default();
    let next = Rc::new(RefCell::new(None));
    let game = Rumbler {
        settings: capped(Cap::Fps60, Cap::Fps30),
        next: next.clone(),
    };
    let mut config = Config::new("rumbler", "0.0.0", world())
        .device(Device::Desktop)
        .pads(Pads::Backend(Box::new(Pad {
            events: events.clone(),
            sent: sent.clone(),
        })));
    if let Some(layout) = layout {
        config = config.layout(layout);
    }
    (
        Headless::new(game, config, WINDOW).unwrap(),
        events,
        sent,
        next,
    )
}

#[test]
fn a_rumble_the_game_queues_reaches_the_pad_and_the_setting_scales_it() {
    let (mut run, events, sent, next) = rumbling(None);
    events.borrow_mut().extend([pad(Family::Xbox), south(1.0)]);
    run.run_for(0.1).unwrap();
    {
        let sent = sent.borrow();
        let first = sent.first().copied().expect("a motor command");
        assert_eq!(first.pad, PadId(1));
        assert!(first.low > first.high && first.high > 0, "{first:?}");
        assert!(first.hold_ms > 0);
    }
    assert!(run.driver.motors_sent() > 0);
    run.run_for(0.5).unwrap();
    assert_eq!(
        sent.borrow()
            .last()
            .map(|command| (command.low, command.high)),
        Some((0, 0))
    );

    let mut quiet = run.driver.settings().clone();
    quiet.rumble = 50;
    *next.borrow_mut() = Some(quiet.clone());
    run.run_for(0.1).unwrap();
    sent.borrow_mut().clear();
    events.borrow_mut().push_back(south(0.0));
    run.run_for(0.1).unwrap();
    sent.borrow_mut().clear();
    events.borrow_mut().push_back(south(1.0));
    run.run_for(0.1).unwrap();
    let half = sent
        .borrow()
        .first()
        .copied()
        .expect("a motor command at half");
    assert!(
        (f32::from(half.low) / 65535.0 - 0.4).abs() < 0.02,
        "{half:?}"
    );

    quiet.rumble = 0;
    *next.borrow_mut() = Some(quiet);
    run.run_for(0.1).unwrap();
    sent.borrow_mut().clear();
    events.borrow_mut().push_back(south(0.0));
    run.run_for(0.1).unwrap();
    sent.borrow_mut().clear();
    events.borrow_mut().push_back(south(1.0));
    run.run_for(0.3).unwrap();
    assert!(
        sent.borrow()
            .iter()
            .all(|command| command.low == 0 && command.high == 0),
        "{:?}",
        sent.borrow()
    );
}

#[test]
fn a_rumble_with_the_keyboard_active_sends_nothing() {
    let (mut run, events, sent, _) = rumbling(None);
    events.borrow_mut().push_back(pad(Family::Xbox));
    run.run_for(0.05).unwrap();
    run.feed(Event::Input(InputEvent::Key {
        key: Key::Q,
        pressed: true,
    }));
    run.run_for(0.2).unwrap();
    assert!(sent.borrow().is_empty(), "{:?}", sent.borrow());
}

struct Azerty {
    id: Rc<Cell<u64>>,
}

impl LayoutSource for Azerty {
    fn layout(&mut self) -> u64 {
        self.id.get()
    }

    fn label(&mut self, key: Key) -> Option<String> {
        match (self.id.get(), key) {
            (1, Key::Q) => Some("a".into()),
            (1, Key::A) => Some("q".into()),
            _ => None,
        }
    }
}

#[test]
fn prompts_follow_the_active_device_and_the_players_layout() {
    let id = Rc::new(Cell::new(0));
    let (mut run, events, _, _) = rumbling(Some(Box::new(Azerty { id: id.clone() })));
    run.frame().unwrap();
    assert_eq!(run.driver.labels().label(Key::Q), "Q");
    assert_eq!(
        run.driver.prompt_parts(),
        &[
            vec![Part::Key(Key::Q)],
            vec![Part::Glyph(glyph_for(
                GlyphStyle::Standard,
                Control::Button(Button::East)
            ))],
        ]
    );
    id.set(1);
    run.frame().unwrap();
    assert_eq!(
        run.driver.labels().label(Key::Q),
        "A",
        "the layout's own label"
    );
    assert_eq!(run.driver.labels().label(Key::A), "Q");

    events
        .borrow_mut()
        .extend([pad(Family::DualSense), south(1.0)]);
    run.run_for(0.1).unwrap();
    let style = GlyphStyle::PlayStation;
    assert_eq!(
        run.driver.prompt_parts(),
        &[
            vec![Part::Glyph(glyph_for(
                style,
                Control::Button(Button::South)
            ))],
            vec![Part::Glyph(glyph_for(style, Control::Button(Button::East)))],
        ]
    );
    run.feed(Event::Input(InputEvent::Key {
        key: Key::Q,
        pressed: true,
    }));
    run.run_for(0.1).unwrap();
    assert_eq!(run.driver.prompt_parts()[0], vec![Part::Key(Key::Q)]);

    run.driver.labels_mut().rename(Key::Q, "Feu");
    assert_eq!(run.driver.labels().label(Key::Q), "FEU");
}

#[cfg(feature = "steam")]
#[test]
fn steam_input_plays_rumble_through_trigger_vibration() {
    use pfx_input::family::STEAM_INPUT_PS5;
    use pfx_steam::{Fake, STEAM_PAD_BASE, SteamPads};

    use crate::steam::SteamInput;

    let mut steam = Fake::new();
    steam.plug_pad(7, STEAM_INPUT_PS5);
    let mut input = SteamInput::new(steam, SteamPads::new("play", &["fire"], &[]));
    let mut events = Vec::new();
    input.poll(&mut events);
    assert!(events.iter().any(|event| matches!(event, InputEvent::PadConnected { pad, .. } if *pad == PadId(STEAM_PAD_BASE))));
    let command = MotorCommand {
        pad: PadId(STEAM_PAD_BASE),
        low: 30_000,
        high: 12_000,
        hold_ms: 40,
    };
    assert!(input.set_motors(&command));
    assert_eq!(input.steam().vibrations(), &[(7, 30_000, 12_000)]);
}
