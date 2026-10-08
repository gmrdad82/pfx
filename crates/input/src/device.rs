use serde::{Deserialize, Serialize};

use crate::family::Family;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PadId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DeviceKind {
    Keyboard,
    Mouse,
    Gamepad,
}

impl DeviceKind {
    pub fn is_gamepad(self) -> bool {
        self == DeviceKind::Gamepad
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Up,
    Down,
    Left,
    Right,
    Escape,
    Tab,
    CapsLock,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,
    ContextMenu,
    Space,
    Enter,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    PrintScreen,
    ScrollLock,
    Pause,
    NumLock,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    Comma,
    Period,
    Slash,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    NumpadEnter,
}

impl Key {
    pub const ALL: [Key; 104] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
        Key::Digit0,
        Key::Digit1,
        Key::Digit2,
        Key::Digit3,
        Key::Digit4,
        Key::Digit5,
        Key::Digit6,
        Key::Digit7,
        Key::Digit8,
        Key::Digit9,
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Escape,
        Key::Tab,
        Key::CapsLock,
        Key::ShiftLeft,
        Key::ShiftRight,
        Key::ControlLeft,
        Key::ControlRight,
        Key::AltLeft,
        Key::AltRight,
        Key::SuperLeft,
        Key::SuperRight,
        Key::ContextMenu,
        Key::Space,
        Key::Enter,
        Key::Backspace,
        Key::Insert,
        Key::Delete,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::PrintScreen,
        Key::ScrollLock,
        Key::Pause,
        Key::NumLock,
        Key::Minus,
        Key::Equal,
        Key::BracketLeft,
        Key::BracketRight,
        Key::Backslash,
        Key::Semicolon,
        Key::Quote,
        Key::Backquote,
        Key::Comma,
        Key::Period,
        Key::Slash,
        Key::Numpad0,
        Key::Numpad1,
        Key::Numpad2,
        Key::Numpad3,
        Key::Numpad4,
        Key::Numpad5,
        Key::Numpad6,
        Key::Numpad7,
        Key::Numpad8,
        Key::Numpad9,
        Key::NumpadAdd,
        Key::NumpadSubtract,
        Key::NumpadMultiply,
        Key::NumpadDivide,
        Key::NumpadDecimal,
        Key::NumpadEnter,
    ];

    pub fn follows_layout(self) -> bool {
        matches!(
            self,
            Key::A
                | Key::B
                | Key::C
                | Key::D
                | Key::E
                | Key::F
                | Key::G
                | Key::H
                | Key::I
                | Key::J
                | Key::K
                | Key::L
                | Key::M
                | Key::N
                | Key::O
                | Key::P
                | Key::Q
                | Key::R
                | Key::S
                | Key::T
                | Key::U
                | Key::V
                | Key::W
                | Key::X
                | Key::Y
                | Key::Z
                | Key::Minus
                | Key::Equal
                | Key::BracketLeft
                | Key::BracketRight
                | Key::Backslash
                | Key::Semicolon
                | Key::Quote
                | Key::Backquote
                | Key::Comma
                | Key::Period
                | Key::Slash
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Key::A => "A",
            Key::B => "B",
            Key::C => "C",
            Key::D => "D",
            Key::E => "E",
            Key::F => "F",
            Key::G => "G",
            Key::H => "H",
            Key::I => "I",
            Key::J => "J",
            Key::K => "K",
            Key::L => "L",
            Key::M => "M",
            Key::N => "N",
            Key::O => "O",
            Key::P => "P",
            Key::Q => "Q",
            Key::R => "R",
            Key::S => "S",
            Key::T => "T",
            Key::U => "U",
            Key::V => "V",
            Key::W => "W",
            Key::X => "X",
            Key::Y => "Y",
            Key::Z => "Z",
            Key::Digit0 | Key::Numpad0 => "0",
            Key::Digit1 | Key::Numpad1 => "1",
            Key::Digit2 | Key::Numpad2 => "2",
            Key::Digit3 | Key::Numpad3 => "3",
            Key::Digit4 | Key::Numpad4 => "4",
            Key::Digit5 | Key::Numpad5 => "5",
            Key::Digit6 | Key::Numpad6 => "6",
            Key::Digit7 | Key::Numpad7 => "7",
            Key::Digit8 | Key::Numpad8 => "8",
            Key::Digit9 | Key::Numpad9 => "9",
            Key::F1 => "F1",
            Key::F2 => "F2",
            Key::F3 => "F3",
            Key::F4 => "F4",
            Key::F5 => "F5",
            Key::F6 => "F6",
            Key::F7 => "F7",
            Key::F8 => "F8",
            Key::F9 => "F9",
            Key::F10 => "F10",
            Key::F11 => "F11",
            Key::F12 => "F12",
            Key::Up => "^",
            Key::Down => "v",
            Key::Left => "<",
            Key::Right => ">",
            Key::Escape => "ESC",
            Key::Tab => "TAB",
            Key::CapsLock => "CAPS",
            Key::ShiftLeft | Key::ShiftRight => "SHIFT",
            Key::ControlLeft | Key::ControlRight => "CTRL",
            Key::AltLeft | Key::AltRight => "ALT",
            Key::SuperLeft | Key::SuperRight => "META",
            Key::ContextMenu => "MENU",
            Key::Space => "SPACE",
            Key::Enter | Key::NumpadEnter => "ENTER",
            Key::Backspace => "BKSP",
            Key::Insert => "INS",
            Key::Delete => "DEL",
            Key::Home => "HOME",
            Key::End => "END",
            Key::PageUp => "PGUP",
            Key::PageDown => "PGDN",
            Key::PrintScreen => "PRTSC",
            Key::ScrollLock => "SCRLK",
            Key::Pause => "PAUSE",
            Key::NumLock => "NUMLK",
            Key::Minus | Key::NumpadSubtract => "-",
            Key::Equal => "=",
            Key::BracketLeft => "[",
            Key::BracketRight => "]",
            Key::Backslash => "\\",
            Key::Semicolon => ";",
            Key::Quote => "'",
            Key::Backquote => "`",
            Key::Comma => ",",
            Key::Period | Key::NumpadDecimal => ".",
            Key::Slash | Key::NumpadDivide => "/",
            Key::NumpadAdd => "+",
            Key::NumpadMultiply => "*",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Wheel {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Button {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Menu,
    View,
    Guide,
    Touchpad,
    LeftGrip,
    RightGrip,
    LeftGripLower,
    RightGripLower,
    Misc,
}

impl Button {
    pub const ALL: [Button; 23] = [
        Button::South,
        Button::East,
        Button::West,
        Button::North,
        Button::LeftBumper,
        Button::RightBumper,
        Button::LeftTrigger,
        Button::RightTrigger,
        Button::LeftStick,
        Button::RightStick,
        Button::DPadUp,
        Button::DPadDown,
        Button::DPadLeft,
        Button::DPadRight,
        Button::Menu,
        Button::View,
        Button::Guide,
        Button::Touchpad,
        Button::LeftGrip,
        Button::RightGrip,
        Button::LeftGripLower,
        Button::RightGripLower,
        Button::Misc,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Axis {
    LeftX,
    LeftY,
    RightX,
    RightY,
}

impl Axis {
    pub fn stick(self) -> Stick {
        match self {
            Axis::LeftX | Axis::LeftY => Stick::Left,
            Axis::RightX | Axis::RightY => Stick::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Stick {
    Left,
    Right,
}

impl Stick {
    pub fn axes(self) -> (Axis, Axis) {
        match self {
            Stick::Left => (Axis::LeftX, Axis::LeftY),
            Stick::Right => (Axis::RightX, Axis::RightY),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Sign {
    Negative,
    Positive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Control {
    Key(Key),
    Mouse(MouseButton),
    Wheel(Wheel),
    MouseMove,
    Button(Button),
    Stick(Stick),
    DPad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Motors {
    None,
    One,
    Two,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadInfo {
    pub name: String,
    pub vendor: Option<u16>,
    pub product: Option<u16>,
    pub family: Family,
    pub motors: Motors,
    pub resolves_actions: bool,
}

impl PadInfo {
    pub fn from_ids(name: &str, vendor: Option<u16>, product: Option<u16>, motors: Motors) -> Self {
        Self {
            name: name.to_owned(),
            vendor,
            product,
            family: match (vendor, product) {
                (Some(vendor), Some(product)) => Family::from_ids(vendor, product),
                _ => Family::Generic,
            },
            motors,
            resolves_actions: false,
        }
    }
}
