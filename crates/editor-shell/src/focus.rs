use egui::Key;

pub trait Code: Copy {
    fn code(self) -> &'static str;
}

pub fn from_code<P: Code>(panels: &[P], code: &str) -> Option<P> {
    panels
        .iter()
        .copied()
        .find(|panel| panel.code().eq_ignore_ascii_case(code))
}

fn letter(key: Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), None) if letter.is_ascii_alphabetic() => Some(letter.to_ascii_lowercase()),
        _ => None,
    }
}

fn digit(key: Key) -> Option<char> {
    match key {
        Key::Num0 => Some('0'),
        Key::Num1 => Some('1'),
        Key::Num2 => Some('2'),
        Key::Num3 => Some('3'),
        Key::Num4 => Some('4'),
        Key::Num5 => Some('5'),
        Key::Num6 => Some('6'),
        Key::Num7 => Some('7'),
        Key::Num8 => Some('8'),
        Key::Num9 => Some('9'),
        _ => None,
    }
}

fn starts<P: Code>(panels: &[P], letter: char) -> bool {
    panels.iter().any(|panel| {
        panel
            .code()
            .chars()
            .next()
            .is_some_and(|first| first.eq_ignore_ascii_case(&letter))
    })
}

fn second<P: Code>(panels: &[P], digit: char) -> bool {
    panels
        .iter()
        .any(|panel| panel.code().chars().nth(1) == Some(digit))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pending: Option<char>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press<P> {
    Focus(P),
    Pending,
    Other(Key),
}

impl Keys {
    pub fn pending(&self) -> Option<char> {
        self.pending
    }

    pub fn press<P: Code>(&mut self, panels: &[P], key: Key) -> Press<P> {
        let pending = self.pending.take();
        if let Some(letter) = letter(key).filter(|letter| starts(panels, *letter)) {
            return match from_code(panels, &letter.to_string()) {
                Some(panel) => Press::Focus(panel),
                None => {
                    self.pending = Some(letter);
                    Press::Pending
                }
            };
        }
        let digit = digit(key).filter(|digit| second(panels, *digit));
        if let (Some(letter), Some(digit)) = (pending, digit)
            && let Some(panel) = from_code(panels, &format!("{letter}{digit}"))
        {
            return Press::Focus(panel);
        }
        if pending.is_some() && digit.is_some() {
            return Press::Pending;
        }
        Press::Other(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Panel {
        A,
        B1,
        B2,
        B3,
        C1,
        C2,
        C3,
        D1,
    }

    const ALL: [Panel; 8] = [
        Panel::A,
        Panel::B1,
        Panel::B2,
        Panel::B3,
        Panel::C1,
        Panel::C2,
        Panel::C3,
        Panel::D1,
    ];

    impl Code for Panel {
        fn code(self) -> &'static str {
            match self {
                Panel::A => "A",
                Panel::B1 => "B1",
                Panel::B2 => "B2",
                Panel::B3 => "B3",
                Panel::C1 => "C1",
                Panel::C2 => "C2",
                Panel::C3 => "C3",
                Panel::D1 => "D1",
            }
        }
    }

    fn run(keys: &[Key]) -> Vec<Press<Panel>> {
        let mut state = Keys::default();
        keys.iter().map(|key| state.press(&ALL, *key)).collect()
    }

    #[test]
    fn a_letter_then_a_digit_jumps() {
        assert_eq!(
            run(&[Key::B, Key::Num1]),
            vec![Press::Pending, Press::Focus(Panel::B1)]
        );
        assert_eq!(
            run(&[Key::C, Key::Num3]),
            vec![Press::Pending, Press::Focus(Panel::C3)]
        );
        assert_eq!(
            run(&[Key::D, Key::Num1]),
            vec![Press::Pending, Press::Focus(Panel::D1)]
        );
    }

    #[test]
    fn the_top_bar_answers_its_letter() {
        assert_eq!(run(&[Key::A]), vec![Press::Focus(Panel::A)]);
    }

    #[test]
    fn a_broken_sequence_forgets_its_letter() {
        assert_eq!(
            run(&[Key::B, Key::X, Key::Num1]),
            vec![
                Press::Pending,
                Press::Other(Key::X),
                Press::Other(Key::Num1)
            ]
        );
        assert_eq!(
            run(&[Key::D, Key::Num2, Key::Num1]),
            vec![Press::Pending, Press::Pending, Press::Other(Key::Num1)]
        );
        assert_eq!(run(&[Key::Num1]), vec![Press::Other(Key::Num1)]);
    }

    #[test]
    fn letters_and_digits_that_no_code_uses_pass_through() {
        assert_eq!(
            run(&[Key::E, Key::B, Key::Num4, Key::Num1]),
            vec![
                Press::Other(Key::E),
                Press::Pending,
                Press::Other(Key::Num4),
                Press::Other(Key::Num1)
            ]
        );
        assert_eq!(run(&[Key::Num0]), vec![Press::Other(Key::Num0)]);
    }

    #[test]
    fn every_code_names_its_panel_in_either_case() {
        for panel in ALL {
            assert_eq!(from_code(&ALL, panel.code()), Some(panel));
            assert_eq!(
                from_code(&ALL, &panel.code().to_ascii_lowercase()),
                Some(panel)
            );
        }
        assert_eq!(from_code(&ALL, "E1"), None);
    }

    #[test]
    fn other_codes_parse_their_own_chords() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        struct Named(&'static str);

        impl Code for Named {
            fn code(self) -> &'static str {
                self.0
            }
        }

        let panels = [Named("X"), Named("Y7"), Named("Y9")];
        let mut keys = Keys::default();
        assert_eq!(keys.press(&panels, Key::X), Press::Focus(Named("X")));
        assert_eq!(keys.press(&panels, Key::Y), Press::Pending);
        assert_eq!(keys.pending(), Some('y'));
        assert_eq!(keys.press(&panels, Key::Num9), Press::Focus(Named("Y9")));
        assert_eq!(keys.press(&panels, Key::A), Press::Other(Key::A));
    }
}
