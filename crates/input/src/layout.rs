#[cfg(windows)]
mod windows;

use std::collections::BTreeMap;

use crate::device::Key;

#[cfg(windows)]
pub use windows::WindowsLayout;

pub trait LayoutSource {
    fn layout(&mut self) -> u64;

    fn label(&mut self, key: Key) -> Option<String>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Us;

impl LayoutSource for Us {
    fn layout(&mut self) -> u64 {
        0
    }

    fn label(&mut self, _key: Key) -> Option<String> {
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyLabels {
    layout: Option<u64>,
    labels: BTreeMap<Key, String>,
    names: BTreeMap<Key, String>,
}

impl KeyLabels {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn label(&self, key: Key) -> &str {
        self.names
            .get(&key)
            .or_else(|| self.labels.get(&key))
            .map_or_else(|| key.label(), String::as_str)
    }

    pub fn layout(&self) -> Option<u64> {
        self.layout
    }

    pub fn refresh(&mut self, source: &mut (impl LayoutSource + ?Sized)) -> bool {
        let layout = source.layout();
        if self.layout == Some(layout) {
            return false;
        }
        self.layout = Some(layout);
        let labels: BTreeMap<Key, String> = Key::ALL
            .into_iter()
            .filter(|key| key.follows_layout())
            .filter_map(|key| {
                let label = cap_label(&source.label(key)?)?;
                (label != key.label()).then_some((key, label))
            })
            .collect();
        let changed = labels != self.labels;
        self.labels = labels;
        changed
    }

    pub fn learn(&mut self, key: Key, text: &str) -> bool {
        if !key.follows_layout() {
            return false;
        }
        let Some(label) = cap_label(text) else {
            return false;
        };
        let known = self.labels.get(&key).map(String::as_str);
        if known.unwrap_or(key.label()) == label {
            return false;
        }
        if known.is_some() {
            self.labels.clear();
            self.layout = None;
        }
        if label != key.label() {
            self.labels.insert(key, label);
        }
        true
    }

    pub fn rename(&mut self, key: Key, name: &str) {
        match cap_label(name) {
            Some(name) => self.names.insert(key, name),
            None => self.names.remove(&key),
        };
    }

    pub fn forget(&mut self) {
        self.layout = None;
        self.labels.clear();
    }
}

pub fn cap_label(text: &str) -> Option<String> {
    let label: String = text
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(single), None) => single,
                _ => c,
            }
        })
        .collect();
    (!label.is_empty()).then_some(label)
}

pub fn scancode(key: Key) -> Option<u16> {
    Some(match key {
        Key::Escape => 0x01,
        Key::Digit1 => 0x02,
        Key::Digit2 => 0x03,
        Key::Digit3 => 0x04,
        Key::Digit4 => 0x05,
        Key::Digit5 => 0x06,
        Key::Digit6 => 0x07,
        Key::Digit7 => 0x08,
        Key::Digit8 => 0x09,
        Key::Digit9 => 0x0a,
        Key::Digit0 => 0x0b,
        Key::Minus => 0x0c,
        Key::Equal => 0x0d,
        Key::Backspace => 0x0e,
        Key::Tab => 0x0f,
        Key::Q => 0x10,
        Key::W => 0x11,
        Key::E => 0x12,
        Key::R => 0x13,
        Key::T => 0x14,
        Key::Y => 0x15,
        Key::U => 0x16,
        Key::I => 0x17,
        Key::O => 0x18,
        Key::P => 0x19,
        Key::BracketLeft => 0x1a,
        Key::BracketRight => 0x1b,
        Key::Enter => 0x1c,
        Key::ControlLeft => 0x1d,
        Key::A => 0x1e,
        Key::S => 0x1f,
        Key::D => 0x20,
        Key::F => 0x21,
        Key::G => 0x22,
        Key::H => 0x23,
        Key::J => 0x24,
        Key::K => 0x25,
        Key::L => 0x26,
        Key::Semicolon => 0x27,
        Key::Quote => 0x28,
        Key::Backquote => 0x29,
        Key::ShiftLeft => 0x2a,
        Key::Backslash => 0x2b,
        Key::Z => 0x2c,
        Key::X => 0x2d,
        Key::C => 0x2e,
        Key::V => 0x2f,
        Key::B => 0x30,
        Key::N => 0x31,
        Key::M => 0x32,
        Key::Comma => 0x33,
        Key::Period => 0x34,
        Key::Slash => 0x35,
        Key::ShiftRight => 0x36,
        Key::AltLeft => 0x38,
        Key::Space => 0x39,
        Key::CapsLock => 0x3a,
        _ => return None,
    })
}
