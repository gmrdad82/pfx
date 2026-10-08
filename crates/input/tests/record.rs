mod common;

use common::*;
use pfx_gpu::window::{LAYOUT_UNITS, RenderScale, Size, viewport};
use pfx_input::{
    ActionState, Axis, Button, Input, InputEvent, Key, MouseButton, Recording, Script,
};

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn unit(&mut self) -> f32 {
        (self.next() % 10_000) as f32 / 10_000.0
    }
}

fn events(rng: &mut Lcg) -> Vec<InputEvent> {
    let keys = [
        Key::A,
        Key::D,
        Key::Space,
        Key::Digit1,
        Key::Up,
        Key::Down,
        Key::Escape,
    ];
    let buttons = [
        Button::South,
        Button::East,
        Button::LeftBumper,
        Button::DPadUp,
        Button::RightTrigger,
    ];
    let axes = [Axis::LeftX, Axis::LeftY, Axis::RightX];
    let mut out = Vec::new();
    for _ in 0..rng.next() % 4 {
        out.push(match rng.next() % 6 {
            0 => key(
                keys[(rng.next() % keys.len() as u64) as usize],
                rng.next().is_multiple_of(2),
            ),
            1 => button(
                0,
                buttons[(rng.next() % buttons.len() as u64) as usize],
                rng.unit(),
            ),
            2 => axis(
                0,
                axes[(rng.next() % axes.len() as u64) as usize],
                rng.unit() * 2.0 - 1.0,
            ),
            3 => InputEvent::PointerMoved {
                window: [rng.unit() * 2560.0, rng.unit() * 1440.0],
            },
            4 => InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: rng.next().is_multiple_of(2),
            },
            _ => InputEvent::Wheel {
                x: 0.0,
                y: rng.unit() - 0.5,
            },
        });
    }
    out
}

type Bits = (bool, bool, bool, bool, u32, u32, u32, u32);

fn bits(states: &[ActionState]) -> Vec<Bits> {
    states
        .iter()
        .map(|s| {
            (
                s.held,
                s.pressed,
                s.released,
                s.repeated,
                s.held_steps,
                s.value.to_bits(),
                s.vector[0].to_bits(),
                s.vector[1].to_bits(),
            )
        })
        .collect()
}

#[test]
fn a_recorded_session_replays_bit_for_bit() {
    let mut live = Input::new(actions(), STEP_US);
    live.set_viewport(viewport(
        Size {
            width: 2560,
            height: 1440,
        },
        RenderScale::Factor(0.75),
        LAYOUT_UNITS,
    ));
    live.feed(connect(0, dualsense()));
    live.feed(key(Key::D, true));
    live.feed(button(0, Button::LeftBumper, 1.0));
    live.feed(InputEvent::PointerMoved {
        window: [640.0, 360.0],
    });
    live.step();
    live.start_recording();
    let mut rng = Lcg(0x5eed);
    let mut trace = Vec::new();
    for _ in 0..600 {
        for event in events(&mut rng) {
            live.feed(event);
        }
        let step = live.step();
        trace.push((
            bits(live.states()),
            live.pointer(),
            live.active(),
            step.changed,
            live.wheel(),
        ));
    }
    let recording = live.stop_recording().unwrap();
    assert_eq!(recording.len(), 600);
    assert!(!live.is_recording());
    let saved = serde_json::to_string(&recording).unwrap();
    let loaded: Recording = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded, recording);

    let mut replay = Input::new(actions(), loaded.step_us);
    replay.begin_replay(&loaded);
    let mut player = loaded.player();
    let mut changes = 0;
    for (index, expected) in trace.iter().enumerate() {
        let step = replay.update(&mut player);
        changes += usize::from(step.changed.is_some());
        let got = (
            bits(replay.states()),
            replay.pointer(),
            replay.active(),
            step.changed,
            replay.wheel(),
        );
        assert_eq!(&got, expected, "step {index}");
    }
    assert!(player.finished());
    assert!(changes > 3);
}

#[test]
fn a_script_drives_input_without_a_device() {
    let mut input = Input::new(actions(), STEP_US);
    let mut script = Script::new();
    script.push(vec![key(Key::Enter, true)]);
    script.push(vec![key(Key::Enter, false)]);
    input.update(&mut script);
    assert!(input.action("confirm").pressed);
    input.update(&mut script);
    assert!(input.action("confirm").released);
    assert_eq!(script.remaining(), 0);
    assert_eq!(input.step_index(), 2);
}
