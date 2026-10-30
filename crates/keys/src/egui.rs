use egui::{Event, Key as E, Modifiers};

use crate::{Chord, Key, Mods};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pressed {
    pub chord: Chord,
    pub repeat: bool,
}

const PUNCTUATION: [(E, char); 18] = [
    (E::Colon, ':'),
    (E::Comma, ','),
    (E::Backslash, '\\'),
    (E::Slash, '/'),
    (E::Pipe, '|'),
    (E::Questionmark, '?'),
    (E::Exclamationmark, '!'),
    (E::OpenBracket, '['),
    (E::CloseBracket, ']'),
    (E::OpenCurlyBracket, '{'),
    (E::CloseCurlyBracket, '}'),
    (E::Backtick, '`'),
    (E::Minus, '-'),
    (E::Period, '.'),
    (E::Plus, '+'),
    (E::Equals, '='),
    (E::Semicolon, ';'),
    (E::Quote, '\''),
];

const DIGITS: [E; 10] = [
    E::Num0,
    E::Num1,
    E::Num2,
    E::Num3,
    E::Num4,
    E::Num5,
    E::Num6,
    E::Num7,
    E::Num8,
    E::Num9,
];

const LETTERS: [E; 26] = [
    E::A,
    E::B,
    E::C,
    E::D,
    E::E,
    E::F,
    E::G,
    E::H,
    E::I,
    E::J,
    E::K,
    E::L,
    E::M,
    E::N,
    E::O,
    E::P,
    E::Q,
    E::R,
    E::S,
    E::T,
    E::U,
    E::V,
    E::W,
    E::X,
    E::Y,
    E::Z,
];

const NAMED: [(E, Key); 15] = [
    (E::ArrowDown, Key::Down),
    (E::ArrowUp, Key::Up),
    (E::ArrowLeft, Key::Left),
    (E::ArrowRight, Key::Right),
    (E::Escape, Key::Esc),
    (E::Tab, Key::Tab),
    (E::Backspace, Key::Backspace),
    (E::Enter, Key::Enter),
    (E::Insert, Key::Insert),
    (E::Delete, Key::Delete),
    (E::Home, Key::Home),
    (E::End, Key::End),
    (E::PageUp, Key::PageUp),
    (E::PageDown, Key::PageDown),
    (E::Space, Key::Space),
];

const FUNCTION: [E; 24] = [
    E::F1,
    E::F2,
    E::F3,
    E::F4,
    E::F5,
    E::F6,
    E::F7,
    E::F8,
    E::F9,
    E::F10,
    E::F11,
    E::F12,
    E::F13,
    E::F14,
    E::F15,
    E::F16,
    E::F17,
    E::F18,
    E::F19,
    E::F20,
    E::F21,
    E::F22,
    E::F23,
    E::F24,
];

pub fn key(key: E) -> Option<Key> {
    if let Some((_, named)) = NAMED.iter().find(|(egui, _)| *egui == key) {
        return Some(*named);
    }
    if let Some((_, c)) = PUNCTUATION.iter().find(|(egui, _)| *egui == key) {
        return Some(Key::Char(*c));
    }
    if let Some(at) = DIGITS.iter().position(|egui| *egui == key) {
        return char::from_digit(at as u32, 10).map(Key::Char);
    }
    if let Some(at) = LETTERS.iter().position(|egui| *egui == key) {
        return Some(Key::Char(char::from(b'a' + at as u8)));
    }
    FUNCTION
        .iter()
        .position(|egui| *egui == key)
        .map(|at| Key::F(at as u8 + 1))
}

pub fn mods(modifiers: Modifiers) -> Mods {
    let mut mods = Mods::NONE;
    if modifiers.ctrl || modifiers.command || modifiers.mac_cmd {
        mods = mods.with(Mods::CTRL);
    }
    if modifiers.alt {
        mods = mods.with(Mods::ALT);
    }
    if modifiers.shift {
        mods = mods.with(Mods::SHIFT);
    }
    mods
}

pub fn chord(key: E, modifiers: Modifiers) -> Option<Chord> {
    Some(Chord::new(self::key(key)?, mods(modifiers)))
}

pub fn pressed(events: &[Event]) -> Vec<Pressed> {
    let held = |c: char| Pressed {
        chord: Chord::ctrl(c),
        repeat: false,
    };
    events
        .iter()
        .filter_map(|event| match event {
            Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } => chord(*key, *modifiers).map(|chord| Pressed {
                chord,
                repeat: *repeat,
            }),
            Event::Cut => Some(held('x')),
            Event::Copy => Some(held('c')),
            Event::Paste(_) => Some(held('v')),
            _ => None,
        })
        .collect()
}

pub fn egui_key(key: Key) -> Option<(E, bool)> {
    if let Some((egui, _)) = NAMED.iter().find(|(_, named)| *named == key) {
        return Some((*egui, false));
    }
    match key {
        Key::F(n) => FUNCTION
            .get(usize::from(n).checked_sub(1)?)
            .map(|egui| (*egui, false)),
        Key::Char(c) if c.is_ascii_lowercase() => {
            Some((LETTERS[usize::from(c as u8 - b'a')], false))
        }
        Key::Char(c) if c.is_ascii_uppercase() => {
            Some((LETTERS[usize::from(c as u8 - b'A')], true))
        }
        Key::Char(c) if c.is_ascii_digit() => Some((DIGITS[usize::from(c as u8 - b'0')], false)),
        Key::Char(c) => PUNCTUATION
            .iter()
            .find(|(_, found)| *found == c)
            .map(|(egui, _)| (*egui, false)),
        _ => None,
    }
}

pub fn modifiers(mods: Mods, shift: bool) -> Modifiers {
    let ctrl = mods.contains(Mods::CTRL);
    Modifiers {
        alt: mods.contains(Mods::ALT),
        ctrl,
        shift: shift || mods.contains(Mods::SHIFT),
        mac_cmd: false,
        command: ctrl,
    }
}

pub fn events(chord: Chord) -> Option<[Event; 2]> {
    let (key, shift) = egui_key(chord.key)?;
    let modifiers = modifiers(chord.mods, shift);
    let event = |pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    Some([event(true), event(false)])
}

pub fn name(chord: Chord) -> Option<String> {
    let (key, shift) = egui_key(chord.key)?;
    let mut name = String::new();
    for (on, word) in [
        (chord.mods.contains(Mods::CTRL), "ctrl+"),
        (chord.mods.contains(Mods::ALT), "alt+"),
        (shift || chord.mods.contains(Mods::SHIFT), "shift+"),
    ] {
        if on {
            name.push_str(word);
        }
    }
    name.push_str(key.name());
    Some(name)
}
