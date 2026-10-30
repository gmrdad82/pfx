use winit::keyboard::{Key as Logical, ModifiersState, NamedKey};

use crate::{Chord, Key, Mods};

const NAMED: [(NamedKey, Key); 15] = [
    (NamedKey::ArrowDown, Key::Down),
    (NamedKey::ArrowUp, Key::Up),
    (NamedKey::ArrowLeft, Key::Left),
    (NamedKey::ArrowRight, Key::Right),
    (NamedKey::Escape, Key::Esc),
    (NamedKey::Tab, Key::Tab),
    (NamedKey::Backspace, Key::Backspace),
    (NamedKey::Enter, Key::Enter),
    (NamedKey::Insert, Key::Insert),
    (NamedKey::Delete, Key::Delete),
    (NamedKey::Home, Key::Home),
    (NamedKey::End, Key::End),
    (NamedKey::PageUp, Key::PageUp),
    (NamedKey::PageDown, Key::PageDown),
    (NamedKey::Space, Key::Space),
];

const FUNCTION: [NamedKey; 24] = [
    NamedKey::F1,
    NamedKey::F2,
    NamedKey::F3,
    NamedKey::F4,
    NamedKey::F5,
    NamedKey::F6,
    NamedKey::F7,
    NamedKey::F8,
    NamedKey::F9,
    NamedKey::F10,
    NamedKey::F11,
    NamedKey::F12,
    NamedKey::F13,
    NamedKey::F14,
    NamedKey::F15,
    NamedKey::F16,
    NamedKey::F17,
    NamedKey::F18,
    NamedKey::F19,
    NamedKey::F20,
    NamedKey::F21,
    NamedKey::F22,
    NamedKey::F23,
    NamedKey::F24,
];

pub fn mods(state: ModifiersState) -> Mods {
    let mut mods = Mods::NONE;
    let command = if cfg!(target_os = "macos") {
        Mods::CTRL
    } else {
        Mods::SUPER
    };
    for (on, ours) in [
        (state.control_key(), Mods::CTRL),
        (state.alt_key(), Mods::ALT),
        (state.shift_key(), Mods::SHIFT),
        (state.super_key(), command),
    ] {
        if on {
            mods = mods.with(ours);
        }
    }
    mods
}

pub fn key(logical: &Logical, shift: bool) -> Option<Key> {
    match logical {
        Logical::Named(named) => NAMED
            .iter()
            .find(|(found, _)| found == named)
            .map(|(_, key)| *key)
            .or_else(|| {
                FUNCTION
                    .iter()
                    .position(|found| found == named)
                    .map(|at| Key::F(at as u8 + 1))
            }),
        Logical::Character(text) => {
            let mut chars = text.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            if c.is_control() {
                return None;
            }
            Some(Key::Char(match (c.is_ascii_alphabetic(), shift) {
                (true, true) => c.to_ascii_uppercase(),
                (true, false) => c.to_ascii_lowercase(),
                (false, _) => c,
            }))
        }
        _ => None,
    }
}

pub fn chord(logical: &Logical, state: ModifiersState) -> Option<Chord> {
    Some(Chord::new(key(logical, state.shift_key())?, mods(state)))
}
