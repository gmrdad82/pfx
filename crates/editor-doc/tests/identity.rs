use std::path::{Path, PathBuf};

use pfx_editor_doc::toml_edit::{Item, Table, value};
use pfx_editor_doc::{Edit, Files, GAP, Group, History, TextSplice, TomlPatch, parse_path};

const FULL: &str = include_str!("data/full.toml");
const STAND: &str = include_str!("data/stand.scene.toml");
const NOTES: &str = "first line\nsecond line\nthird line\n";
const STEPS: usize = 400;
const SEED: u64 = 0x5eed_d0c5;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const NUMBERS: &[(&str, &str)] = &[
    ("full.toml", "seed"),
    ("full.toml", "plate.width"),
    ("full.toml", "object[crate].at[1]"),
    ("full.toml", "object[drum].radius"),
    ("full.toml", "object[slab].outline.union[1].segments"),
    ("full.toml", "camera[main].fov_y_deg"),
    ("full.toml", "sun.irradiance"),
    ("full.toml", "sky.room.lights[key].power"),
    ("full.toml", "light[lamp].intensity"),
    ("full.toml", "bake.samples"),
    ("full.toml", "object[new].size"),
    ("stand.scene.toml", "object[ball].at[1]"),
    ("stand.scene.toml", "object[plinth].scale"),
    ("stand.scene.toml", "camera.fov"),
    ("stand.scene.toml", "light[lamp].range"),
    ("stand.scene.toml", "object[crate].rotate[1]"),
];

const LISTS: &[(&str, &str)] = &[
    ("full.toml", "object[drum].materials"),
    ("full.toml", "bake.anchors"),
    ("full.toml", "mover[1].parts"),
    ("stand.scene.toml", "materials"),
    ("stand.scene.toml", "object[floor].scale"),
];

const ENTRIES: &[(&str, &str)] = &[("full.toml", "object"), ("stand.scene.toml", "object")];

fn files() -> Files {
    let mut files = Files::new();
    files.open("full.toml", FULL);
    files.open("stand.scene.toml", STAND);
    files.open("notes.txt", NOTES);
    files
}

fn texts(files: &Files) -> Vec<String> {
    files
        .paths()
        .map(|p| files.text(p).unwrap().to_string())
        .collect()
}

fn patch(rng: &mut Rng, files: &Files) -> Box<dyn Edit> {
    let p = |s: &str| parse_path(s).unwrap();
    match rng.below(6) {
        0..=2 => {
            let (file, path) = *rng.pick(NUMBERS);
            Box::new(TomlPatch::set(
                file,
                p(path),
                value(rng.below(90) as i64 + 1),
            ))
        }
        3 => {
            let (file, path) = *rng.pick(LISTS);
            let len = files
                .doc(Path::new(file))
                .and_then(|d| pfx_editor_doc::value_at(d, &p(path)))
                .and_then(|v| v.as_array().map(|a| a.len()))
                .unwrap_or(0);
            if len > 0 && rng.below(2) == 0 {
                let mut at = p(path);
                at.push(pfx_editor_doc::Key::Index(rng.below(len)));
                Box::new(TomlPatch::remove(file, at))
            } else {
                Box::new(TomlPatch::push(
                    file,
                    p(path),
                    value(rng.below(9) as i64 + 1),
                ))
            }
        }
        4 => {
            let (file, path) = *rng.pick(ENTRIES);
            let mut table = Table::new();
            table.insert("name", value(format!("n{}", rng.below(1000))));
            table.insert("at", value(rng.below(9) as i64));
            Box::new(TomlPatch::push(file, p(path), Item::Table(table)))
        }
        _ => {
            let (file, path) = *rng.pick(ENTRIES);
            let mut at = p(path);
            at.push(pfx_editor_doc::Key::Index(rng.below(4)));
            Box::new(TomlPatch::remove(file, at))
        }
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .filter(|at| *at < text.len())
        .collect()
}

fn splice(rng: &mut Rng, files: &Files, step: usize) -> TextSplice {
    let file = *rng.pick(&["full.toml", "stand.scene.toml", "notes.txt"]);
    let text = files.text(Path::new(file)).unwrap();
    let digits: Vec<usize> = text
        .char_indices()
        .filter(|(_, c)| c.is_ascii_digit())
        .map(|(at, _)| at)
        .collect();
    match rng.below(3) {
        1 if !digits.is_empty() => {
            let at = *rng.pick(&digits);
            let new = char::from(b'1' + rng.below(9) as u8).to_string();
            TextSplice::new(file, at..at + 1, &text[at..at + 1], new)
        }
        2 => {
            let starts = line_starts(text);
            let at = *rng.pick(&starts);
            let line = &text[at..];
            if line.starts_with("# step") {
                let end = at + line.find('\n').map_or(line.len(), |n| n + 1);
                TextSplice::new(file, at..end, &text[at..end], "")
            } else {
                TextSplice::new(file, at..at, "", " ").with_carets(at..at, at + 1..at + 1)
            }
        }
        _ => {
            let at = *rng.pick(&line_starts(text));
            TextSplice::new(file, at..at, "", format!("# step {step}\n"))
        }
    }
}

fn typed_next(splice: &TextSplice) -> TextSplice {
    let end = splice.range.start + splice.new.len();
    TextSplice::new(splice.file.clone(), end..end, "", " ")
}

#[test]
fn undo_then_redo_is_the_identity_over_a_mixed_sequence() {
    let mut rng = Rng(SEED);
    let mut files = files();
    let start = texts(&files);
    let mut history = History::new(10_000);
    let mut now = 0.0;
    let (mut applied, mut refused, mut merged) = (0, 0, 0);
    for step in 0..STEPS {
        now += [0.1, 0.3, 0.9, 2.5][rng.below(4)];
        let generation = files.generation();
        let before = texts(&files);
        let steps = history.undo_len();
        let outcome = match rng.below(10) {
            0..=3 => {
                let edit = patch(&mut rng, &files);
                history.apply_boxed(&mut files, edit)
            }
            4..=5 => {
                let s = splice(&mut rng, &files, step);
                let next = (s.new == " ").then(|| typed_next(&s));
                let first = history.typed(&mut files, s, now, GAP);
                if let Some(next) = next
                    && first.as_ref().is_ok_and(|c| !c.is_empty())
                {
                    let before_merge = history.undo_len();
                    let out = history.typed(&mut files, next, now + 0.05, GAP);
                    if out.is_ok() && history.undo_len() == before_merge {
                        merged += 1;
                    }
                }
                first
            }
            6..=7 => {
                let mut group = Group::new(format!("group {step}"));
                let mut scratch = files.clone();
                for _ in 0..2 + rng.below(2) {
                    let mut edit: Box<dyn Edit> = if rng.below(2) == 0 {
                        patch(&mut rng, &scratch)
                    } else {
                        Box::new(splice(&mut rng, &scratch, step))
                    };
                    let _ = edit.apply(&mut scratch);
                    group.push(edit);
                }
                history.apply(&mut files, group)
            }
            8 => {
                history.undo(&mut files).unwrap();
                continue;
            }
            _ => {
                history.redo(&mut files).unwrap();
                continue;
            }
        };
        match outcome {
            Ok(changed) => {
                applied += 1;
                assert_eq!(
                    changed.is_empty(),
                    files.generation() == generation,
                    "step {step}"
                );
                for path in &changed.files {
                    assert!(files.generation_of(path) > Some(generation), "step {step}");
                }
            }
            Err(_) => {
                refused += 1;
                assert_eq!(
                    texts(&files),
                    before,
                    "a refused step {step} changed a file"
                );
                assert_eq!(
                    files.generation(),
                    generation,
                    "a refused step {step} moved the generation"
                );
                assert_eq!(history.undo_len(), steps);
            }
        }
    }
    eprintln!(
        "{applied} applied, {refused} refused, {merged} merged, {} steps",
        history.undo_len()
    );
    assert!(applied > STEPS / 2, "{applied} applied, {refused} refused");
    assert!(
        refused > 0 && merged > 0,
        "{refused} refused, {merged} merged"
    );
    for path in [PathBuf::from("full.toml"), "stand.scene.toml".into()] {
        assert!(
            files.doc(&path).is_some(),
            "{} still parses: {:?}",
            path.display(),
            files.get(&path).unwrap().error().map(ToString::to_string)
        );
    }

    let end = texts(&files);
    let mut seen = vec![end.clone()];
    while history.undo(&mut files).unwrap().is_some() {
        seen.push(texts(&files));
    }
    assert_eq!(texts(&files), start);
    seen.pop();
    while let Some(changed) = history.redo(&mut files).unwrap() {
        assert!(!changed.is_empty());
        assert_eq!(texts(&files), seen.pop().unwrap());
    }
    assert!(seen.is_empty());
    assert_eq!(texts(&files), end);
}
