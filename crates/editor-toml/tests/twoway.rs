use std::path::Path;

use egui::Context;
use pfx_editor_doc::toml_edit::value;
use pfx_editor_doc::{Changed, Files, History, TomlPatch, parse_path};
use pfx_editor_shell::{Script, context, drive};
use pfx_editor_style::fixture;
use pfx_editor_toml::{NoCheck, Notice, Source, Table};

const FILE: &str = "scene.toml";
const SCENE: &str = "format = 1\n\n[[object]]\nname = \"a\"\nat = 1.0\n\n[[object]]\nname = \"b\"\nat = 2.0\n\n[[object]]\nname = \"c\"\nat = 3.0\n\n[materials.x]\nshine = 0.5\n";
const SETTLE: usize = 12;

struct Editor {
    ctx: Context,
    files: Files,
    history: History,
    source: Source,
    selected: Vec<(usize, Table)>,
    changed: Vec<Changed>,
}

impl Editor {
    fn new() -> Editor {
        let mut files = Files::new();
        files.open(FILE, SCENE);
        Editor {
            ctx: context(&fixture::theme()),
            files,
            history: History::default(),
            source: Source::new("source", FILE),
            selected: Vec::new(),
            changed: Vec::new(),
        }
    }

    fn run(&mut self, script: &str) {
        let script = Script::parse(script).unwrap();
        let Editor {
            ctx,
            files,
            history,
            source,
            selected,
            changed,
        } = self;
        drive(ctx, [480, 360], &script, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut select = |at: usize, table: &Table| selected.push((at, table.clone()));
                if let Some(one) = source.show(ui, files, history, &NoCheck, &mut select) {
                    changed.push(one);
                }
            });
        });
    }

    fn settle(&mut self, script: &str) {
        self.run(&format!("{script}\n{}", "frame\n".repeat(SETTLE)));
    }

    fn patch(&mut self, key: &str, to: i64) {
        let patch =
            TomlPatch::set(FILE, parse_path(key).unwrap(), value(to)).labelled(key.to_string());
        self.history.apply(&mut self.files, patch).unwrap();
    }

    fn text(&self) -> &str {
        self.files.text(Path::new(FILE)).unwrap()
    }

    fn click(&mut self, choice: usize) {
        let at = self.source.choices()[choice].center();
        self.run(&format!("click {} {}\nframe", at.x, at.y));
    }

    fn shows_the_file(&self) {
        assert_eq!(self.source.text(), self.text());
        assert!(!self.source.unapplied());
    }
}

#[test]
fn an_inspector_patch_shows_in_the_panel_and_the_caret_stays_on_its_character() {
    let mut editor = Editor::new();
    let at = SCENE.find("name = \"c\"").unwrap();
    editor.source.jump(at);
    editor.run("frame");
    editor.patch("format", 12);
    editor.run("frame");
    editor.shows_the_file();
    assert!(editor.source.text().starts_with("format = 12\n"));
    assert_eq!(editor.source.caret(), at + 1);
    assert!(editor.source.text()[editor.source.caret()..].starts_with("name = \"c\""));
    editor.settle("text #");
    assert!(editor.text().contains("\n#name = \"c\"\n"));
    editor.shows_the_file();
}

#[test]
fn typing_then_a_patch_unwind_in_the_order_they_happened() {
    let mut editor = Editor::new();
    editor.source.jump(SCENE.find("2.0").unwrap());
    editor.run("frame");
    editor.settle("text 1");
    let typed = SCENE.replace("at = 2.0", "at = 12.0");
    assert_eq!(editor.text(), typed);
    editor.patch("format", 2);
    let patched = typed.replace("format = 1", "format = 2");
    editor.run("frame");
    assert_eq!(editor.source.text(), patched);
    assert_eq!(editor.history.undo_labels(), vec!["typing", "format"]);
    editor.run("key ctrl+Z\nframe");
    assert_eq!(editor.text(), typed);
    editor.shows_the_file();
    editor.run("key ctrl+Z\nframe");
    assert_eq!(editor.text(), SCENE);
    editor.shows_the_file();
    assert_eq!(editor.source.caret(), SCENE.find("2.0").unwrap());
    assert_eq!(editor.history.redo_labels(), vec!["format", "typing"]);
    editor.run("key ctrl+shift+Z\nframe\nkey ctrl+shift+Z\nframe");
    assert_eq!(editor.text(), patched);
    editor.shows_the_file();
    let labels: Vec<&str> = editor.changed.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["typing", "format", "typing", "typing", "format"]
    );
}

#[test]
fn ctrl_z_in_the_panel_undoes_a_patch_made_outside_it() {
    let mut editor = Editor::new();
    editor.source.jump(0);
    editor.run("frame");
    editor.patch("object[1].at", 7);
    editor.run("frame");
    assert!(editor.source.text().contains("at = 7\n"));
    editor.run("key ctrl+Z\nframe");
    assert_eq!(editor.text(), SCENE);
    editor.shows_the_file();
    assert_eq!(editor.history.undo_len(), 0);
    assert_eq!(editor.history.redo_labels(), vec!["object[1].at"]);
}

fn broken(editor: &mut Editor) -> String {
    let end = SCENE.find("at = 1.0").unwrap() + "at = 1.0".len();
    editor.source.jump(end);
    editor.run("frame");
    editor.settle("text x");
    let mine = SCENE.replace("at = 1.0", "at = 1.0x");
    assert_eq!(editor.source.text(), mine);
    assert!(editor.source.unapplied());
    assert_eq!(editor.text(), SCENE);
    editor.patch("format", 3);
    editor.run("frame");
    assert_eq!(editor.source.notice(), Some(&Notice::Underneath));
    assert_eq!(editor.source.text(), mine);
    assert_eq!(editor.source.choices().len(), 2);
    mine
}

#[test]
fn unapplied_text_under_a_patch_asks_and_take_theirs_takes_the_file() {
    let mut editor = Editor::new();
    broken(&mut editor);
    editor.run("key ctrl+Z\nframe");
    assert_eq!(editor.text(), SCENE.replace("format = 1", "format = 3"));
    assert_eq!(editor.source.notice(), Some(&Notice::Underneath));
    editor.click(0);
    assert_eq!(editor.source.notice(), None);
    assert_eq!(
        editor.source.text(),
        SCENE.replace("format = 1", "format = 3")
    );
    editor.shows_the_file();
    assert_eq!(editor.history.undo_labels(), vec!["format"]);
}

#[test]
fn unapplied_text_under_a_patch_asks_and_keep_mine_keeps_every_keystroke() {
    let mut editor = Editor::new();
    let mine = broken(&mut editor);
    editor.click(1);
    assert_eq!(editor.source.notice(), None);
    assert_eq!(editor.source.text(), mine);
    assert_eq!(
        editor.source.base(),
        SCENE.replace("format = 1", "format = 3")
    );
    assert!(editor.source.unapplied());
    editor.ctx = context(&fixture::theme());
    editor.source.jump(editor.source.caret());
    editor.run("frame");
    editor.settle("key Backspace");
    assert_eq!(editor.text(), SCENE);
    editor.shows_the_file();
    assert_eq!(editor.history.undo_labels(), vec!["format", "typing"]);
    editor.run("key ctrl+Z\nframe");
    assert_eq!(editor.text(), SCENE.replace("format = 1", "format = 3"));
    editor.shows_the_file();
}

#[test]
fn moving_the_caret_into_the_third_object_reports_object_three() {
    let mut editor = Editor::new();
    editor.source.jump(SCENE.find("name = \"a\"").unwrap());
    editor.run("frame");
    assert!(editor.selected.is_empty());
    assert_eq!(editor.source.table().unwrap().index, Some(0));
    editor.run(&"key ArrowDown\nframe\n".repeat(7));
    let reported: Vec<Option<usize>> = editor.selected.iter().map(|(_, t)| t.index).collect();
    assert_eq!(reported, vec![Some(1), Some(2)]);
    let (at, table) = editor.selected.last().unwrap().clone();
    assert_eq!(table.path, vec!["object"]);
    assert_eq!(at, editor.source.caret());
    let doc = editor.files.doc(Path::new(FILE)).unwrap();
    let entry = &doc["object"].as_array_of_tables().unwrap().get(2).unwrap()["name"];
    assert_eq!(entry.as_str(), Some("c"));
    editor.run(&"key ArrowDown\nframe\n".repeat(4));
    let last = &editor.selected.last().unwrap().1;
    assert_eq!(
        (last.path.clone(), last.index),
        (vec!["materials".to_string(), "x".to_string()], None)
    );
}

#[test]
fn ctrl_slash_on_three_lines_comments_them_as_one_step_and_undo_restores_the_text() {
    let mut editor = Editor::new();
    editor.source.jump(SCENE.find("[[object]]").unwrap());
    editor.run("frame");
    editor.run("key shift+ArrowDown\nkey shift+ArrowDown\nkey shift+ArrowDown\nframe");
    editor.run("key ctrl+/\nframe");
    let commented = SCENE.replacen(
        "[[object]]\nname = \"a\"\nat = 1.0\n",
        "# [[object]]\n# name = \"a\"\n# at = 1.0\n",
        1,
    );
    assert_eq!(editor.text(), commented);
    editor.shows_the_file();
    assert_eq!(editor.history.undo_labels(), vec!["comment"]);
    editor.run("key ctrl+Z\nframe");
    assert_eq!(editor.text(), SCENE);
    editor.shows_the_file();
    assert!(editor.history.can_redo());
}
