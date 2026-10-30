use std::fmt;
use std::ops::BitOr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Key {
    Char(char),
    F(u8),
    Space,
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const CTRL: Mods = Mods(1);
    pub const ALT: Mods = Mods(2);
    pub const SHIFT: Mods = Mods(4);
    pub const SUPER: Mods = Mods(8);

    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn with(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }

    pub const fn without(self, other: Mods) -> Mods {
        Mods(self.0 & !other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for Mods {
    type Output = Mods;

    fn bitor(self, other: Mods) -> Mods {
        self.with(other)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: Key,
    pub mods: Mods,
}

impl Chord {
    pub const fn new(key: Key, mods: Mods) -> Chord {
        match key {
            Key::Char(' ') => Chord {
                key: Key::Space,
                mods,
            },
            Key::Char(c) => {
                let c = if mods.contains(Mods::SHIFT) {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                Chord {
                    key: Key::Char(c),
                    mods: mods.without(Mods::SHIFT),
                }
            }
            _ => Chord { key, mods },
        }
    }

    pub const fn plain(key: Key) -> Chord {
        Chord::new(key, Mods::NONE)
    }

    pub const fn char(c: char) -> Chord {
        Chord::new(Key::Char(c), Mods::NONE)
    }

    pub const fn ctrl(c: char) -> Chord {
        Chord::new(Key::Char(c), Mods::CTRL)
    }

    pub fn parse(text: &str) -> Result<Chord, Unknown> {
        let unknown = || Unknown(text.to_string());
        let text = text.trim();
        let (mods, key) = if text == "+" {
            ("", "+")
        } else if let Some(head) = text.strip_suffix("++") {
            (head, "+")
        } else {
            match text.rsplit_once('+') {
                Some((head, key)) => (head, key),
                None => ("", text),
            }
        };
        let mut all = Mods::NONE;
        for word in mods.split('+').filter(|word| !word.is_empty()) {
            all = all.with(match word.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => Mods::CTRL,
                "alt" | "option" => Mods::ALT,
                "shift" => Mods::SHIFT,
                "super" | "cmd" | "command" | "meta" => Mods::SUPER,
                _ => return Err(unknown()),
            });
        }
        let key = named(key).ok_or_else(unknown)?;
        Ok(Chord::new(key, all))
    }
}

fn named(word: &str) -> Option<Key> {
    let mut chars = word.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        return Some(match c {
            '↑' => Key::Up,
            '↓' => Key::Down,
            '←' => Key::Left,
            '→' => Key::Right,
            c => Key::Char(c),
        });
    }
    let lower = word.to_ascii_lowercase();
    Some(match lower.as_str() {
        "space" => Key::Space,
        "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Esc,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pgup" | "pageup" => Key::PageUp,
        "pgdn" | "pagedown" => Key::PageDown,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        _ => {
            let n: u8 = lower.strip_prefix('f')?.parse().ok()?;
            if n == 0 {
                return None;
            }
            Key::F(n)
        }
    })
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) if c.is_ascii_uppercase() => write!(f, "shift+{}", c.to_ascii_lowercase()),
            Key::Char(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "f{n}"),
            Key::Space => f.write_str("space"),
            Key::Enter => f.write_str("enter"),
            Key::Esc => f.write_str("esc"),
            Key::Tab => f.write_str("tab"),
            Key::Backspace => f.write_str("backspace"),
            Key::Delete => f.write_str("delete"),
            Key::Insert => f.write_str("insert"),
            Key::Home => f.write_str("home"),
            Key::End => f.write_str("end"),
            Key::PageUp => f.write_str("pgup"),
            Key::PageDown => f.write_str("pgdn"),
            Key::Up => f.write_str("up"),
            Key::Down => f.write_str("down"),
            Key::Left => f.write_str("left"),
            Key::Right => f.write_str("right"),
        }
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let command = if cfg!(target_os = "macos") {
            "cmd+"
        } else {
            "super+"
        };
        for (bit, word) in [
            (Mods::CTRL, "ctrl+"),
            (Mods::ALT, "alt+"),
            (Mods::SHIFT, "shift+"),
            (Mods::SUPER, command),
        ] {
            if self.mods.contains(bit) {
                f.write_str(word)?;
            }
        }
        write!(f, "{}", self.key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unknown(pub String);

impl fmt::Display for Unknown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}' is not a key", self.0)
    }
}

impl std::error::Error for Unknown {}
