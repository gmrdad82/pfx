use crate::{Chord, Key, Keymap, Mode, Mods, Now, Outcome, Sheet, When, bind};

const RAIL: Mode = Mode("clips");
const CUT: Mode = Mode("cut");
const PLAYING: When = When("while the cut plays");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Keep,
    Drop,
    Play,
    Stop,
    Undo,
}

fn keymap() -> Keymap<Act> {
    Keymap::new()
        .group(
            "Rail",
            [
                bind("right", "keeps", Act::Keep).on(RAIL),
                bind("left", "drops", Act::Drop).on(RAIL).rank(1),
            ],
        )
        .group(
            "Player",
            [
                bind("space esc", "stops it", Act::Stop)
                    .modes(&[RAIL, CUT])
                    .when(PLAYING)
                    .shown("space or esc"),
                bind("space", "plays", Act::Play).modes(&[RAIL, CUT]),
            ],
        )
        .group("Edit", [bind("ctrl+z", "undoes", Act::Undo).rank(2)])
}

#[test]
fn the_action_follows_the_mode_and_the_context() {
    let keys = keymap();
    let rail = Now::new(RAIL);
    assert_eq!(
        keys.action(rail, Chord::plain(Key::Right)),
        Some(&Act::Keep)
    );
    assert_eq!(keys.action(Now::new(CUT), Chord::plain(Key::Right)), None);
    assert_eq!(
        keys.action(rail, Chord::plain(Key::Space)),
        Some(&Act::Play)
    );
    let playing = rail.when(&[PLAYING]);
    assert_eq!(
        keys.action(playing, Chord::plain(Key::Space)),
        Some(&Act::Stop)
    );
    assert_eq!(
        keys.action(playing, Chord::plain(Key::Esc)),
        Some(&Act::Stop)
    );
    assert_eq!(
        keys.action(Now::new(CUT), Chord::ctrl('z')),
        Some(&Act::Undo)
    );
    assert_eq!(keys.action(rail, Chord::char('?')), None);
}

#[test]
fn the_sheet_keeps_the_conventions_and_never_steals_other_keys() {
    let mut sheet = Sheet::new();
    assert_eq!(sheet.key(Chord::plain(Key::Right)), Outcome::Pass);
    assert_eq!(sheet.key(Chord::plain(Key::Esc)), Outcome::Pass);
    assert_eq!(sheet.key(Chord::char('?')), Outcome::Took);
    assert!(sheet.is_open());
    assert_eq!(sheet.key(Chord::plain(Key::Right)), Outcome::Pass);
    assert_eq!(sheet.key(Chord::ctrl('z')), Outcome::Pass);
    assert_eq!(sheet.key(Chord::char('?')), Outcome::Took);
    assert!(!sheet.is_open());

    sheet.toggle();
    assert_eq!(sheet.key(Chord::char('/')), Outcome::Took);
    for c in "undo".chars() {
        assert_eq!(sheet.key(Chord::char(c)), Outcome::Took);
    }
    assert_eq!(sheet.query(), "undo");
    assert_eq!(sheet.key(Chord::plain(Key::Down)), Outcome::Pass);
    assert_eq!(sheet.key(Chord::plain(Key::Enter)), Outcome::Took);
    assert!(!sheet.typing() && sheet.query() == "undo");
    assert_eq!(sheet.key(Chord::plain(Key::Esc)), Outcome::Took);
    assert!(sheet.is_open() && sheet.query().is_empty());
    assert_eq!(sheet.key(Chord::plain(Key::Esc)), Outcome::Took);
    assert!(!sheet.is_open());
}

#[test]
fn the_sheet_puts_the_mode_first_and_searches() {
    let keys = keymap();
    let mut sheet = Sheet::new();
    let view = sheet.view(&keys, Now::new(CUT));
    let here: Vec<&str> = view.here.iter().map(|section| section.group).collect();
    assert_eq!(here, ["Player", "Edit", "Help"]);
    assert_eq!(view.elsewhere[0].group, "Rail");
    let stops = &view.here[0].rows[0];
    assert_eq!((stops.keys, stops.live), ("space or esc", false));
    assert!(view.here[2].rows.iter().all(|row| row.live));

    sheet.search("UNDO");
    let view = sheet.view(&keys, Now::new(CUT));
    assert_eq!(view.here.len(), 1);
    assert_eq!(view.here[0].rows[0].keys, "ctrl+z");
    assert!(view.elsewhere.is_empty());
    sheet.search("nothing answers this");
    assert!(sheet.view(&keys, Now::new(CUT)).is_empty());

    sheet.search("undo ".repeat(100_000));
    assert_eq!(sheet.query().chars().count(), 64);
    assert_eq!(
        sheet.view(&keys, Now::new(CUT)).here[0].rows[0].keys,
        "ctrl+z"
    );
}

#[test]
fn the_ticker_and_the_sheet_say_the_same_words() {
    let keys = keymap();
    let now = Now::new(RAIL).when(&[PLAYING]);
    let sheet = Sheet::new().view(&keys, now);
    let rows: Vec<(&str, &str)> = sheet
        .here
        .iter()
        .flat_map(|section| &section.rows)
        .map(|row| (row.keys, row.does))
        .collect();
    let ticker = keys.ticker(now);
    assert!(ticker[0].now && ticker[0].mode == RAIL);
    for hint in &ticker[0].hints {
        if hint.keys != "?" {
            assert!(
                rows.contains(&(hint.keys, hint.does)),
                "{hint} is not on the sheet"
            );
        }
    }
    assert_eq!(
        ticker[0].to_string(),
        "right keeps · space or esc stops it · space plays · ? help · left drops · ctrl+z undoes"
    );
    assert_eq!(ticker[1].mode, CUT);
    assert_eq!(ticker[1].to_string(), "space plays");
}

#[test]
fn clashes_are_found_and_overrides_are_not() {
    assert!(keymap().clashes().is_empty());
    let keys = keymap()
        .group("Mine", [bind("right", "nudges", Act::Keep).on(RAIL)])
        .group("Theirs", [bind("?", "asks", Act::Drop)]);
    let clashes = keys.clashes();
    assert_eq!(clashes.len(), 2);
    assert_eq!(clashes[0].second.does, "nudges");
    assert_eq!(clashes[1].first.does, "asks");
}

#[test]
fn chords_read_and_write_the_same_way() {
    for text in [
        "ctrl+z",
        "ctrl+shift+z",
        "shift+space",
        "shift+a",
        "?",
        "f2",
        "pgdn",
        "alt++",
    ] {
        let chord = Chord::parse(text).unwrap();
        assert_eq!(chord.to_string(), text);
    }
    assert_eq!(Chord::parse("ctrl+Z"), Chord::parse("ctrl+shift+z"));
    assert_eq!(Chord::new(Key::Char('?'), Mods::SHIFT), Chord::char('?'));
    assert_eq!(Chord::char(' '), Chord::plain(Key::Space));
    assert!(Chord::parse("ctlr+z").is_err());
}
