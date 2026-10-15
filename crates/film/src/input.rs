use pfx_input::{InputEvent, Key, MouseButton};
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Touch {
    pub at: f64,
    #[serde(default)]
    pub pointer: Option<[f32; 2]>,
    #[serde(default)]
    pub click: bool,
    #[serde(default)]
    pub key: Option<String>,
}

const DIGITS: [Key; 10] = [
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
];

pub fn key(name: &str) -> Option<Key> {
    if let [d @ b'0'..=b'9'] = name.as_bytes() {
        return Some(DIGITS[usize::from(d - b'0')]);
    }
    let bare: String = name.chars().filter(|c| *c != '_').collect();
    Key::ALL
        .into_iter()
        .find(|k| format!("{k:?}").eq_ignore_ascii_case(&bare))
}

pub fn check(touches: &[Touch]) -> Result<(), String> {
    match touches
        .iter()
        .find_map(|t| t.key.as_deref().filter(|k| key(k).is_none()))
    {
        Some(k) => Err(format!(
            "input key '{k}' isn't a pfx-input key: 0 to 9, a to z, f1 to f12, left, right, up, down, enter, space, escape, tab, shift_left, page_up, numpad_1, …"
        )),
        None => Ok(()),
    }
}

pub fn events(touches: &[Touch], (from, to): (f64, f64)) -> Vec<InputEvent> {
    let mut out = Vec::new();
    for t in touches.iter().filter(|t| t.at >= from && t.at < to) {
        if let Some(layout) = t.pointer {
            out.push(InputEvent::PointerAt {
                layout,
                inside: true,
            });
        }
        if t.click {
            for pressed in [true, false] {
                out.push(InputEvent::MouseButton {
                    button: MouseButton::Left,
                    pressed,
                });
            }
        }
        if let Some(k) = t.key.as_deref().and_then(key) {
            for pressed in [true, false] {
                out.push(InputEvent::Key { key: k, pressed });
            }
        }
    }
    out
}
