# Mark mistakes in an unsaved buffer

A text editor holds a file the person hasn't saved yet. `check` checks that text in every context it takes part in, against the project on disk for references, and returns only the diagnostics located in that file, so the editor can mark them as someone types.

```rust
use std::path::Path;

use pfx_scene::{Project, check};

fn main() {
    let project = Project::open(Path::new("my-game")).unwrap();
    let buffer = std::fs::read_to_string("my-game/scenes/room.scene.toml").unwrap()
        .replace("\"brass\"", "\"bras\"");
    for d in check(&buffer, Path::new("scenes/room.scene.toml"), &project) {
        println!("{}:{}-{}:{} {}", d.line, d.column, d.end_line, d.end_column, d.message);
    }
}
```

On the demo project this prints the misspelt material's span, ready to underline:

```text
28:12-28:18 bras names no material of the scene's libraries
```

The file's path is absolute or relative to the project root. Each diagnostic's `line`, `column`, `end_line` and `end_column` are 1-based and count characters, with the end just past the span, so they map straight onto an editor's ranges. Its `code` is stable, so an editor can offer a fix by code.

## Several unsaved files

When more than one file is unsaved, or a new include exists only in the editor, give every text at once. `Project::check_with` checks the whole project and `Project::scene_with` resolves a scene, each with the overlay's texts in place of the files they name; a file only the overlay holds counts as existing.

```rust
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_scene::Project;

fn main() {
    let project = Project::open(Path::new("my-game")).unwrap();
    let mut overlay = BTreeMap::new();
    overlay.insert(
        PathBuf::from("scenes/extra.scene.toml"),
        "format = 1\n\n[[light]]\nid = \"q8m2k4n6p0\"\nname = \"fill\"\nposition = [2.0, 2.0, 0.0]\nintensity = 10.0\n".to_string(),
    );
    overlay.insert(
        PathBuf::from("scenes/room.scene.toml"),
        std::fs::read_to_string("my-game/scenes/room.scene.toml").unwrap()
            .replacen("format = 1\n", "format = 1\ninclude = [\"scenes/extra.scene.toml\"]\n", 1),
    );
    let scene = project.scene_with(Path::new("scenes/room.scene.toml"), overlay).unwrap();
    println!("{} lights", scene.lights.len());
}
```

`check` checks the whole project for every buffer, so its time grows with the project: 2.8 ms for a 100-object scene and 33 ms for 1,000 objects (see [Measure the speed](scene-speed.md)).
