use std::fs;
use std::path::{Path, PathBuf};

use pfx_input::{InputEvent, Key};
use pfx_play::{Options, UiFrame};

use crate::editor::Editor;
use crate::play::Play;
use crate::tests::scene_copy;

fn marker() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../game/examples/marker/marker.wat");
    fs::read_to_string(&path).unwrap()
}
const STEP: f32 = 1.0 / 60.0;

fn module(wat: &str) -> Vec<u8> {
    pfx_game::wasm::embed(&wat::parse_str(wat).unwrap()).unwrap()
}

fn marked(name: &str, wasm: &[u8]) -> PathBuf {
    let scene = scene_copy(name);
    let root = scene.parent().unwrap();
    fs::create_dir_all(root.join("modules")).unwrap();
    fs::write(root.join("modules/marker.wasm"), wasm).unwrap();
    let mut text = fs::read_to_string(&scene).unwrap();
    text.push_str(
        "\n[play]\nmodule = \"modules/marker.wasm\"\n\n[[object]]\nname = \"marker\"\nmesh = \"block\"\nat = [1.0, 0.25, 0.5]\nscale = 0.5\n",
    );
    fs::write(&scene, text).unwrap();
    scene
}

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found: Vec<_> = fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| (entry.path(), fs::read(entry.path()).unwrap()))
        .collect();
    found.sort();
    found
}

fn at(play: &Play) -> [f32; 3] {
    let world = play.session().unwrap().world();
    world.at(world.object("marker").unwrap())
}

#[test]
fn the_example_module_plays_the_open_scene_and_stop_restores_it() {
    let scene = marked("play-module", &module(&marker()));
    let root = scene.parent().unwrap().to_path_buf();
    let editor = Editor::open(&scene).unwrap();
    assert_eq!(
        editor.module.module(),
        Some(root.join("modules/marker.wasm").as_path()),
        "the scene's [play] module names the module"
    );
    let before = files(&root);
    let edited = editor.scene().clone();
    let mut play = Play::of(editor.scene());
    play.start(editor.scene(), None, Options::default())
        .unwrap();
    assert!(play.is_running());
    let start = at(&play);
    play.feed(InputEvent::Key {
        key: Key::Right,
        pressed: true,
    });
    for _ in 0..30 {
        assert!(play.frame(STEP).unwrap());
    }
    let moved = at(&play);
    assert!(moved[0] > start[0] + 0.8, "{start:?} to {moved:?}");
    assert_eq!([moved[1], moved[2]], [start[1], start[2]]);
    let ui = play.session_mut().unwrap().ui(&UiFrame::default());
    let score: u32 = ui.labels[0].text["score ".len()..].parse().unwrap();
    assert!(score >= 29, "{}", ui.labels[0].text);
    let stopped = play.stop().unwrap();
    assert!(!play.is_running());
    assert_eq!(stopped.scene, edited, "stop hands back the edited scene");
    assert_eq!(
        *editor.scene(),
        edited,
        "nothing the game did reached the editor"
    );
    assert_eq!(files(&root), before, "play writes no file");
    play.start(editor.scene(), None, Options::default())
        .unwrap();
    assert!(play.frame(STEP).unwrap());
    assert_eq!(at(&play), start, "play again starts from the edited scene");
    let ui = play.session_mut().unwrap().ui(&UiFrame::default());
    assert_eq!(ui.labels[0].text, "score 0");
    play.stop();
}

#[test]
fn a_module_that_never_returns_or_does_not_load_stops_play_with_one_message() {
    let spin = module(
        "(module (memory (export \"memory\") 1) (func (export \"start\")) (func (export \"tick\") (loop $l (br $l))))",
    );
    let scene = marked("play-module-spin", &spin);
    let editor = Editor::open(&scene).unwrap();
    let mut play = Play::of(editor.scene());
    play.start(editor.scene(), None, Options::default())
        .unwrap();
    let error = (0..4)
        .find_map(|_| play.frame(STEP).err())
        .expect("the spinning module stops play");
    assert!(error.contains("ran out of fuel"), "{error}");
    assert!(!play.is_running());
    assert!(play.stop().is_some(), "the stopped session is handed back");

    let root = scene.parent().unwrap();
    fs::write(root.join("modules/marker.wasm"), b"not wasm").unwrap();
    let error = play
        .start(editor.scene(), None, Options::default())
        .unwrap_err();
    assert!(error.contains("module `marker`"), "{error}");
    assert!(!play.is_running());
}
