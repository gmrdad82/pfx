use std::collections::{BTreeMap, BTreeSet};

use pfx_input::glyph::keycap::{self, KEYCAP_WIDTHS, KeycapLabel};
use pfx_input::glyph::{BOX, Glyph, shape_of};
use pfx_input::layout::{cap_label, scancode};
use pfx_input::{Control, Family, Key, KeyLabels, LayoutSource, Us, glyph};
use pfx_text::{Face, Placement, Representation, TextEngine};

const SANS: &[u8] = pfx_text::fixture::DM_SANS;

const AZERTY: &[(Key, &str)] = &[
    (Key::Q, "a"),
    (Key::W, "z"),
    (Key::A, "q"),
    (Key::Z, "w"),
    (Key::Semicolon, "m"),
    (Key::M, ","),
    (Key::Comma, ";"),
    (Key::Period, ":"),
    (Key::Slash, "!"),
    (Key::BracketLeft, "^"),
    (Key::BracketRight, "$"),
    (Key::Quote, "ù"),
    (Key::Backquote, "²"),
    (Key::Minus, ")"),
    (Key::Equal, "="),
    (Key::Backslash, "*"),
];

const QWERTZ: &[(Key, &str)] = &[
    (Key::Y, "z"),
    (Key::Z, "y"),
    (Key::Semicolon, "ö"),
    (Key::Quote, "ä"),
    (Key::BracketLeft, "ü"),
    (Key::BracketRight, "+"),
    (Key::Minus, "ß"),
    (Key::Equal, "´"),
    (Key::Backslash, "#"),
    (Key::Backquote, "^"),
    (Key::Slash, "-"),
];

struct Fake {
    layout: u64,
    tables: BTreeMap<u64, &'static [(Key, &'static str)]>,
    reads: usize,
}

const US_ID: u64 = 0x0409_0409;
const FR_ID: u64 = 0x040c_040c;
const DE_ID: u64 = 0x0407_0407;

impl Fake {
    fn new(layout: u64) -> Self {
        Self {
            layout,
            tables: BTreeMap::from([(US_ID, &[][..]), (FR_ID, AZERTY), (DE_ID, QWERTZ)]),
            reads: 0,
        }
    }
}

impl LayoutSource for Fake {
    fn layout(&mut self) -> u64 {
        self.layout
    }

    fn label(&mut self, key: Key) -> Option<String> {
        self.reads += 1;
        let table = self.tables[&self.layout];
        if let Some((_, text)) = table.iter().find(|(k, _)| *k == key) {
            return Some((*text).to_owned());
        }
        Some(key.label().to_lowercase())
    }
}

#[test]
fn azerty_shows_its_letters_on_physical_positions() {
    let mut labels = KeyLabels::new();
    assert!(labels.refresh(&mut Fake::new(FR_ID)));
    assert_eq!(labels.layout(), Some(FR_ID));
    let expected = [
        (Key::Q, "A"),
        (Key::W, "Z"),
        (Key::A, "Q"),
        (Key::Z, "W"),
        (Key::Semicolon, "M"),
        (Key::M, ","),
        (Key::Comma, ";"),
        (Key::Period, ":"),
        (Key::Slash, "!"),
        (Key::BracketLeft, "^"),
        (Key::BracketRight, "$"),
        (Key::Quote, "Ù"),
        (Key::Backquote, "²"),
        (Key::Minus, ")"),
        (Key::Backslash, "*"),
        (Key::E, "E"),
        (Key::D, "D"),
        (Key::Digit1, "1"),
        (Key::Space, "SPACE"),
        (Key::Escape, "ESC"),
    ];
    for (key, label) in expected {
        assert_eq!(labels.label(key), label, "{key:?}");
    }
}

#[test]
fn qwertz_shows_its_letters_on_physical_positions() {
    let mut labels = KeyLabels::new();
    assert!(labels.refresh(&mut Fake::new(DE_ID)));
    let expected = [
        (Key::Y, "Z"),
        (Key::Z, "Y"),
        (Key::Semicolon, "Ö"),
        (Key::Quote, "Ä"),
        (Key::BracketLeft, "Ü"),
        (Key::BracketRight, "+"),
        (Key::Minus, "ß"),
        (Key::Equal, "´"),
        (Key::Backslash, "#"),
        (Key::Backquote, "^"),
        (Key::Slash, "-"),
        (Key::Q, "Q"),
        (Key::W, "W"),
        (Key::ShiftLeft, "SHIFT"),
    ];
    for (key, label) in expected {
        assert_eq!(labels.label(key), label, "{key:?}");
    }
}

#[test]
fn the_cache_rereads_only_when_the_layout_changes() {
    let mut labels = KeyLabels::new();
    let mut source = Fake::new(US_ID);
    assert!(!labels.refresh(&mut source));
    let first = source.reads;
    assert!(first > 0);
    assert_eq!(labels.label(Key::Q), "Q");
    for _ in 0..10 {
        assert!(!labels.refresh(&mut source));
    }
    assert_eq!(source.reads, first);
    source.layout = FR_ID;
    assert!(labels.refresh(&mut source));
    assert_eq!(labels.label(Key::Q), "A");
    assert!(source.reads > first);
    source.layout = DE_ID;
    assert!(labels.refresh(&mut source));
    assert_eq!(labels.label(Key::Q), "Q");
    assert_eq!(labels.label(Key::Y), "Z");
    source.layout = US_ID;
    assert!(labels.refresh(&mut source));
    assert_eq!(labels, {
        let mut fresh = KeyLabels::new();
        fresh.refresh(&mut Fake::new(US_ID));
        fresh
    });
}

#[test]
fn the_us_source_and_an_empty_cache_read_us() {
    let mut labels = KeyLabels::new();
    assert_eq!(labels.layout(), None);
    for key in Key::ALL {
        assert_eq!(labels.label(key), key.label());
    }
    assert!(!labels.refresh(&mut Us));
    assert_eq!(labels.layout(), Some(0));
    for key in Key::ALL {
        assert_eq!(labels.label(key), key.label());
    }
}

#[test]
fn learning_from_key_events_follows_a_layout_switch() {
    let mut labels = KeyLabels::new();
    assert!(!labels.learn(Key::E, "e"));
    assert!(labels.learn(Key::Q, "a"));
    assert!(labels.learn(Key::W, "z"));
    assert!(!labels.learn(Key::Q, "a"));
    assert!(!labels.learn(Key::Digit1, "&"));
    assert!(!labels.learn(Key::Space, " "));
    assert_eq!(labels.label(Key::Q), "A");
    assert_eq!(labels.label(Key::W), "Z");
    assert_eq!(labels.label(Key::Digit1), "1");
    assert!(labels.learn(Key::Q, "q"));
    assert_eq!(labels.label(Key::Q), "Q");
    assert_eq!(labels.label(Key::W), "W");
    assert!(labels.learn(Key::Z, "y"));
    assert!(labels.learn(Key::Minus, "ß"));
    assert_eq!(labels.label(Key::Z), "Y");
    assert_eq!(labels.label(Key::Minus), "ß");
}

#[test]
fn a_learned_label_that_disagrees_forces_a_reread() {
    let mut labels = KeyLabels::new();
    let mut source = Fake::new(FR_ID);
    labels.refresh(&mut source);
    assert!(labels.learn(Key::Q, "q"));
    assert_eq!(labels.layout(), None);
    assert_eq!(labels.label(Key::W), "W");
    source.layout = US_ID;
    labels.refresh(&mut source);
    assert_eq!(labels.layout(), Some(US_ID));
    assert_eq!(labels.label(Key::Q), "Q");
}

#[test]
fn renamed_keys_win_over_the_layout() {
    let mut labels = KeyLabels::new();
    labels.rename(Key::ShiftLeft, "Maj");
    labels.rename(Key::Escape, "Échap");
    labels.refresh(&mut Fake::new(FR_ID));
    assert_eq!(labels.label(Key::ShiftLeft), "MAJ");
    assert_eq!(labels.label(Key::Escape), "ÉCHAP");
    labels.rename(Key::ShiftLeft, "");
    assert_eq!(labels.label(Key::ShiftLeft), "SHIFT");
    labels.forget();
    assert_eq!(labels.label(Key::Q), "Q");
    assert_eq!(labels.label(Key::Escape), "ÉCHAP");
}

#[test]
fn labels_are_capitals_without_changing_length() {
    assert_eq!(cap_label("a").as_deref(), Some("A"));
    assert_eq!(cap_label("ù").as_deref(), Some("Ù"));
    assert_eq!(cap_label("ß").as_deref(), Some("ß"));
    assert_eq!(cap_label(" ").as_deref(), None);
    assert_eq!(cap_label("\u{1b}").as_deref(), None);
    assert_eq!(cap_label("Strg").as_deref(), Some("STRG"));
}

#[test]
fn layout_keys_have_distinct_scancodes() {
    let mut seen = BTreeSet::new();
    for key in Key::ALL {
        if key.follows_layout() {
            let code = scancode(key).unwrap_or_else(|| panic!("{key:?} has no scancode"));
            assert!(seen.insert(code), "{key:?} repeats {code:#x}");
        }
    }
    assert_eq!(seen.len(), 37);
    assert_eq!(scancode(Key::Q), Some(0x10));
    assert_eq!(scancode(Key::Semicolon), Some(0x27));
}

fn engine() -> (TextEngine, Face) {
    let engine = TextEngine::new(SANS).unwrap();
    (
        engine,
        Face {
            family: "DM Sans".into(),
            size: 16.0,
            line: 20.0,
            weight: 500,
            italic: false,
            spacing: 0.0,
        },
    )
}

#[test]
fn keycaps_are_blank_so_labels_need_no_atlas() {
    for key in Key::ALL {
        match glyph(Family::Generic, Control::Key(key)) {
            Glyph::Keycap(width) => assert!(KEYCAP_WIDTHS.contains(&width), "{key:?}"),
            Glyph::Arrow(_) => {}
            other => panic!("{key:?} drew {other:?}"),
        }
    }
    for width in KEYCAP_WIDTHS {
        let shape = shape_of(Glyph::Keycap(width));
        assert_eq!(shape.width, f32::from(width));
        assert_eq!(shape.height, BOX);
        assert_eq!(shape.contours.len(), 2);
    }
    assert_eq!(keycap::keycap_for(0.0), Glyph::Keycap(48));
    assert_eq!(keycap::keycap_for(28.0), Glyph::Keycap(48));
    assert_eq!(keycap::keycap_for(28.5), Glyph::Keycap(64));
    assert_eq!(keycap::keycap_for(1000.0), Glyph::Keycap(160));
}

#[test]
fn layout_labels_fit_their_caps_through_the_text_crate() {
    let (mut engine, face) = engine();
    let mut labels = KeyLabels::new();
    labels.refresh(&mut Fake::new(DE_ID));
    let umlaut = KeycapLabel::key(&mut engine, &face, &labels, Key::Semicolon).unwrap();
    assert_eq!(umlaut.text, "Ö");
    assert_eq!(umlaut.glyph, Glyph::Keycap(48));
    assert_eq!(umlaut.size, keycap::LETTER_EM);
    let z = KeycapLabel::key(&mut engine, &face, &labels, Key::Y).unwrap();
    assert_eq!(z.text, "Z");
    let space = KeycapLabel::key(&mut engine, &face, &labels, Key::Space).unwrap();
    let Glyph::Keycap(width) = space.glyph else {
        panic!("{:?}", space.glyph);
    };
    assert!(width > 48);
    let long = KeycapLabel::fit(&mut engine, &face, "LEERTASTE LEERTASTE LEERTASTE").unwrap();
    assert_eq!(long.glyph, Glyph::Keycap(160));
    assert!(long.size < keycap::WORD_EM);
    assert!(long.origin[0] >= keycap::INSET - 0.5);
    let arrow = KeycapLabel::key(&mut engine, &face, &labels, Key::Up).unwrap();
    assert!(matches!(arrow.glyph, Glyph::Arrow(_)));
    assert!(arrow.text.is_empty());

    let placement = Placement {
        scale: 2.0,
        representation: Representation::Msdf,
        origin: [0.0, 0.0],
        alpha: 1.0,
        clip: [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
        turn: [0.0; 3],
    };
    let origin = [100.0, 50.0];
    let height = 24.0;
    let k = height / BOX;
    let placed = umlaut
        .place(&mut engine, &face, [1.0; 4], origin, height, &placement)
        .unwrap();
    assert_eq!(placed.quads.len(), 1);
    let rect = placed.quads[0].quad.rect;
    let (cx, cy) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    let cap = [origin[0], origin[1], 48.0 * k, BOX * k];
    assert!(
        rect[0] >= cap[0] && rect[0] + rect[2] <= cap[0] + cap[2],
        "{rect:?} leaves {cap:?}"
    );
    assert!(
        rect[1] >= cap[1] && rect[1] + rect[3] <= cap[1] + cap[3],
        "{rect:?} leaves {cap:?}"
    );
    assert!((cx - (cap[0] + cap[2] / 2.0)).abs() < 2.0 * k, "{cx}");
    assert!((cy - (cap[1] + cap[3] / 2.0)).abs() < 5.0 * k, "{cy}");
    let none = arrow
        .place(&mut engine, &face, [1.0; 4], origin, height, &placement)
        .unwrap();
    assert!(none.quads.is_empty());
}

#[test]
fn under_a_floor_the_keycap_widens_instead_of_shrinking_the_label() {
    let (mut engine, face) = engine();
    let text = "LEERTASTE LEERTASTE LEERTASTE";
    let shrunk = KeycapLabel::fit(&mut engine, &face, text).unwrap();
    assert_eq!(shrunk.glyph, Glyph::Keycap(160));
    let floor = keycap::WORD_EM * 0.9;
    assert!(shrunk.size < floor);
    let held = KeycapLabel::fit_with_floor(&mut engine, &face, text, floor).unwrap();
    assert_eq!(held.size, floor);
    let Glyph::Keycap(width) = held.glyph else {
        panic!("{:?}", held.glyph);
    };
    assert!(width > 160, "{width}");
    assert!(
        (held.origin[0] - keycap::INSET).abs() < 1.0,
        "{:?}",
        held.origin
    );
    assert_eq!(
        KeycapLabel::fit_with_floor(&mut engine, &face, text, 0.0).unwrap(),
        shrunk
    );
    let short = KeycapLabel::fit(&mut engine, &face, "Ö").unwrap();
    assert_eq!(
        KeycapLabel::fit_with_floor(&mut engine, &face, "Ö", floor).unwrap(),
        short
    );
    let above = KeycapLabel::fit_with_floor(&mut engine, &face, "Ö", 40.0).unwrap();
    assert_eq!(above.size, keycap::LETTER_EM);
    assert!(KeycapLabel::fit_with_floor(&mut engine, &face, text, -1.0).is_err());

    let atlas = pfx_input::GlyphAtlas::build().unwrap();
    let base = *atlas.get(Glyph::Keycap(160)).unwrap();
    assert_eq!(atlas.pieces(Glyph::Keycap(160)).unwrap(), vec![base]);
    assert!(atlas.pieces(Glyph::Keycap(150)).is_none());
    let pieces = atlas.pieces(held.glyph).unwrap();
    assert!(pieces.len() >= 3);
    let first = pieces[0];
    let last = pieces[pieces.len() - 1];
    assert_eq!(first.offset[0], base.offset[0]);
    let right = last.offset[0] + last.size[0];
    let shift = f32::from(width) - 160.0;
    assert!((right - (base.offset[0] + base.size[0] + shift)).abs() < 1e-3);
    for pair in pieces.windows(2) {
        assert!((pair[0].offset[0] + pair[0].size[0] - pair[1].offset[0]).abs() < 1e-3);
    }
    for piece in &pieces {
        assert_eq!(piece.box_size, [f32::from(width), BOX]);
        assert_eq!(
            [piece.offset[1], piece.size[1]],
            [base.offset[1], base.size[1]]
        );
        assert!(piece.uv[0] >= base.uv[0] - 1e-6 && piece.uv[2] <= base.uv[2] + 1e-6);
    }
}
