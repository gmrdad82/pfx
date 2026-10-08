mod common;

use common::*;
use pfx_gpu::screens::{Aspect, DeckModel, Device, ScreenPolicy};
use pfx_gpu::window::Size;
use pfx_input::{Input, InputEvent};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn policy() -> ScreenPolicy {
    ScreenPolicy {
        deck: vec![Aspect::DECK],
        desktop: vec![Aspect::WIDE],
        ..ScreenPolicy::default()
    }
}

#[test]
fn the_pointer_maps_through_the_screen_fit_and_bars_are_outside() {
    let report = policy()
        .fit(
            Device::Desktop,
            Size {
                width: 5120,
                height: 1440,
            },
        )
        .unwrap();
    let mut input = Input::new(actions(), STEP_US);
    input.set_viewport(report.viewport());
    input.feed(InputEvent::PointerMoved {
        window: [600.0, 720.0],
    });
    let pointer = input.pointer();
    assert!(!pointer.inside);
    assert!(pointer.layout.unwrap()[0] < 0.0);
    input.feed(InputEvent::PointerMoved {
        window: [4600.0, 720.0],
    });
    let pointer = input.pointer();
    assert!(!pointer.inside);
    assert!(pointer.layout.unwrap()[0] > 1920.0);
    input.feed(InputEvent::PointerMoved {
        window: [2560.0, 720.0],
    });
    let pointer = input.pointer();
    assert!(pointer.inside);
    let [x, y] = pointer.layout.unwrap();
    assert!(close(x, 960.0) && close(y, 540.0), "{x} {y}");

    let deck = policy()
        .fit(
            Device::SteamDeck {
                model: DeckModel::Oled,
            },
            Size {
                width: 1280,
                height: 800,
            },
        )
        .unwrap();
    input.set_viewport(deck.viewport());
    input.feed(InputEvent::PointerMoved {
        window: [1279.0, 0.0],
    });
    let pointer = input.pointer();
    assert!(pointer.inside);
    let [x, y] = pointer.layout.unwrap();
    assert!(
        close(x, 1279.0 * 1080.0 / 800.0) && close(y, 0.0),
        "{x} {y}"
    );
}

fn superwide() -> Input {
    let report = policy()
        .fit(
            Device::Desktop,
            Size {
                width: 5120,
                height: 1440,
            },
        )
        .unwrap();
    let mut input = Input::new(actions(), STEP_US);
    input.set_viewport(report.viewport());
    input
}

fn click(input: &mut Input, window: [f32; 2], pressed: bool) -> pfx_input::ActionState {
    input.feed(InputEvent::PointerMoved { window });
    input.feed(InputEvent::MouseButton {
        button: pfx_input::MouseButton::Left,
        pressed,
    });
    input.step();
    input.action("burst")
}

#[test]
fn a_mouse_press_outside_the_frame_fires_no_action_but_keys_still_do() {
    use pfx_input::{Key, MouseButton};
    let mut input = superwide();
    let outside = [600.0, 720.0];
    let inside = [2560.0, 720.0];
    let burst = click(&mut input, outside, true);
    assert!(!burst.pressed && !burst.held, "{burst:?}");
    assert!(input.mouse_held(MouseButton::Left));
    assert!(input.mouse_outside(MouseButton::Left));
    assert!(!input.pointer().inside);
    input.feed(InputEvent::PointerMoved { window: inside });
    input.step();
    let burst = input.action("burst");
    assert!(
        !burst.held,
        "moving in while held keeps the outside press off"
    );
    let burst = click(&mut input, inside, false);
    assert!(!burst.released && !burst.held);
    assert!(!input.mouse_outside(MouseButton::Left));
    let burst = click(&mut input, inside, true);
    assert!(burst.pressed && burst.held, "{burst:?}");
    let burst = click(&mut input, outside, false);
    assert!(burst.released, "a press from inside still releases outside");

    input.feed(InputEvent::PointerMoved { window: outside });
    input.feed(InputEvent::Key {
        key: Key::Space,
        pressed: true,
    });
    input.step();
    assert!(
        input.action("burst").pressed,
        "keys fire with the pointer outside"
    );

    let mut unknown = Input::new(actions(), STEP_US);
    unknown.feed(InputEvent::MouseButton {
        button: MouseButton::Left,
        pressed: true,
    });
    unknown.step();
    assert!(
        unknown.action("burst").pressed,
        "with no pointer yet a press fires"
    );
}

#[test]
fn a_recording_started_with_an_outside_press_replays_it_held_off() {
    use pfx_input::MouseButton;
    let mut input = superwide();
    click(&mut input, [600.0, 720.0], true);
    input.start_recording();
    input.step();
    let recording = input.stop_recording().unwrap();
    assert_eq!(recording.start.outside, vec![MouseButton::Left]);
    let mut replay = superwide();
    replay.begin_replay(&recording);
    replay.step();
    assert!(replay.mouse_held(MouseButton::Left));
    assert!(replay.mouse_outside(MouseButton::Left));
    assert!(!replay.action("burst").held);
}
