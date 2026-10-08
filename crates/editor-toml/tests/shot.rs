use std::path::{Path, PathBuf};

use std::collections::HashSet;

use egui::epaint::ClippedShape;
use egui::{Color32, Context, Shape};
use pfx_editor_doc::toml_edit::DocumentMut;
use pfx_editor_doc::{Files, History};
use pfx_editor_shell::{App, Script, context, drive, gpu_turn, shot};
use pfx_editor_style::{Theme, fixture, theme};
use pfx_editor_toml::{Diagnostic, Head, NoCheck, NoHead, Opener, Source};

const FILE: &str = "scene.toml";
const SCENE: &str = "# a small scene\nformat = 1\n\n[object.lamp]\nname = \"lamp\"\nat = [0.0, 1.5, -2.0]\nlit = true\n\n[object.box]\nsize = [1, 2 3]\ncolor = { r = 0.8, g = 0.2, b = 0.1 }\n\n[[light]]\nkind = 'sun'\npower = 3.5e2\n";
const SIZE: [u32; 2] = [480, 300];

struct Committed;

impl Head for Committed {
    fn show(&self, _: &Path) -> Result<String, NoHead> {
        Ok(SCENE.to_string())
    }
}

struct Editor;

impl Opener for Editor {
    fn command(&self) -> Option<String> {
        Some("editor".into())
    }

    fn open(&self, _: &str, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

fn source() -> Source {
    Source::new("source", FILE)
        .with_head(Committed)
        .with_opener(Editor)
}

struct Panel {
    source: Source,
    files: Files,
    history: History,
    applied: u32,
    theme: Theme,
}

impl Panel {
    fn new() -> Panel {
        Panel::themed(fixture::theme())
    }

    fn themed(theme: Theme) -> Panel {
        let mut files = Files::new();
        files.open(FILE, SCENE);
        let mut source = source();
        source.sync(&files, &NoCheck);
        let start = SCENE.find("[object.box]").unwrap();
        let end = SCENE.find("\n\n[[light]]").unwrap();
        source.reveal(start..end);
        Panel {
            source,
            files,
            history: History::default(),
            applied: 0,
            theme,
        }
    }

    fn diffed(theme: Theme) -> Panel {
        let mut panel = Panel::themed(theme);
        panel.source = source();
        panel.source.sync(&panel.files, &NoCheck);
        let text = SCENE
            .replace("format = 1", "format = 2")
            .replace("lit = true\n", "")
            .replace("power = 3.5e2\n", "power = 3.5e2\ntilt = 12\n");
        panel.source.edit(text, 0.0);
        panel.source.set_diff(true);
        panel
    }

    fn doc(&self) -> Option<&DocumentMut> {
        self.files.doc(Path::new(FILE))
    }
}

impl App for Panel {
    fn ui(&mut self, ctx: &Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let Panel {
                source,
                files,
                history,
                ..
            } = self;
            if source
                .show(ui, files, history, &NoCheck, &mut |_, _| {})
                .is_some()
            {
                self.applied += 1;
            }
        });
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }
}

fn colours(theme: &Theme) -> HashSet<Color32> {
    fixture::colours(theme).into_iter().collect()
}

fn loud() -> Theme {
    let mut theme = fixture::theme();
    theme.name = "loud".into();
    let mut next = 0u8;
    let mut take = || {
        next += 7;
        Color32::from_rgb(
            next.wrapping_mul(3),
            255 - next,
            next.wrapping_mul(5) ^ 0x5A,
        )
    };
    theme.bg = take();
    theme.surface = take();
    theme.well = take();
    theme.line = take();
    theme.track = take();
    theme.text = take();
    theme.text2 = take();
    theme.muted = take();
    theme.dim = take();
    theme.code = take();
    theme.on_accent = take();
    theme.accent = take();
    theme.accent_bright = take();
    theme.accent_deep = take();
    theme.secondary = take();
    theme.secondary_soft = take();
    theme.hot = take();
    theme.hot_dim = take();
    theme.ok = take();
    theme.ok_tint = take();
    theme.error = take();
    theme.error_tint = take();
    theme.warning = take();
    theme.syntax.key = take();
    theme.syntax.table = take();
    theme.syntax.string = take();
    theme.syntax.number = take();
    theme.syntax.boolean = take();
    theme.syntax.date = take();
    theme.syntax.comment = take();
    theme.syntax.punctuation = take();
    theme.syntax.plain = take();
    theme.diff.add = take();
    theme.diff.del = take();
    theme.diff.change = take();
    theme
}

fn painted(shapes: &[ClippedShape], into: &mut Vec<Color32>) {
    for clipped in shapes {
        shape(&clipped.shape, into);
    }
}

fn shape(shape: &Shape, into: &mut Vec<Color32>) {
    match shape {
        Shape::Vec(inner) => inner.iter().for_each(|shape| self::shape(shape, into)),
        Shape::Rect(rect) => {
            into.push(rect.fill);
            into.push(rect.stroke.color);
        }
        Shape::LineSegment { stroke, .. } => into.push(stroke.color),
        Shape::Path(path) => {
            into.push(path.fill);
            if let egui::epaint::ColorMode::Solid(color) = path.stroke.color {
                into.push(color);
            }
        }
        Shape::Text(text) => {
            for section in &text.galley.job.sections {
                let color = section.format.color;
                into.push(if color == Color32::PLACEHOLDER {
                    text.override_text_color.unwrap_or(text.fallback_color)
                } else {
                    text.override_text_color.unwrap_or(color)
                });
            }
        }
        _ => {}
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/toml-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn problem(panel: &Panel) -> &Diagnostic {
    &panel.source.problems()[0]
}

#[test]
fn the_panel_over_a_broken_scene_lists_its_one_problem() {
    let ctx = context(&fixture::theme());
    let mut panel = Panel::new();
    drive(&ctx, SIZE, &Script::default(), |ctx| panel.ui(ctx));
    assert!(panel.doc().is_none());
    assert_eq!(panel.source.problems().len(), 1);
    assert_eq!(problem(&panel).line, 10);
    assert_eq!(panel.source.text(), SCENE);
    assert_eq!(panel.applied, 0);
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn a_source_shot_underlines_the_broken_line_and_lists_it() {
    let since = std::time::Instant::now();
    let mut first = Panel::new();
    let one = shot(&mut first, SIZE, &Script::default()).unwrap();
    let mut second = Panel::new();
    let two = shot(&mut second, SIZE, &Script::default()).unwrap();
    assert_eq!(one, two);
    assert_eq!(problem(&first).line, 10);
    std::fs::write(scratch("source.png"), &one).unwrap();
    gpu_turn(since);
}

#[test]
fn a_panel_paints_only_the_installed_themes_colours() {
    let theme = loud();
    let allowed = colours(&theme);
    assert!(allowed.is_disjoint(&colours(&fixture::theme())));
    let ctx = context(&theme);
    let mut panel = Panel::themed(theme.clone());
    let output = drive(&ctx, SIZE, &Script::default(), |ctx| panel.ui(ctx));
    let mut seen = Vec::new();
    painted(&output.shapes, &mut seen);
    assert!(seen.contains(&theme.well));
    assert!(seen.contains(&theme.syntax.table));
    assert!(seen.contains(&theme.syntax.key));
    assert!(seen.contains(&theme.error));
    assert!(seen.contains(&theme.dim));
    assert!(seen.contains(&theme.accent_deep));
    let strays: Vec<_> = seen
        .iter()
        .filter(|color| **color != Color32::TRANSPARENT && !allowed.contains(color))
        .collect();
    assert!(strays.is_empty(), "{strays:?}");
    assert!(fixture::assert_faces(&ctx, &theme, &output.shapes) >= 3);
}

#[test]
fn a_recoloured_theme_recolours_the_same_panel() {
    let mut panel = Panel::new();
    let ctx = context(&fixture::theme());
    let mut before = Vec::new();
    let output = drive(&ctx, SIZE, &Script::default(), |ctx| panel.ui(ctx));
    painted(&output.shapes, &mut before);
    theme::apply_theme(&ctx, &loud());
    let mut again = Vec::new();
    let output = drive(&ctx, SIZE, &Script::default(), |ctx| panel.ui(ctx));
    painted(&output.shapes, &mut again);
    assert!(before.contains(&fixture::theme().syntax.key));
    assert!(again.contains(&loud().syntax.key));
    assert!(!again.contains(&fixture::theme().syntax.key));
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn a_source_shot_under_another_theme_differs_and_repeats() {
    let since = std::time::Instant::now();
    let mut first = Panel::themed(loud());
    let one = shot(&mut first, SIZE, &Script::default()).unwrap();
    let mut second = Panel::themed(loud());
    let two = shot(&mut second, SIZE, &Script::default()).unwrap();
    assert_eq!(one, two);
    std::fs::write(scratch("source-loud.png"), &one).unwrap();
    let mut plain = Panel::new();
    let plain = shot(&mut plain, SIZE, &Script::default()).unwrap();
    assert_ne!(one, plain);
    gpu_turn(since);
}

#[test]
fn a_diff_view_paints_only_the_installed_themes_colours() {
    let theme = loud();
    let allowed = colours(&theme);
    let ctx = context(&theme);
    let mut panel = Panel::diffed(theme.clone());
    let output = drive(&ctx, SIZE, &Script::default(), |ctx| panel.ui(ctx));
    let mut seen = Vec::new();
    painted(&output.shapes, &mut seen);
    assert!(seen.contains(&theme.diff.add));
    assert!(seen.contains(&theme.diff.del));
    assert!(seen.contains(&theme.diff.change));
    assert!(seen.contains(&theme.dim));
    assert!(seen.contains(&theme.well));
    let strays: Vec<_> = seen
        .iter()
        .filter(|color| **color != Color32::TRANSPARENT && !allowed.contains(color))
        .collect();
    assert!(strays.is_empty(), "{strays:?}");
    assert!(fixture::assert_faces(&ctx, &theme, &output.shapes) >= 3);
    assert_eq!(panel.source.diff().unwrap().hunks().len(), 3);
}

#[test]
#[ignore = "GPU: pgpu run --class interactive --as pfx -- cargo test -- --ignored"]
fn a_diff_shot_draws_each_hunk_in_its_colour_and_repeats() {
    let since = std::time::Instant::now();
    let size = [520, 420];
    let mut first = Panel::diffed(fixture::theme());
    let one = shot(&mut first, size, &Script::default()).unwrap();
    let mut second = Panel::diffed(fixture::theme());
    let two = shot(&mut second, size, &Script::default()).unwrap();
    assert_eq!(one, two);
    std::fs::write(scratch("diff.png"), &one).unwrap();
    let mut plain = Panel::new();
    let plain = shot(&mut plain, size, &Script::default()).unwrap();
    assert_ne!(one, plain);
    gpu_turn(since);
}
