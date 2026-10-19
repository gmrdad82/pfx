use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::epaint::ColorMode;
use egui::{Color32, Pos2, Shape, Ui};
use pfx_editor_shell::{App, Script, context, drive, gpu_turn, shot};
use pfx_editor_style::fixture::{self, colours};
use pfx_editor_style::theme::{Caps, Diff, Syntax, Theme};
use pfx_editor_style::tokens;
use pfx_editor_style::widgets::{
    Dotted, Header, KeyCap, Number, Pill, ScaleBar, Slider, Tabs, Tone, fraction,
};

const SIZE: [u32; 2] = [240, 64];

#[derive(Clone, Copy, Debug)]
enum Board {
    Header,
    Tabs,
    Pill,
    KeyCap,
    Number,
    Dotted,
    Slider,
    Scale,
}

const BOARDS: [Board; 8] = [
    Board::Header,
    Board::Tabs,
    Board::Pill,
    Board::KeyCap,
    Board::Number,
    Board::Dotted,
    Board::Slider,
    Board::Scale,
];

struct Shelf {
    board: Board,
    value: f32,
    other: f32,
    tab: Option<usize>,
    target: Option<Pos2>,
}

impl Shelf {
    fn new(board: Board) -> Shelf {
        Shelf {
            board,
            value: 1.25,
            other: -4.0,
            tab: Some(1),
            target: None,
        }
    }
}

impl App for Shelf {
    fn ui(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            let target = match self.board {
                Board::Header => {
                    let first = ui.add(Header::new("OU", "outliner").info("12 items"));
                    ui.add(Header::new("PR", "properties").focused(true));
                    first.rect.center()
                }
                Board::Tabs => {
                    let all = ui.add(Tabs::new(&mut self.tab, &["scene", "light", "camera"]));
                    Pos2::new(all.rect.left() + 12.0, all.rect.center().y)
                }
                Board::Pill => {
                    let mut first = None;
                    ui.horizontal(|ui| {
                        for tone in [Tone::Neutral, Tone::Ok, Tone::Busy, Tone::Danger] {
                            let pill = ui.add(Pill::new("ready", tone));
                            first.get_or_insert(pill.rect.center());
                        }
                    });
                    ui.horizontal(|ui| {
                        for tone in [Tone::Neutral, Tone::Ok, Tone::Busy, Tone::Danger] {
                            ui.add(Pill::new("live", tone).active(true));
                        }
                    });
                    first.unwrap()
                }
                Board::KeyCap => {
                    let mut first = None;
                    ui.horizontal(|ui| {
                        first = Some(ui.add(KeyCap::new("Esc").button()).rect.center());
                        ui.add(KeyCap::new("B"));
                        ui.add(KeyCap::new("1").lit(true));
                        ui.add_enabled(false, KeyCap::new("2"));
                        ui.add_enabled(false, KeyCap::new("3").button());
                    });
                    first.unwrap()
                }
                Board::Number => {
                    let first = ui.add(Number::new(&mut self.value).unit("m"));
                    ui.add(Number::new(&mut self.other).decimals(1).width(60.0));
                    first.rect.center()
                }
                Board::Dotted => {
                    let first = ui.add(Dotted::new());
                    ui.add(Dotted::new().active(true));
                    first.rect.center()
                }
                Board::Slider => {
                    let range = 0.0..=2.0;
                    let at = fraction(self.value, &range);
                    let slider = ui.add(Slider::new(&mut self.value, range).width(120.0));
                    let inner = slider.rect.shrink2(egui::Vec2::new(6.5, 0.0));
                    Pos2::new(inner.left() + at * inner.width(), slider.rect.center().y)
                }
                Board::Scale => ui.add(ScaleBar::new(0.01)).rect.center(),
            };
            self.target = Some(target);
        });
    }

    fn theme(&self) -> Theme {
        fixture::theme()
    }
}

fn target(board: Board) -> Pos2 {
    let ctx = context(&fixture::theme());
    let mut shelf = Shelf::new(board);
    drive(&ctx, SIZE, &Script::default(), |ui| shelf.ui(ui));
    shelf.target.unwrap()
}

fn states(board: Board) -> Vec<(&'static str, Script)> {
    let at = target(board);
    let mut states = vec![
        ("rest", String::new()),
        ("hover", format!("move {} {}\n", at.x, at.y)),
        ("active", format!("press {} {}\n", at.x, at.y)),
    ];
    if let Board::Number = board {
        states.push(("edit", format!("click {} {}\nframe\ntext 7\n", at.x, at.y)));
    }
    states
        .into_iter()
        .map(|(name, text)| (name, Script::parse(&text).unwrap()))
        .collect()
}

fn scratch() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/style-shots");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn every_widget_in_every_state_shoots_under_the_fixture_theme() {
    let dir = scratch();
    for board in BOARDS {
        for (state, script) in states(board) {
            let since = std::time::Instant::now();
            let mut shelf = Shelf::new(board);
            let png = shot(&mut shelf, SIZE, &script).unwrap();
            let name = format!("{board:?}-{state}.png").to_lowercase();
            std::fs::write(dir.join(name), &png).unwrap();
            gpu_turn(since);
        }
    }
}

fn loud() -> Theme {
    let mut next = 0u8;
    let mut colour = || {
        next += 1;
        Color32::from_rgb(0xFE, next * 5, 0x01)
    };
    Theme {
        name: Cow::Borrowed("loud"),
        bg: colour(),
        surface: colour(),
        card: colour(),
        card_line: colour(),
        stripe: colour(),
        selected: colour(),
        well: colour(),
        line: colour(),
        track: colour(),
        grid: colour(),
        rule: colour(),
        title: colour(),
        text: colour(),
        text2: colour(),
        muted: colour(),
        dim: colour(),
        code: colour(),
        on_accent: colour(),
        on_hot: colour(),
        accent: colour(),
        accent_bright: colour(),
        accent_deep: colour(),
        secondary: colour(),
        secondary_soft: colour(),
        hot: colour(),
        hot_dim: colour(),
        value: colour(),
        ramp: [colour(), colour(), colour(), colour(), colour()],
        ok: colour(),
        ok_tint: colour(),
        error: colour(),
        error_tint: colour(),
        warning: colour(),
        syntax: Syntax {
            key: colour(),
            table: colour(),
            string: colour(),
            number: colour(),
            boolean: colour(),
            date: colour(),
            comment: colour(),
            punctuation: colour(),
            plain: colour(),
        },
        diff: Diff {
            add: colour(),
            del: colour(),
            change: colour(),
        },
        ..fixture::theme()
    }
}

fn painted(shape: &Shape, into: &mut BTreeSet<[u8; 4]>) {
    let mut add = |colour: Color32| {
        if colour != Color32::TRANSPARENT {
            into.insert(colour.to_array());
        }
    };
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                painted(shape, into);
            }
        }
        Shape::Rect(rect) => {
            add(rect.fill);
            add(rect.stroke.color);
        }
        Shape::Circle(circle) => {
            add(circle.fill);
            add(circle.stroke.color);
        }
        Shape::LineSegment { stroke, .. } => add(stroke.color),
        Shape::Path(path) => {
            add(path.fill);
            if let ColorMode::Solid(colour) = path.stroke.color {
                add(colour);
            }
        }
        Shape::Text(text) => {
            for section in &text.galley.job.sections {
                add(section.format.color);
            }
            add(text.underline.color);
            if let Some(colour) = text.override_text_color {
                add(colour);
            }
        }
        Shape::Noop => {}
        other => panic!("a widget painted an unexpected shape: {other:?}"),
    }
}

fn shapes(theme: &Theme, board: Board, script: &Script) -> Vec<Shape> {
    let ctx = context(theme);
    let mut shelf = Shelf::new(board);
    let output = drive(&ctx, SIZE, script, |ui| shelf.ui(ui));
    let families = fixture::assert_faces(&ctx, theme, &output.shapes);
    assert!(
        families > 0 || matches!(board, Board::Dotted | Board::Slider),
        "{board:?}"
    );
    output
        .shapes
        .into_iter()
        .map(|clipped| clipped.shape)
        .collect()
}

fn set(theme: &Theme) -> BTreeSet<[u8; 4]> {
    colours(theme).iter().map(|c| c.to_array()).collect()
}

fn reaches_every_widget(theme: &Theme, foreign: &BTreeSet<[u8; 4]>) {
    let ours = set(theme);
    assert!(ours.is_disjoint(foreign), "{}", theme.name);
    for board in BOARDS {
        for (state, script) in states(board) {
            let mut seen = BTreeSet::new();
            for shape in shapes(theme, board, &script) {
                painted(&shape, &mut seen);
            }
            let opaque: BTreeSet<[u8; 4]> = seen.iter().copied().filter(|c| c[3] == 255).collect();
            assert!(
                seen.is_disjoint(foreign),
                "{} {board:?} {state}: {:?}",
                theme.name,
                seen.intersection(foreign).collect::<Vec<_>>()
            );
            assert!(
                opaque.is_subset(&ours),
                "{} {board:?} {state}: {:?}",
                theme.name,
                opaque.difference(&ours).collect::<Vec<_>>()
            );
            assert!(
                opaque.len() >= 3,
                "{} {board:?} {state}: {opaque:?}",
                theme.name
            );
        }
    }
}

#[test]
fn a_second_theme_reaches_every_widget_in_every_state() {
    let theme = loud();
    assert_eq!(set(&theme).len(), colours(&theme).len());
    reaches_every_widget(&theme, &set(&fixture::theme()));
    let game = tokens::game();
    let foreign: BTreeSet<[u8; 4]> = set(&fixture::theme())
        .union(&set(&tokens::theme()))
        .copied()
        .collect();
    reaches_every_widget(&game, &foreign);
}

#[test]
fn each_state_paints_its_own_roles() {
    let theme = loud();
    let roles = |board: Board, state: &str| {
        let (_, script) = states(board)
            .into_iter()
            .find(|(name, _)| *name == state)
            .unwrap();
        let mut seen = BTreeSet::new();
        for shape in shapes(&theme, board, &script) {
            painted(&shape, &mut seen);
        }
        seen
    };
    let has = |seen: &BTreeSet<[u8; 4]>, colour: Color32| seen.contains(&colour.to_array());
    assert!(has(&roles(Board::Dotted, "rest"), theme.track));
    assert!(has(&roles(Board::Dotted, "rest"), theme.hot));
    assert!(has(&roles(Board::Header, "rest"), theme.hot));
    assert!(has(&roles(Board::Header, "rest"), theme.on_hot));
    assert!(has(&roles(Board::Header, "rest"), theme.card));
    assert!(!has(&roles(Board::Header, "rest"), theme.accent));
    assert!(has(&roles(Board::KeyCap, "hover"), theme.accent_deep));
    assert!(has(&roles(Board::KeyCap, "rest"), theme.accent_bright));
    assert!(has(&roles(Board::Tabs, "rest"), theme.hot));
    assert!(has(&roles(Board::Tabs, "hover"), theme.accent_deep));
    assert!(has(&roles(Board::Number, "rest"), theme.well));
    assert!(has(&roles(Board::Number, "rest"), theme.code));
    assert!(!has(&roles(Board::Number, "rest"), theme.hot));
    assert!(has(&roles(Board::Number, "active"), theme.hot));
    assert!(has(&roles(Board::Slider, "rest"), theme.hot));
    assert!(has(&roles(Board::Slider, "rest"), theme.track));
    assert!(!has(&roles(Board::Slider, "rest"), theme.accent));
    assert!(has(&roles(Board::Slider, "hover"), theme.accent));
    assert!(has(&roles(Board::Scale, "rest"), theme.rule));
    let pills = roles(Board::Pill, "rest");
    for colour in [
        theme.surface,
        theme.text2,
        theme.ok,
        theme.ok_tint,
        theme.hot,
        theme.hot_dim,
        theme.error,
        theme.error_tint,
    ] {
        assert!(has(&pills, colour), "{colour:?}");
    }
}

fn texts(theme: &Theme, board: Board) -> Vec<(String, f32, f32)> {
    let mut found = Vec::new();
    for shape in shapes(theme, board, &Script::default()) {
        if let Shape::Text(text) = shape {
            let format = &text.galley.job.sections[0].format;
            found.push((
                text.galley.job.text.clone(),
                format.extra_letter_spacing,
                text.galley.size().x,
            ));
        }
    }
    found
}

fn find<'a>(texts: &'a [(String, f32, f32)], text: &str) -> &'a (String, f32, f32) {
    texts
        .iter()
        .find(|(shown, ..)| shown == text)
        .unwrap_or_else(|| panic!("{text} not in {texts:?}"))
}

#[test]
fn titles_upper_and_track_when_the_theme_says_so() {
    let tracked = Theme {
        titles: Caps {
            upper: true,
            tracking: 0.3,
            weight: None,
        },
        ..fixture::theme()
    };
    let wide = texts(&tracked, Board::Header);
    let (_, spacing, width) = find(&wide, "OUTLINER");
    assert_eq!(*spacing, 0.3 * tracked.size_small);
    let upper = Theme {
        titles: Caps {
            upper: true,
            tracking: 0.0,
            weight: None,
        },
        ..fixture::theme()
    };
    let narrow = texts(&upper, Board::Header);
    let (_, plain, plain_width) = find(&narrow, "OUTLINER");
    assert_eq!(*plain, 0.0);
    assert!(
        *width > *plain_width + 7.0 * spacing - 0.5,
        "{width} {plain_width}"
    );
    let lower = Theme {
        titles: Caps::PLAIN,
        ..fixture::theme()
    };
    find(&texts(&lower, Board::Header), "outliner");
    find(&texts(&lower, Board::Header), "12 items");
}

#[test]
fn data_labels_upper_and_track_when_the_theme_says_so() {
    let labels = Theme {
        labels: Caps {
            upper: true,
            tracking: 0.2,
            weight: None,
        },
        ..fixture::theme()
    };
    let (_, spacing, _) = find(&texts(&labels, Board::Header), "12 ITEMS").clone();
    assert_eq!(spacing, 0.2 * labels.size_code);
    find(&texts(&labels, Board::Pill), "READY");
    find(&texts(&labels, Board::Number), "m");
    find(&texts(&fixture::theme(), Board::Pill), "ready");
    find(&texts(&fixture::theme(), Board::Number), "m");
}
