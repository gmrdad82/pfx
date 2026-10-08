#![allow(dead_code)]

use pfx_input::{
    ActionSpec, Actions, Axis, Binding, Button, InputEvent, Key, Motors, MouseButton, PadId,
    PadInfo, Stick,
};

pub const STEP_US: u32 = 16_667;

pub fn actions() -> Actions {
    Actions::new(vec![
        ActionSpec::axis("column")
            .context("play")
            .key(Binding::keys(Key::A, Key::D))
            .key(Binding::keys(Key::Left, Key::Right))
            .pad(Binding::buttons(Button::LeftBumper, Button::RightBumper))
            .pad(Binding::buttons(Button::DPadLeft, Button::DPadRight))
            .pad(Binding::PadAxis(Axis::LeftX)),
        ActionSpec::digital("burst")
            .context("play")
            .key(Binding::key(Key::Space))
            .key(Binding::mouse(MouseButton::Left))
            .pad(Binding::button(Button::South)),
        ActionSpec::digital("rule1")
            .context("play")
            .key(Binding::key(Key::Digit1))
            .pad(Binding::button(Button::West)),
        ActionSpec::digital("rule2")
            .context("play")
            .key(Binding::key(Key::Digit2))
            .pad(Binding::button(Button::North)),
        ActionSpec::digital("rule3")
            .context("play")
            .key(Binding::key(Key::Digit3))
            .pad(Binding::button(Button::East)),
        ActionSpec::digital("pause")
            .context("play")
            .key(Binding::key(Key::Escape))
            .pad(Binding::button(Button::Menu)),
        ActionSpec::stick("navigate")
            .context("menu")
            .repeat(20, 6)
            .key(Binding::key_stick(
                Key::Up,
                Key::Down,
                Key::Left,
                Key::Right,
            ))
            .pad(Binding::dpad())
            .pad(Binding::PadStick(Stick::Left)),
        ActionSpec::digital("confirm")
            .context("menu")
            .key(Binding::key(Key::Enter))
            .pad(Binding::button(Button::South)),
        ActionSpec::digital("back")
            .context("menu")
            .key(Binding::key(Key::Escape))
            .pad(Binding::button(Button::East)),
        ActionSpec::axis("throttle")
            .context("play")
            .dead_zone(0.1)
            .pad(Binding::button(Button::RightTrigger)),
    ])
    .unwrap()
}

pub fn dualsense() -> PadInfo {
    PadInfo::from_ids(
        "DualSense Wireless Controller",
        Some(0x054c),
        Some(0x0ce6),
        Motors::Two,
    )
}

pub fn xbox() -> PadInfo {
    PadInfo::from_ids(
        "Xbox Series X Controller",
        Some(0x045e),
        Some(0x0b12),
        Motors::Two,
    )
}

pub fn key(key: Key, pressed: bool) -> InputEvent {
    InputEvent::Key { key, pressed }
}

pub fn button(pad: u32, button: Button, value: f32) -> InputEvent {
    InputEvent::PadButton {
        pad: PadId(pad),
        button,
        value,
    }
}

pub fn axis(pad: u32, axis: Axis, value: f32) -> InputEvent {
    InputEvent::PadAxis {
        pad: PadId(pad),
        axis,
        value,
    }
}

pub fn connect(pad: u32, info: PadInfo) -> InputEvent {
    InputEvent::PadConnected {
        pad: PadId(pad),
        info,
    }
}
