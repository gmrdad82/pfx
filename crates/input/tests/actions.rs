mod common;

use common::*;
use pfx_gpu::window::{LAYOUT_UNITS, RenderScale, Size, viewport};
use pfx_input::glyph::{Glyph, Mark};
use pfx_input::{
    Axis, Button, Control, DeviceKind, Family, GlyphStyle, Input, InputEvent, Key, Motors,
    MouseButton, PadId, PadInfo, glyph, glyph_for,
};

fn input() -> Input {
    Input::new(actions(), STEP_US)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn a_key_press_moves_through_pressed_held_and_released() {
    let mut input = input();
    input.feed(key(Key::Space, true));
    input.step();
    let burst = input.action("burst");
    assert!(burst.pressed && burst.held && !burst.released);
    assert_eq!(burst.value, 1.0);
    input.step();
    let burst = input.action("burst");
    assert!(!burst.pressed && burst.held);
    assert_eq!(burst.held_steps, 1);
    input.feed(key(Key::Space, false));
    input.step();
    let burst = input.action("burst");
    assert!(burst.released && !burst.held && !burst.pressed);
    input.step();
    assert_eq!(input.action("burst"), Default::default());
}

#[test]
fn a_tap_inside_one_step_is_not_lost() {
    let mut input = input();
    input.feed(key(Key::Digit2, true));
    input.feed(key(Key::Digit2, false));
    input.step();
    assert!(input.action("rule2").pressed);
    input.step();
    assert!(input.action("rule2").released);
    input.feed(InputEvent::MouseButton {
        button: MouseButton::Left,
        pressed: true,
    });
    input.feed(InputEvent::MouseButton {
        button: MouseButton::Left,
        pressed: false,
    });
    input.step();
    assert!(input.action("burst").pressed);
}

#[test]
fn opposing_keys_cancel_and_pads_drive_the_same_axis() {
    let mut input = input();
    input.feed(key(Key::A, true));
    input.step();
    assert_eq!(input.action("column").value, -1.0);
    assert!(input.action("column").pressed);
    input.feed(key(Key::D, true));
    input.step();
    assert_eq!(input.action("column").value, 0.0);
    assert!(input.action("column").released);
    input.feed(key(Key::A, false));
    input.feed(key(Key::D, false));
    input.feed(connect(0, xbox()));
    input.feed(button(0, Button::RightBumper, 1.0));
    input.step();
    assert_eq!(input.action("column").value, 1.0);
    input.feed(button(0, Button::RightBumper, 0.0));
    input.feed(axis(0, Axis::LeftX, -0.6));
    input.step();
    assert!(close(input.action("column").value, -0.5));
}

#[test]
fn every_named_action_reads_through_the_map() {
    let mut input = input();
    input.feed(connect(3, dualsense()));
    for (button, name) in [
        (Button::South, "burst"),
        (Button::West, "rule1"),
        (Button::North, "rule2"),
        (Button::East, "rule3"),
        (Button::Menu, "pause"),
    ] {
        input.feed(button_event(button, 1.0));
        input.step();
        assert!(input.action(name).pressed, "{name}");
        input.feed(button_event(button, 0.0));
        input.step();
        assert!(input.action(name).released, "{name}");
    }
    assert!(!input.action("missing").held);
}

fn button_event(button: Button, value: f32) -> InputEvent {
    common::button(3, button, value)
}

#[test]
fn the_dead_zone_is_radial_and_rescaled() {
    let mut input = input();
    input.feed(connect(0, xbox()));
    input.feed(axis(0, Axis::LeftX, 0.15));
    input.feed(axis(0, Axis::LeftY, 0.1));
    input.step();
    assert_eq!(input.action("navigate").vector, [0.0, 0.0]);
    assert_eq!(input.action("column").value, 0.0);
    input.feed(axis(0, Axis::LeftX, 0.0));
    input.feed(axis(0, Axis::LeftY, 1.0));
    input.step();
    assert_eq!(input.action("navigate").vector, [0.0, 1.0]);
    input.feed(axis(0, Axis::LeftX, 0.36));
    input.feed(axis(0, Axis::LeftY, 0.48));
    input.step();
    let v = input.action("navigate").vector;
    assert!(close(v[0], 0.6 * 0.5) && close(v[1], 0.8 * 0.5), "{v:?}");
    input.feed(axis(0, Axis::LeftX, 0.0));
    input.feed(axis(0, Axis::LeftY, 0.0));
    input.feed(button(0, Button::RightTrigger, 0.05));
    input.step();
    assert_eq!(input.action("throttle").value, 0.0);
    input.feed(button(0, Button::RightTrigger, 0.55));
    input.step();
    assert!(close(input.action("throttle").value, 0.5));
    input.feed(key(Key::Up, true));
    input.feed(key(Key::Right, true));
    input.step();
    let v = input.action("navigate").vector;
    assert!(close(v[0].hypot(v[1]), 1.0) && close(v[0], v[1]), "{v:?}");
}

#[test]
fn a_held_direction_repeats_after_its_delay() {
    let mut input = input();
    input.feed(key(Key::Down, true));
    let mut fired = Vec::new();
    for step in 0..40 {
        input.step();
        if input.action("navigate").fired() {
            fired.push(step);
        }
    }
    assert_eq!(fired, vec![0, 20, 26, 32, 38]);
    assert_eq!(input.action("navigate").vector, [0.0, -1.0]);
}

#[test]
fn prompts_follow_the_last_used_device() {
    let mut input = input();
    assert_eq!(input.active().kind, DeviceKind::Keyboard);
    assert_eq!(input.prompt("burst"), vec![Control::Key(Key::Space)]);
    input.feed(connect(1, dualsense()));
    let step = input.step();
    assert_eq!(step.changed, None);
    input.feed(axis(1, Axis::LeftX, 0.3));
    assert_eq!(input.step().changed, None);
    input.feed(button(1, Button::South, 1.0));
    let changed = input.step().changed.unwrap();
    assert_eq!(changed.kind, DeviceKind::Gamepad);
    assert_eq!(changed.pad, Some(PadId(1)));
    assert_eq!(changed.family, Some(Family::DualSense));
    assert_eq!(input.prompt("burst"), vec![Control::Button(Button::South)]);
    assert_eq!(
        glyph(input.prompt_family(), input.prompt("burst")[0]),
        Glyph::Disc(Mark::Cross)
    );
    assert_eq!(input.prompt("navigate"), vec![Control::DPad]);
    assert_eq!(input.step().changed, None);
    input.feed(InputEvent::PointerMoved {
        window: [100.0, 100.0],
    });
    input.feed(InputEvent::PointerMoved {
        window: [102.0, 101.0],
    });
    assert_eq!(input.step().changed, None);
    input.feed(InputEvent::PointerMoved {
        window: [104.0, 103.0],
    });
    assert_eq!(input.step().changed.unwrap().kind, DeviceKind::Mouse);
    assert_eq!(
        input.prompt("burst"),
        vec![Control::Mouse(MouseButton::Left)]
    );
    input.feed(key(Key::Escape, true));
    assert_eq!(input.step().changed.unwrap().kind, DeviceKind::Keyboard);
    assert_eq!(
        input.prompt("column"),
        vec![Control::Key(Key::A), Control::Key(Key::D)]
    );
    assert_eq!(input.prompt_family(), Family::DualSense);
    input.feed(connect(2, xbox()));
    input.feed(axis(2, Axis::RightY, -0.9));
    let changed = input.step().changed.unwrap();
    assert_eq!(changed.family, Some(Family::Xbox));
    assert_eq!(
        glyph(input.prompt_family(), input.prompt("burst")[0]),
        Glyph::Disc(Mark::Text("A"))
    );
    input.feed(InputEvent::PadDisconnected { pad: PadId(2) });
    assert_eq!(input.step().changed.unwrap().kind, DeviceKind::Keyboard);
}

#[test]
fn the_glyph_style_follows_the_controller_in_hand() {
    let mut input = input();
    assert_eq!(input.glyph_style(), GlyphStyle::Standard);
    assert_eq!(input.step().style, None);
    input.feed(connect(1, dualsense()));
    input.feed(connect(2, xbox()));
    assert_eq!(input.step().style, None);
    input.feed(button(1, Button::South, 1.0));
    let step = input.step();
    assert_eq!(step.style, Some(GlyphStyle::PlayStation));
    assert_eq!(input.glyph_style(), GlyphStyle::PlayStation);
    assert_eq!(
        glyph_for(input.glyph_style(), input.prompt("burst")[0]),
        Glyph::Disc(Mark::Cross)
    );
    input.feed(button(1, Button::South, 0.0));
    input.feed(button(1, Button::East, 1.0));
    assert_eq!(input.step().style, None);
    input.feed(key(Key::Space, true));
    let step = input.step();
    assert_eq!(step.changed.unwrap().kind, DeviceKind::Keyboard);
    assert_eq!(step.style, None);
    assert_eq!(input.glyph_style(), GlyphStyle::PlayStation);
    input.feed(button(2, Button::South, 1.0));
    let step = input.step();
    assert_eq!(step.style, Some(GlyphStyle::Standard));
    assert_eq!(input.glyph_style(), GlyphStyle::Standard);
    assert_eq!(
        glyph_for(input.glyph_style(), input.prompt("burst")[0]),
        Glyph::Disc(Mark::Text("A"))
    );
    input.feed(button(1, Button::North, 1.0));
    assert_eq!(input.step().style, Some(GlyphStyle::PlayStation));
    input.feed(InputEvent::PadDisconnected { pad: PadId(1) });
    let step = input.step();
    assert_eq!(step.changed.unwrap().kind, DeviceKind::Keyboard);
    assert_eq!(step.style, Some(GlyphStyle::Standard));
}

#[test]
fn the_pointer_lands_in_layout_units() {
    let mut input = input();
    input.set_viewport(viewport(
        Size {
            width: 2560,
            height: 1440,
        },
        RenderScale::Native,
        LAYOUT_UNITS,
    ));
    input.feed(InputEvent::PointerMoved {
        window: [1280.0, 720.0],
    });
    let pointer = input.pointer();
    assert_eq!(pointer.layout, Some([960.0, 540.0]));
    assert!(pointer.inside);
    input.set_viewport(viewport(
        Size {
            width: 2560,
            height: 1600,
        },
        RenderScale::Factor(0.5),
        LAYOUT_UNITS,
    ));
    input.feed(InputEvent::PointerMoved {
        window: [0.0, 40.0],
    });
    let pointer = input.pointer();
    assert!(!pointer.inside);
    let layout = pointer.layout.unwrap();
    assert!(close(layout[0], 0.0) && layout[1] < 0.0, "{layout:?}");
    input.feed(InputEvent::PointerMoved {
        window: [2560.0 / 2.0, 800.0],
    });
    let layout = input.pointer().layout.unwrap();
    assert!(
        close(layout[0], 960.0) && close(layout[1], 540.0),
        "{layout:?}"
    );
    input.feed(InputEvent::PointerLeft);
    assert_eq!(input.pointer().layout, None);
}

#[test]
fn losing_focus_releases_held_keys() {
    let mut input = input();
    input.feed(key(Key::Space, true));
    input.step();
    assert!(input.key_held(Key::Space));
    input.feed(InputEvent::FocusLost);
    input.step();
    assert!(input.action("burst").released);
    assert!(!input.key_held(Key::Space));
}

#[test]
fn the_wheel_taps_and_reports_its_delta() {
    let mut input = input();
    input.feed(InputEvent::Wheel { x: 0.0, y: 2.0 });
    input.feed(InputEvent::Wheel { x: 0.0, y: 1.0 });
    let step = input.step();
    assert_eq!(input.wheel(), [0.0, 3.0]);
    assert_eq!(step.changed.unwrap().kind, DeviceKind::Mouse);
    input.step();
    assert_eq!(input.wheel(), [0.0, 0.0]);
}

#[test]
fn a_pad_that_resolves_its_own_actions_overrides_the_map() {
    let mut input = input();
    let mut info = PadInfo::from_ids("Steam Deck", Some(0x28de), Some(0x1205), Motors::Two);
    info.resolves_actions = true;
    input.feed(connect(7, info));
    input.feed(button(7, Button::South, 1.0));
    let first = input.step();
    assert!(!input.action("burst").held);
    assert_eq!(first.changed.unwrap().family, Some(Family::SteamDeck));
    input.feed(InputEvent::PadAction {
        pad: PadId(7),
        action: "burst".into(),
        value: [1.0, 0.0],
    });
    input.feed(InputEvent::PadOrigins {
        pad: PadId(7),
        action: "burst".into(),
        controls: vec![Control::Button(Button::RightGripLower)],
    });
    input.feed(InputEvent::PadAction {
        pad: PadId(7),
        action: "navigate".into(),
        value: [0.1, -0.9],
    });
    input.step();
    assert!(input.action("burst").pressed);
    let size = 0.9f32.hypot(0.1);
    let expected = -0.9 * ((size - 0.2) / 0.8) / size;
    assert!(close(input.action("navigate").vector[1], expected));
    assert_eq!(
        input.prompt("burst"),
        vec![Control::Button(Button::RightGripLower)]
    );
    assert_eq!(
        glyph(input.prompt_family(), input.prompt("burst")[0]),
        Glyph::Grip("R5")
    );
}
