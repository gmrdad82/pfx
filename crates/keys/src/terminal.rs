use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{Chord, Key, Mods};

const MODS: [(KeyModifiers, Mods); 4] = [
    (KeyModifiers::CONTROL, Mods::CTRL),
    (KeyModifiers::ALT, Mods::ALT),
    (KeyModifiers::SHIFT, Mods::SHIFT),
    (KeyModifiers::SUPER, Mods::SUPER),
];

pub fn mods(modifiers: KeyModifiers) -> Mods {
    MODS.iter()
        .filter(|(bit, _)| modifiers.contains(*bit))
        .fold(Mods::NONE, |mods, (_, ours)| mods.with(*ours))
}

pub fn key(code: KeyCode) -> Option<(Key, Mods)> {
    Some((
        match code {
            KeyCode::Char(c) => Key::Char(c),
            KeyCode::F(n) if n > 0 => Key::F(n),
            KeyCode::Enter => Key::Enter,
            KeyCode::Esc => Key::Esc,
            KeyCode::Tab => Key::Tab,
            KeyCode::BackTab => return Some((Key::Tab, Mods::SHIFT)),
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Delete => Key::Delete,
            KeyCode::Insert => Key::Insert,
            KeyCode::Home => Key::Home,
            KeyCode::End => Key::End,
            KeyCode::PageUp => Key::PageUp,
            KeyCode::PageDown => Key::PageDown,
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Left => Key::Left,
            KeyCode::Right => Key::Right,
            _ => return None,
        },
        Mods::NONE,
    ))
}

pub fn chord(event: &KeyEvent) -> Option<Chord> {
    if event.kind == KeyEventKind::Release {
        return None;
    }
    let (key, extra) = key(event.code)?;
    Some(Chord::new(key, mods(event.modifiers).with(extra)))
}

impl Chord {
    pub fn from_crossterm(event: &KeyEvent) -> Option<Chord> {
        chord(event)
    }
}
