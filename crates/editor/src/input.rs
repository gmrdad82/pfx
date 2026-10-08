use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, MouseWheelUnit, PointerButton};
use pfx_input::device::{Key, MouseButton};
use pfx_input::event::InputEvent;
use pfx_input::{KeyLabels, LayoutSource};

pub const POINTS_PER_LINE: f32 = 40.0;

pub fn key(key: egui::Key) -> Option<Key> {
    use egui::Key as E;
    Some(match key {
        E::A => Key::A,
        E::B => Key::B,
        E::C => Key::C,
        E::D => Key::D,
        E::E => Key::E,
        E::F => Key::F,
        E::G => Key::G,
        E::H => Key::H,
        E::I => Key::I,
        E::J => Key::J,
        E::K => Key::K,
        E::L => Key::L,
        E::M => Key::M,
        E::N => Key::N,
        E::O => Key::O,
        E::P => Key::P,
        E::Q => Key::Q,
        E::R => Key::R,
        E::S => Key::S,
        E::T => Key::T,
        E::U => Key::U,
        E::V => Key::V,
        E::W => Key::W,
        E::X => Key::X,
        E::Y => Key::Y,
        E::Z => Key::Z,
        E::Num0 => Key::Digit0,
        E::Num1 => Key::Digit1,
        E::Num2 => Key::Digit2,
        E::Num3 => Key::Digit3,
        E::Num4 => Key::Digit4,
        E::Num5 => Key::Digit5,
        E::Num6 => Key::Digit6,
        E::Num7 => Key::Digit7,
        E::Num8 => Key::Digit8,
        E::Num9 => Key::Digit9,
        E::F1 => Key::F1,
        E::F2 => Key::F2,
        E::F3 => Key::F3,
        E::F4 => Key::F4,
        E::F5 => Key::F5,
        E::F6 => Key::F6,
        E::F7 => Key::F7,
        E::F8 => Key::F8,
        E::F9 => Key::F9,
        E::F10 => Key::F10,
        E::F11 => Key::F11,
        E::F12 => Key::F12,
        E::ArrowUp => Key::Up,
        E::ArrowDown => Key::Down,
        E::ArrowLeft => Key::Left,
        E::ArrowRight => Key::Right,
        E::Escape => Key::Escape,
        E::Tab => Key::Tab,
        E::Space => Key::Space,
        E::Enter => Key::Enter,
        E::Backspace => Key::Backspace,
        E::Insert => Key::Insert,
        E::Delete => Key::Delete,
        E::Home => Key::Home,
        E::End => Key::End,
        E::PageUp => Key::PageUp,
        E::PageDown => Key::PageDown,
        E::Minus => Key::Minus,
        E::Equals => Key::Equal,
        E::OpenBracket => Key::BracketLeft,
        E::CloseBracket => Key::BracketRight,
        E::Backslash => Key::Backslash,
        E::Semicolon => Key::Semicolon,
        E::Quote => Key::Quote,
        E::Backtick => Key::Backquote,
        E::Comma => Key::Comma,
        E::Period => Key::Period,
        E::Slash => Key::Slash,
        _ => return None,
    })
}

#[derive(Clone)]
pub struct Layout(Rc<RefCell<Box<dyn LayoutSource>>>);

impl LayoutSource for Layout {
    fn layout(&mut self) -> u64 {
        self.0.borrow_mut().layout()
    }

    fn label(&mut self, key: Key) -> Option<String> {
        self.0.borrow_mut().label(key)
    }
}

#[cfg(windows)]
fn platform() -> Option<Box<dyn LayoutSource>> {
    Some(Box::new(pfx_input::WindowsLayout::new()))
}

#[cfg(not(windows))]
fn platform() -> Option<Box<dyn LayoutSource>> {
    None
}

pub fn layout(configured: Option<Box<dyn LayoutSource>>) -> Option<Layout> {
    configured
        .or_else(platform)
        .map(|source| Layout(Rc::new(RefCell::new(source))))
}

pub fn learn(labels: &mut KeyLabels, events: &[Event]) -> bool {
    let mut learned = false;
    let mut typing: Option<(Key, bool)> = None;
    for event in events {
        match event {
            Event::Key {
                physical_key: Some(physical),
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } => {
                typing = key(*physical)
                    .filter(|_| !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd)
                    .map(|key| (key, modifiers.shift));
            }
            Event::Key { .. } => typing = None,
            Event::Text(text) => {
                if let Some((key, shifted)) = typing.take()
                    && text.chars().count() == 1
                    && (!shifted || text.chars().all(char::is_alphabetic))
                {
                    learned |= labels.learn(key, text);
                }
            }
            _ => {}
        }
    }
    learned
}

pub fn button(button: PointerButton) -> Option<MouseButton> {
    match button {
        PointerButton::Primary => Some(MouseButton::Left),
        PointerButton::Secondary => Some(MouseButton::Right),
        PointerButton::Middle => Some(MouseButton::Middle),
        PointerButton::Extra1 => Some(MouseButton::Back),
        PointerButton::Extra2 => Some(MouseButton::Forward),
    }
}

fn modifiers(was: egui::Modifiers, now: egui::Modifiers, out: &mut Vec<InputEvent>) {
    for (key, before, after) in [
        (Key::ShiftLeft, was.shift, now.shift),
        (Key::ControlLeft, was.ctrl, now.ctrl),
        (Key::AltLeft, was.alt, now.alt),
    ] {
        if before != after {
            out.push(InputEvent::Key {
                key,
                pressed: after,
            });
        }
    }
}

pub fn events(
    events: &[Event],
    pixels_per_point: f32,
    was: egui::Modifiers,
    now: egui::Modifiers,
) -> Vec<InputEvent> {
    let mut out = Vec::new();
    modifiers(was, now, &mut out);
    for event in events {
        match event {
            Event::Key {
                key: logical,
                physical_key,
                pressed,
                repeat: false,
                ..
            } => {
                if let Some(key) = key(physical_key.unwrap_or(*logical)) {
                    out.push(InputEvent::Key {
                        key,
                        pressed: *pressed,
                    });
                }
            }
            Event::PointerMoved(at) => out.push(InputEvent::PointerMoved {
                window: [at.x * pixels_per_point, at.y * pixels_per_point],
            }),
            Event::PointerButton {
                button: pressed_button,
                pressed,
                ..
            } => {
                if let Some(button) = button(*pressed_button) {
                    out.push(InputEvent::MouseButton {
                        button,
                        pressed: *pressed,
                    });
                }
            }
            Event::MouseWheel { unit, delta, .. } => {
                let lines = match unit {
                    MouseWheelUnit::Point => *delta / POINTS_PER_LINE,
                    MouseWheelUnit::Line | MouseWheelUnit::Page => *delta,
                };
                out.push(InputEvent::Wheel {
                    x: lines.x,
                    y: lines.y,
                });
            }
            Event::PointerGone => out.push(InputEvent::PointerLeft),
            Event::WindowFocused(false) => out.push(InputEvent::FocusLost),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: egui::Key, physical: Option<egui::Key>, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: physical,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn keys_buttons_wheels_and_pointers_reach_the_game_in_window_pixels() {
        let typed = [
            press(egui::Key::Z, Some(egui::Key::Y), true),
            press(egui::Key::F5, None, false),
            Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: true,
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerMoved(egui::pos2(10.0, 20.0)),
            Event::PointerButton {
                pos: egui::pos2(10.0, 20.0),
                button: PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -80.0),
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerGone,
            Event::WindowFocused(false),
            Event::Text("y".into()),
        ];
        assert_eq!(
            events(&typed, 2.0, egui::Modifiers::NONE, egui::Modifiers::NONE),
            vec![
                InputEvent::Key {
                    key: Key::Y,
                    pressed: true
                },
                InputEvent::Key {
                    key: Key::F5,
                    pressed: false
                },
                InputEvent::PointerMoved {
                    window: [20.0, 40.0]
                },
                InputEvent::MouseButton {
                    button: MouseButton::Right,
                    pressed: true
                },
                InputEvent::Wheel { x: 0.0, y: -2.0 },
                InputEvent::PointerLeft,
                InputEvent::FocusLost,
            ]
        );
    }

    fn typed(physical: egui::Key, text: &str, modifiers: egui::Modifiers) -> [Event; 2] {
        [
            Event::Key {
                key: physical,
                physical_key: Some(physical),
                pressed: true,
                repeat: false,
                modifiers,
            },
            Event::Text(text.into()),
        ]
    }

    #[test]
    fn typed_keys_teach_labels_by_physical_position_and_never_when_a_chord() {
        let none = egui::Modifiers::NONE;
        let mut labels = KeyLabels::new();
        assert_eq!(labels.label(Key::Q), "Q");
        assert!(learn(&mut labels, &typed(egui::Key::Q, "a", none)));
        assert_eq!(labels.label(Key::Q), "A");
        assert!(!learn(&mut labels, &typed(egui::Key::Q, "a", none)));
        let shift = egui::Modifiers {
            shift: true,
            ..none
        };
        assert!(!learn(&mut labels, &typed(egui::Key::Num1, "!", shift)));
        assert_eq!(labels.label(Key::Digit1), "1");
        let ctrl = egui::Modifiers { ctrl: true, ..none };
        assert!(!learn(&mut labels, &typed(egui::Key::W, "z", ctrl)));
        assert_eq!(labels.label(Key::W), "W");
        assert!(!learn(&mut labels, &typed(egui::Key::F5, "x", none)));
        let held = [
            Event::Key {
                key: egui::Key::W,
                physical_key: Some(egui::Key::W),
                pressed: true,
                repeat: true,
                modifiers: none,
            },
            Event::Text("z".into()),
        ];
        assert!(!learn(&mut labels, &held));
        let mut unmapped = typed(egui::Key::Z, "y", none);
        if let Event::Key { physical_key, .. } = &mut unmapped[0] {
            *physical_key = None;
        }
        assert!(!learn(&mut labels, &unmapped));
        assert!(learn(&mut labels, &typed(egui::Key::Y, "z", shift)));
        assert_eq!(labels.label(Key::Y), "Z");
    }

    struct Azerty;

    impl LayoutSource for Azerty {
        fn layout(&mut self) -> u64 {
            1
        }

        fn label(&mut self, key: Key) -> Option<String> {
            (key == Key::Q).then(|| "a".to_string())
        }
    }

    fn driver(layout: Option<Layout>) -> pfx_game::Driver {
        let session = pfx_play::PlaySession::play_with(
            &pfx_load::scene::Scene::empty(),
            None,
            Box::new(pfx_play::SceneGame),
            pfx_play::Options::default(),
        )
        .unwrap();
        let mut driver = pfx_game::Driver::new(
            session,
            pfx_play::Settings::default(),
            crate::session::free_policy(),
            pfx_gpu::screens::Device::Desktop,
            pfx_gpu::window::Size {
                width: 640,
                height: 360,
            },
            None,
        );
        driver.set_layout(layout.map(|layout| Box::new(layout) as Box<dyn LayoutSource>));
        driver
    }

    #[test]
    fn an_azerty_stand_in_teaches_the_play_driver_that_q_reads_a() {
        let none = egui::Modifiers::NONE;
        let mut played = driver(None);
        played.update(0);
        assert_eq!(played.labels().label(Key::Q), "Q");
        assert!(learn(played.labels_mut(), &typed(egui::Key::Q, "a", none)));
        played.update(16_666_667);
        assert_eq!(played.labels().label(Key::Q), "A");
        assert_eq!(played.labels().label(Key::W), "W");
    }

    #[test]
    fn a_configured_layout_source_labels_the_play_driver_and_is_shared_across_plays() {
        let layout = layout(Some(Box::new(Azerty))).unwrap();
        for _ in 0..2 {
            let mut played = driver(Some(layout.clone()));
            played.update(0);
            assert_eq!(played.labels().label(Key::Q), "A");
        }
    }

    #[test]
    fn held_modifiers_press_and_release_their_left_keys() {
        let none = egui::Modifiers::NONE;
        let shift = egui::Modifiers {
            shift: true,
            ..none
        };
        let ctrl = egui::Modifiers { ctrl: true, ..none };
        let pressed = |key, pressed| InputEvent::Key { key, pressed };
        assert_eq!(
            events(&[], 1.0, none, shift),
            vec![pressed(Key::ShiftLeft, true)]
        );
        assert_eq!(
            events(&[press(egui::Key::W, None, true)], 1.0, shift, ctrl),
            vec![
                pressed(Key::ShiftLeft, false),
                pressed(Key::ControlLeft, true),
                pressed(Key::W, true),
            ]
        );
        assert!(events(&[], 1.0, ctrl, ctrl).is_empty());
    }
}
