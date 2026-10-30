use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use egui::{Event, Key as E, Modifiers};
use winit::keyboard::{Key as Logical, ModifiersState, NamedKey, SmolStr};

use crate::{Chord, Key, Keymap, Mode, Mods, Now, Outcome, Sheet, When, bind};

const LIST: Mode = Mode("list");
const PAGE: Mode = Mode("page");
const PICKED: When = When("while a row is picked");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Step,
    Open,
    Remove,
    Undo,
    Redo,
    Back,
}

fn keymap() -> Keymap<Act> {
    Keymap::new()
        .modes(&[LIST, PAGE])
        .group(
            "List",
            [
                bind("up down", "steps through the rows", Act::Step)
                    .on(LIST)
                    .shown("up and down"),
                bind("enter", "opens the row", Act::Open).on(LIST),
                bind("delete", "removes it", Act::Remove)
                    .on(LIST)
                    .when(PICKED),
            ],
        )
        .group(
            "Edit",
            [
                bind("ctrl+z", "undoes", Act::Undo),
                bind("ctrl+shift+z ctrl+y", "redoes", Act::Redo),
                bind("esc", "goes back", Act::Back).on(PAGE),
            ],
        )
}

fn key_event(key: E, modifiers: Modifiers, repeat: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat,
        modifiers,
    }
}

#[test]
fn every_egui_key_the_tables_name_comes_back_from_its_chord() {
    let mut seen = 0;
    for key in [
        E::ArrowDown,
        E::ArrowUp,
        E::ArrowLeft,
        E::ArrowRight,
        E::Escape,
        E::Tab,
        E::Backspace,
        E::Enter,
        E::Space,
        E::Insert,
        E::Delete,
        E::Home,
        E::End,
        E::PageUp,
        E::PageDown,
        E::Colon,
        E::Comma,
        E::Backslash,
        E::Slash,
        E::Pipe,
        E::Questionmark,
        E::Exclamationmark,
        E::OpenBracket,
        E::CloseBracket,
        E::OpenCurlyBracket,
        E::CloseCurlyBracket,
        E::Backtick,
        E::Minus,
        E::Period,
        E::Plus,
        E::Equals,
        E::Semicolon,
        E::Quote,
        E::Num0,
        E::Num5,
        E::Num9,
        E::A,
        E::K,
        E::Z,
        E::F1,
        E::F3,
        E::F10,
        E::F24,
    ] {
        let chord = super::egui::chord(key, Modifiers::NONE).unwrap();
        let [down, up] = super::egui::events(chord).unwrap();
        assert_eq!(
            super::egui::pressed(&[down]),
            [super::egui::Pressed {
                chord,
                repeat: false
            }],
            "{key:?}"
        );
        assert!(super::egui::pressed(&[up]).is_empty());
        seen += 1;
    }
    assert_eq!(seen, 43);
}

#[test]
fn egui_keys_read_as_the_keymaps_words() {
    let plain = |key| super::egui::chord(key, Modifiers::NONE);
    assert_eq!(plain(E::Questionmark), Chord::parse("?").ok());
    assert_eq!(plain(E::Slash), Chord::parse("/").ok());
    assert_eq!(plain(E::Minus), Chord::parse("-").ok());
    assert_eq!(plain(E::Num7), Chord::parse("7").ok());
    assert_eq!(plain(E::F5), Chord::parse("f5").ok());
    assert_eq!(plain(E::Space), Chord::parse("space").ok());
    assert_eq!(plain(E::ArrowLeft), Chord::parse("left").ok());
    assert_eq!(plain(E::Escape), Chord::parse("esc").ok());
    assert_eq!(plain(E::BrowserBack), None);
    assert_eq!(plain(E::ShiftLeft), None);
    assert_eq!(plain(E::F35), None);
}

#[test]
fn shift_ctrl_command_and_alt_reach_the_chord() {
    assert_eq!(
        super::egui::chord(E::Z, Modifiers::SHIFT),
        Chord::parse("shift+z").ok()
    );
    assert_eq!(
        super::egui::chord(E::Z, Modifiers::CTRL | Modifiers::SHIFT),
        Chord::parse("ctrl+shift+z").ok()
    );
    assert_eq!(
        super::egui::chord(E::S, Modifiers::COMMAND),
        Chord::parse("ctrl+s").ok()
    );
    assert_eq!(
        super::egui::chord(E::S, Modifiers::MAC_CMD),
        Chord::parse("ctrl+s").ok()
    );
    assert_eq!(
        super::egui::chord(E::Enter, Modifiers::ALT),
        Chord::parse("alt+enter").ok()
    );
    assert_eq!(
        super::egui::chord(E::Questionmark, Modifiers::SHIFT),
        Chord::parse("?").ok(),
        "a ? typed with shift is still ?"
    );
    assert_eq!(
        super::egui::chord(E::ArrowLeft, Modifiers::SHIFT),
        Chord::parse("shift+left").ok()
    );
}

#[test]
fn cut_copy_and_paste_events_read_as_their_ctrl_chords_and_repeats_are_marked() {
    let pressed = super::egui::pressed(&[
        Event::Cut,
        Event::Copy,
        Event::Paste("text".into()),
        Event::Text("t".into()),
        key_event(E::J, Modifiers::NONE, true),
    ]);
    let chords: Vec<String> = pressed.iter().map(|p| p.chord.to_string()).collect();
    assert_eq!(chords, ["ctrl+x", "ctrl+c", "ctrl+v", "j"]);
    assert_eq!(
        pressed.iter().map(|p| p.repeat).collect::<Vec<_>>(),
        [false, false, false, true]
    );
}

#[test]
fn a_chord_names_the_egui_key_a_script_types() {
    let name = |text: &str| super::egui::name(Chord::parse(text).unwrap()).unwrap();
    assert_eq!(name("ctrl+shift+z"), "ctrl+shift+Z");
    assert_eq!(name("shift+b"), "shift+B");
    assert_eq!(name("?"), "Questionmark");
    assert_eq!(name("f5"), "F5");
    assert_eq!(name("esc"), "Escape");
    assert_eq!(name("1"), "1");
    for text in ["ctrl+shift+z", "shift+b", "?", "f5", "esc", "1", "["] {
        let shown = name(text);
        let bare = shown.rsplit('+').next().unwrap();
        assert!(E::from_name(bare).is_some(), "{shown}");
    }
}

#[test]
fn winit_keys_read_as_the_keymaps_words_whatever_caps_lock_says() {
    let chord = |logical: Logical, state| super::winit::chord(&logical, state);
    let none = ModifiersState::empty();
    let character = |text: &str| Logical::Character(SmolStr::new(text));
    assert_eq!(
        chord(character("?"), ModifiersState::SHIFT),
        Chord::parse("?").ok()
    );
    assert_eq!(chord(character("A"), none), Chord::parse("a").ok());
    assert_eq!(
        chord(character("a"), ModifiersState::SHIFT),
        Chord::parse("shift+a").ok()
    );
    assert_eq!(
        chord(character("z"), ModifiersState::CONTROL),
        Chord::parse("ctrl+z").ok()
    );
    assert_eq!(
        chord(Logical::Named(NamedKey::F3), none),
        Chord::parse("f3").ok()
    );
    assert_eq!(
        chord(Logical::Named(NamedKey::Enter), ModifiersState::ALT),
        Chord::parse("alt+enter").ok()
    );
    assert_eq!(
        chord(Logical::Named(NamedKey::Escape), none),
        Chord::parse("esc").ok()
    );
    assert_eq!(chord(character(" "), none), Chord::parse("space").ok());
    assert_eq!(chord(Logical::Named(NamedKey::Shift), none), None);
    assert_eq!(chord(character("ab"), none), None);
    assert_eq!(chord(character("\u{8}"), none), None);
    let command = super::winit::mods(ModifiersState::SUPER);
    if cfg!(target_os = "macos") {
        assert_eq!(command, Mods::CTRL);
    } else {
        assert_eq!(command, Mods::SUPER);
    }
}

#[test]
fn every_function_key_up_to_f24_converts_both_ways() {
    for n in 1..=24u8 {
        let chord = Chord::plain(Key::F(n));
        let [down, _] = super::egui::events(chord).unwrap();
        assert_eq!(super::egui::pressed(&[down])[0].chord, chord);
        let named = super::winit::key(
            &Logical::Named(match n {
                1 => NamedKey::F1,
                12 => NamedKey::F12,
                24 => NamedKey::F24,
                _ => continue,
            }),
            false,
        );
        assert_eq!(named, Some(Key::F(n)));
    }
}

#[test]
fn an_action_names_its_keys_and_the_first_is_written() {
    let keys = keymap();
    assert_eq!(
        crate::chords_of(&keys, &Act::Redo),
        [
            Chord::parse("ctrl+shift+z").unwrap(),
            Chord::parse("ctrl+y").unwrap()
        ]
    );
    assert_eq!(
        crate::written(&keys, &Act::Redo).as_deref(),
        Some("ctrl+shift+z")
    );
    assert_eq!(crate::written(&keys, &Act::Step).as_deref(), Some("up"));
}

#[test]
fn the_sheet_lists_the_keymaps_groups_and_its_help_and_every_hint_is_on_it() {
    let keys = keymap();
    assert!(crate::clash_lines(&keys).is_empty());
    for (mode, groups) in [
        (LIST, ["List", "Edit", "Help"]),
        (PAGE, ["Edit", "Help", "List"]),
    ] {
        for when in [&[][..], &[PICKED][..]] {
            let now = Now::new(mode).when(when);
            let view = Sheet::new().view(&keys, now);
            assert_eq!(crate::shown_groups(&view), groups);
            assert!(crate::hints_off_the_sheet(&keys, now).is_empty());
        }
    }
    assert_eq!(crate::group_names(&keys), ["List", "Edit"]);
}

#[test]
fn a_clash_is_written_out() {
    let keys = Keymap::new().group(
        "Twice",
        [
            bind("x", "does one", Act::Open),
            bind("x", "does two", Act::Back),
        ],
    );
    assert_eq!(crate::clash_lines(&keys).len(), 1);
}

#[test]
fn the_question_mark_opens_the_sheet_and_esc_closes_it_through_converted_events() {
    let mut sheet = Sheet::new();
    let question = super::egui::chord(E::Questionmark, Modifiers::SHIFT).unwrap();
    let escape = super::egui::chord(E::Escape, Modifiers::NONE).unwrap();
    let down = super::egui::chord(E::ArrowDown, Modifiers::NONE).unwrap();
    assert_eq!(sheet.key(question), Outcome::Took);
    assert!(sheet.is_open());
    assert_eq!(sheet.key(down), Outcome::Pass);
    assert_eq!(sheet.key(escape), Outcome::Took);
    assert!(!sheet.is_open());
}

fn terminal(code: KeyCode, modifiers: KeyModifiers) -> Option<Chord> {
    super::terminal::chord(&KeyEvent::new(code, modifiers))
}

#[test]
fn every_terminal_key_reads_as_the_keymaps_words() {
    let plain = |code| terminal(code, KeyModifiers::NONE);
    for (code, words) in [
        (KeyCode::Char('a'), "a"),
        (KeyCode::Char('?'), "?"),
        (KeyCode::Char('7'), "7"),
        (KeyCode::Char(' '), "space"),
        (KeyCode::F(1), "f1"),
        (KeyCode::F(24), "f24"),
        (KeyCode::Enter, "enter"),
        (KeyCode::Esc, "esc"),
        (KeyCode::Tab, "tab"),
        (KeyCode::BackTab, "shift+tab"),
        (KeyCode::Backspace, "backspace"),
        (KeyCode::Delete, "delete"),
        (KeyCode::Insert, "insert"),
        (KeyCode::Home, "home"),
        (KeyCode::End, "end"),
        (KeyCode::PageUp, "pgup"),
        (KeyCode::PageDown, "pgdn"),
        (KeyCode::Up, "up"),
        (KeyCode::Down, "down"),
        (KeyCode::Left, "left"),
        (KeyCode::Right, "right"),
    ] {
        assert_eq!(plain(code), Chord::parse(words).ok(), "{code:?}");
    }
    assert_eq!(plain(KeyCode::F(0)), None);
    assert_eq!(plain(KeyCode::CapsLock), None);
    assert_eq!(plain(KeyCode::Null), None);
}

#[test]
fn terminal_modifiers_reach_the_chord_and_a_release_is_no_key() {
    assert_eq!(
        terminal(KeyCode::Char('z'), KeyModifiers::CONTROL),
        Chord::parse("ctrl+z").ok()
    );
    assert_eq!(
        terminal(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        Chord::parse("ctrl+shift+z").ok()
    );
    assert_eq!(
        terminal(KeyCode::Char('Z'), KeyModifiers::SHIFT),
        Chord::parse("shift+z").ok()
    );
    assert_eq!(
        terminal(KeyCode::Enter, KeyModifiers::ALT),
        Chord::parse("alt+enter").ok()
    );
    assert_eq!(
        terminal(KeyCode::Char('s'), KeyModifiers::SUPER),
        Chord::parse("super+s").ok()
    );
    assert_eq!(
        terminal(KeyCode::BackTab, KeyModifiers::SHIFT),
        Chord::parse("shift+tab").ok()
    );
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    assert_eq!(super::terminal::chord(&release), None);
    let repeat =
        KeyEvent::new_with_kind(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Repeat);
    assert_eq!(super::terminal::chord(&repeat), Chord::parse("a").ok());
    let event = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT);
    assert_eq!(
        Chord::from_crossterm(&event),
        super::terminal::chord(&event)
    );
}

#[test]
fn a_terminal_key_does_what_the_keymap_says_and_opens_the_sheet() {
    let keys = keymap();
    let list = Now::new(LIST);
    let picked = Now::new(LIST).when(&[PICKED]);
    let delete = terminal(KeyCode::Delete, KeyModifiers::NONE).unwrap();
    assert_eq!(keys.action(list, delete), None);
    assert_eq!(keys.action(picked, delete), Some(&Act::Remove));
    let redo = terminal(KeyCode::Char('y'), KeyModifiers::CONTROL).unwrap();
    assert_eq!(keys.action(list, redo), Some(&Act::Redo));
    let mut sheet = Sheet::new();
    let question = terminal(KeyCode::Char('?'), KeyModifiers::SHIFT).unwrap();
    assert_eq!(sheet.key(question), Outcome::Took);
    assert!(sheet.is_open());
    let escape = terminal(KeyCode::Esc, KeyModifiers::NONE).unwrap();
    assert_eq!(sheet.key(escape), Outcome::Took);
    assert!(!sheet.is_open());
}
