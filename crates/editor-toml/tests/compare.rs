use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::Context;
use pfx_editor_doc::{Changed, Files, History};
use pfx_editor_shell::{Script, context, drive};
use pfx_editor_style::fixture;
use pfx_editor_toml::diff::REVERT;
use pfx_editor_toml::{Basis, Head, Mark, NO_EDITOR, NoCheck, NoHead, Notice, Opener, Source};

const SAVED: &str = "format = 1\n\n[object.a]\nname = \"a\"\nat = 1.0\n\n[object.b]\nname = \"b\"\nat = 2.0\n\n[object.c]\nname = \"c\"\nat = 3.0\n";
const SIZE: [u32; 2] = [520, 640];

struct Fixed(Result<String, NoHead>);

impl Head for Fixed {
    fn show(&self, _: &Path) -> Result<String, NoHead> {
        self.0.clone()
    }
}

type Log = Arc<Mutex<Vec<(String, PathBuf)>>>;

struct Spy {
    command: Option<String>,
    log: Log,
}

impl Opener for Spy {
    fn command(&self) -> Option<String> {
        self.command.clone()
    }

    fn open(&self, command: &str, file: &Path) -> io::Result<()> {
        self.log
            .lock()
            .unwrap()
            .push((command.to_string(), file.to_path_buf()));
        Ok(())
    }
}

struct Rig {
    ctx: Context,
    files: Files,
    history: History,
    source: Source,
    file: PathBuf,
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/compare-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

impl Rig {
    fn new(name: &str, head: Result<String, NoHead>) -> Rig {
        let file = scratch(name);
        std::fs::write(&file, SAVED).unwrap();
        let mut files = Files::new();
        files.load(&file).unwrap();
        let mut source = Source::new("source", &file).with_head(Fixed(head));
        source.sync(&files, &NoCheck);
        Rig {
            ctx: context(&fixture::theme()),
            files,
            history: History::default(),
            source,
            file,
        }
    }

    fn type_in(&mut self, text: &str, now: f64) -> Option<Changed> {
        self.source.edit(text, now);
        self.source
            .tick(now + 1.0, &mut self.files, &mut self.history, &NoCheck)
    }

    fn text(&self) -> &str {
        self.files.text(&self.file).unwrap()
    }

    fn frames(&mut self, script: &str) {
        let script = Script::parse(script).unwrap();
        let Rig {
            ctx,
            files,
            history,
            source,
            ..
        } = self;
        drive(ctx, SIZE, &script, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                source.show(ui, files, history, &NoCheck, &mut |_, _| {});
            });
        });
    }
}

fn edited() -> String {
    SAVED
        .replace("at = 1.0", "at = 1.5")
        .replace("name = \"b\"\n", "")
        .replace("at = 3.0\n", "at = 3.0\ntag = 9\n")
}

#[test]
fn the_diff_against_the_saved_file_follows_the_buffer_and_a_save() {
    let mut rig = Rig::new("saved.toml", Err(NoHead::NoGit));
    rig.source.set_diff(true);
    assert!(rig.source.diff().unwrap().is_empty());
    let text = edited();
    assert!(rig.type_in(&text, 0.0).is_some());
    let marks: Vec<Mark> = rig
        .source
        .diff()
        .unwrap()
        .hunks()
        .iter()
        .map(|hunk| hunk.mark)
        .collect();
    assert_eq!(marks, vec![Mark::Change, Mark::Del, Mark::Add]);
    rig.files.save(&rig.file).unwrap();
    rig.frames("frame");
    assert!(rig.source.diff().unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(&rig.file).unwrap(), text);
}

#[test]
fn the_diff_against_head_uses_the_text_the_seam_gives() {
    let head = SAVED.replace("format = 1", "format = 0");
    let mut rig = Rig::new("head.toml", Ok(head.clone()));
    rig.source.set_diff(true);
    rig.source.set_basis(Basis::Head);
    assert_eq!(rig.source.basis(), Basis::Head);
    let diff = rig.source.diff().unwrap().clone();
    let [hunk] = diff.hunks() else {
        panic!("{:?}", diff.hunks())
    };
    assert_eq!(hunk.mark, Mark::Change);
    assert_eq!(&head[hunk.old.clone()], "format = 0\n");
    rig.source.set_basis(Basis::Saved);
    assert!(rig.source.diff().unwrap().is_empty());
}

#[test]
fn a_hunk_revert_is_one_undoable_splice() {
    let mut rig = Rig::new("revert.toml", Err(NoHead::NoGit));
    let text = edited();
    rig.type_in(&text, 0.0);
    rig.source.set_diff(true);
    let before = rig.history.undo_len();
    let Rig {
        files,
        history,
        source,
        file,
        ..
    } = &mut rig;
    let changed = source
        .revert_hunk(1, 5.0, files, history, &NoCheck)
        .expect("applied");
    assert_eq!(changed.label, REVERT);
    assert_eq!(history.undo_len(), before + 1);
    assert_eq!(source.text(), files.text(file).unwrap());
    let wanted = SAVED
        .replace("at = 1.0", "at = 1.5")
        .replace("at = 3.0\n", "at = 3.0\ntag = 9\n");
    assert_eq!(source.text(), wanted);
    assert_eq!(source.diff().unwrap().hunks().len(), 2);
    assert!(source.undo(6.0, files, history, &NoCheck).is_some());
    assert_eq!(source.text(), text);
    assert_eq!(rig.text(), text);
    assert_eq!(rig.source.diff().unwrap().hunks().len(), 3);
}

#[test]
fn a_revert_button_click_reverts_that_hunk_and_ctrl_z_brings_it_back() {
    let mut rig = Rig::new("button.toml", Err(NoHead::NoGit));
    let text = edited();
    rig.type_in(&text, 0.0);
    rig.source.set_diff(true);
    rig.frames("frame");
    let reverts = rig.source.reverts();
    assert_eq!(reverts.len(), 3);
    let at = reverts[0].center();
    rig.frames(&format!("click {} {}\nframe", at.x, at.y));
    assert_eq!(rig.text(), text.replace("at = 1.5", "at = 1.0"));
    assert_eq!(rig.source.text(), rig.text());
    rig.frames("key ctrl+Z\nframe");
    assert_eq!(rig.text(), text);
}

#[test]
fn without_git_the_head_toggle_says_so() {
    let mut rig = Rig::new("nogit.toml", Err(NoHead::NoGit));
    rig.source.set_diff(true);
    assert_eq!(rig.source.head_note().unwrap(), "git is not on PATH");
    rig.frames("frame");
    let head = rig.source.bar()[2].center();
    rig.frames(&format!("click {} {}\nframe", head.x, head.y));
    assert_eq!(rig.source.basis(), Basis::Saved);
    rig.source.set_basis(Basis::Head);
    assert!(rig.source.diff().is_none());
    rig.frames("frame");
    let mut outside = Rig::new("nowhere.toml", Err(NoHead::NotRepo));
    outside.source.set_diff(true);
    assert!(outside.source.head_note().unwrap().contains("repository"));
}

#[test]
fn the_diff_is_computed_only_when_the_text_or_the_base_changes() {
    let mut rig = Rig::new("count.toml", Ok(SAVED.replace("a", "x")));
    rig.source.set_diff(true);
    rig.frames("frame\nframe\nframe");
    assert_eq!(rig.source.diffs(), 1);
    rig.frames("frame\nframe");
    assert_eq!(rig.source.diffs(), 1);
    rig.type_in(&edited(), 0.0);
    rig.frames("frame\nframe");
    assert_eq!(rig.source.diffs(), 2);
    rig.source.set_basis(Basis::Head);
    rig.frames("frame\nframe");
    assert_eq!(rig.source.diffs(), 3);
}

fn spied(command: Option<&str>) -> (Rig, Log) {
    let mut rig = Rig::new("open.toml", Err(NoHead::NoGit));
    let log = Log::default();
    let spy = Spy {
        command: command.map(str::to_string),
        log: log.clone(),
    };
    let source = Source::new("source", &rig.file)
        .with_head(Fixed(Err(NoHead::NoGit)))
        .with_opener(spy);
    rig.source = source;
    rig.source.sync(&rig.files, &NoCheck);
    (rig, log)
}

#[test]
fn open_in_editor_runs_the_command_on_the_file() {
    let (mut rig, log) = spied(Some("code --wait"));
    assert!(rig.source.open_note().is_none());
    assert!(rig.source.open_in_editor());
    assert_eq!(
        log.lock().unwrap().clone(),
        vec![("code --wait".to_string(), rig.file.clone())]
    );
    rig.frames("frame");
    let at = rig.source.bar().last().unwrap().center();
    rig.frames(&format!("click {} {}\nframe", at.x, at.y));
    assert_eq!(log.lock().unwrap().len(), 2);
}

#[test]
fn with_no_editor_variable_the_button_is_disabled_with_its_reason() {
    let (mut rig, log) = spied(None);
    assert_eq!(rig.source.open_note(), Some(NO_EDITOR));
    rig.frames("frame");
    let at = rig.source.bar().last().unwrap().center();
    rig.frames(&format!("click {} {}\nframe", at.x, at.y));
    assert!(log.lock().unwrap().is_empty());
    assert!(!rig.source.open_in_editor());
    assert_eq!(
        rig.source.notice(),
        Some(&Notice::Refused(NO_EDITOR.to_string()))
    );
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn a_bound_key_opens_the_editor() {
    let (rig, log) = spied(Some("vi"));
    let key = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::E);
    let source = Source::new("source", &rig.file)
        .with_head(Fixed(Err(NoHead::NoGit)))
        .with_opener(Spy {
            command: Some("vi".into()),
            log: log.clone(),
        })
        .open_shortcut(Some(key));
    let mut rig = Rig { source, ..rig };
    rig.source.sync(&rig.files, &NoCheck);
    rig.source.jump(0);
    rig.frames("frame");
    rig.frames("key ctrl+E\nframe");
    assert_eq!(log.lock().unwrap().len(), 1);
}

#[test]
fn an_outside_write_reloads_as_one_undoable_step() {
    let mut rig = Rig::new("outside.toml", Err(NoHead::NoGit));
    let theirs = SAVED.replace("format = 1", "format = 2");
    std::fs::write(&rig.file, &theirs).unwrap();
    let Rig {
        files,
        history,
        source,
        ..
    } = &mut rig;
    let changed = source.reload(files, history, &NoCheck).expect("reloaded");
    assert_eq!(changed.label, "reload");
    assert_eq!(source.text(), theirs);
    assert!(!source.unapplied());
    assert_eq!(history.undo_len(), 1);
    assert!(source.reload(files, history, &NoCheck).is_none());
    assert_eq!(history.undo_len(), 1);
    assert!(source.undo(1.0, files, history, &NoCheck).is_some());
    assert_eq!(source.text(), SAVED);
    assert!(source.redo(2.0, files, history, &NoCheck).is_some());
    assert_eq!(source.text(), theirs);
}

#[test]
fn the_panels_own_save_is_not_an_outside_write() {
    let mut rig = Rig::new("own.toml", Err(NoHead::NoGit));
    rig.type_in(&edited(), 0.0);
    rig.files.save(&rig.file).unwrap();
    let Rig {
        files,
        history,
        source,
        ..
    } = &mut rig;
    let before = history.undo_len();
    assert!(source.reload(files, history, &NoCheck).is_none());
    assert_eq!(history.undo_len(), before);
}

#[test]
fn two_drives_on_one_context_with_clicks_in_both_do_not_panic() {
    let mut rig = Rig::new("twice.toml", Err(NoHead::NoGit));
    rig.source.set_diff(true);
    rig.frames("click 40 40\nframe");
    rig.frames("click 60 60\nframe");
    rig.source.set_diff(false);
    rig.frames("click 40 90\nframe");
    rig.frames("click 60 90\nframe");
}
