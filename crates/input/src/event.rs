use serde::{Deserialize, Serialize};

use crate::device::{Axis, Button, Control, Key, MouseButton, PadId, PadInfo};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    Key {
        key: Key,
        pressed: bool,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    Wheel {
        x: f32,
        y: f32,
    },
    PointerMoved {
        window: [f32; 2],
    },
    PointerAt {
        layout: [f32; 2],
        inside: bool,
    },
    PointerLeft,
    FocusLost,
    PadConnected {
        pad: PadId,
        info: PadInfo,
    },
    PadDisconnected {
        pad: PadId,
    },
    PadButton {
        pad: PadId,
        button: Button,
        value: f32,
    },
    PadAxis {
        pad: PadId,
        axis: Axis,
        value: f32,
    },
    PadAction {
        pad: PadId,
        action: String,
        value: [f32; 2],
    },
    PadOrigins {
        pad: PadId,
        action: String,
        controls: Vec<Control>,
    },
}
