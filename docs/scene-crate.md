# The pfx-scene crate

<p align="center"><img src="images/scene-check.gif" width="800" alt="The scene-check example on the demo project: a path with the wrong case and a misspelt material, each reported at its line and column with its code, then both fixed and the scene resolved"></p>

`pfx-scene` is one TOML format for 3D scenes, prefabs and material libraries, shared by a game engine and a 3D editor, so a scene saved in one opens in the other. It is the content format of pfx and of its editor, and the crate (`crates/scene`) reads, checks, resolves, edits, migrates and packs it. It reads no clock and no system randomness, loads no asset and draws nothing.

- **Fast.** It resolves a 1,000-object scene in 25 ms and checks its whole project in 28 ms; a 100-object scene takes 3 ms for either. These times were measured on the earlier standalone crate (pfx-scene 1.6.0), before shadows, live, play and the later keys, and have not been re-measured ([Measure the speed](scene-speed.md)).
- **Small.** Its dependencies are `serde`, `toml`, `toml_edit`, `serde_json` and `pfx-file` (the `.pitofx` container); `schemars` joins them only with the `schema` feature. `#![forbid(unsafe_code)]` holds.
- **Exact.** Every mistake is reported at its file, line and column with a stable code, and every edit keeps the author's comments, spacing and key order.

Its tests run on Linux. It builds for Windows as part of pfx's Windows check, but its own tests are untested on Windows and on macOS.

It is for the tools on either side of a scene file: an engine's loader, an editor that saves what a person arranges, a build tool that emits scenes, and a text editor that marks mistakes as someone types.

## Take it

The crate is a package of pfx's workspace, taken by pfx's tag or by path inside a checkout:

```toml
[dependencies]
pfx-scene = { git = "https://github.com/gmrdad82/pfx", tag = "<tag>" }
```

Put the pfx tag your project pins in place of `<tag>`. The crate's `.pitofx` container is the `pfx-file` crate, which `pfx_scene` re-exports as `pfx_scene::pfx_file`.

## Try the demo

The crate holds a demo project in `crates/scene/examples/demo` and small programs on its API. From a checkout of pfx, with a stable Rust toolchain, check the demo:

```sh
cargo run --release -p pfx-scene --example scene-check -- crates/scene/examples/demo
```

```text
scenes/room.scene.toml: 6 objects, 2 prefab placements, 3 bodies, 1 light, 2 materials
no errors
```

The other programs are `scene-tree` (prints a resolved scene), `scene-write` (moves, renames or sets a material in place, or migrates a format 0 file), `scene-pack` (packs a scene into a `.pitofx` and unpacks one), `scene-schema` (writes the JSON Schemas, with `--features schema`) and `scene-bench` (measures the table in [Measure the speed](scene-speed.md)). The guides below use each of them.

The shortest useful program opens a project, prints every finding and resolves its scene:

```rust
use std::path::Path;

fn main() {
    let project = pfx_scene::Project::open(Path::new("my-game")).unwrap();
    for diagnostic in project.check() {
        println!("{diagnostic}");
    }
    let scene = project.scene(Path::new("scenes/room.scene.toml")).unwrap();
    println!("{} objects", scene.objects.len());
}
```

`my-game` is a folder with a `project.toml`. [Lay out a project](scene-projects.md) says what goes in it.

## The format

A project is a folder with a `project.toml`, and every file in it is plain TOML that starts with `format = 1`. This is the demo's scene, shortened:

```toml
format = 1
materials = ["materials/room.materials.toml"]

[mesh.ball]
id = "ttfwjw4wnm"
file = "assets/ball.gltf"

[[object]]
id = "j66hkn5wx4"
name = "lamp"
mesh = "ball"
material = "brass"
at = [0.0, 2.4, 0.0]
scale = 0.3

# Two crates from one prefab; the second is turned and lighter.
[[object]]
id = "axxz3n1hcj"
name = "right crate"
prefab = "prefabs/crate.prefab.toml"
at = [1.2, 0.5, 0.3]
rotate = [0.0, 30.0, 0.0]

[object.set]
"box.body.density" = 0.5
```

Scenes (`*.scene.toml`), prefabs (`*.prefab.toml`), material libraries (`*.materials.toml`) and collision proxies (`*.proxies.toml`) cover meshes, lights, materials, the sky, sun, haze, camera and render settings, rigid bodies, character controllers, trigger volumes, camera rigs, collision layers, glTF animation clips, sprite atlases, tile layers, and a game's tunables with their ranges. Assets are standard files referenced by path: glTF 2.0, PNG, WAV or Ogg Vorbis, TTF or OTF, Radiance HDR. [The scene format](scene-format.md) specifies every key, its type, default and range.

Every file names its format version. A file without one is format 0: reading it migrates it in memory and warns once (`old-format`), and a newer one than the crate knows is refused with the version it knows (`newer-format`). `Project::migrate` gives a format 0 file's text as format 1, and a plain `.scene.toml` is imported into a `.pitofx` once ([Import a TOML file](scene-migrate.md)).

## Errors at file, line and column

`Project::check` checks a whole project and `check` checks one unsaved buffer against it, so a text editor can mark mistakes as someone types. Each finding has a file, a line, a column, an end, a stable code (`bad-reference`, `path-case`, `missing-key` and the rest, listed in the spec) and, where it helps, the other place involved. A misspelt material, a path whose case differs from the disk, a key of the wrong type, an id used twice and a reference to nothing are each one finding, at the exact text. [Check a project](scene-check.md) and [mark mistakes in an unsaved buffer](scene-check-buffer.md) show them.

## Prefabs, overrides and stable ids

`Project::scene` resolves a scene: includes merged, prefabs placed as objects with their per-placement overrides (`[object.set]`), and every reference rewritten to the key of the entry it names. A placed entry's key and id are namespaced by its placement (`right crate/box`, `axxz3n1hcj/6dek3mm9mh`), so two placements of one prefab never collide. References go by id, with names as labels, so a rename or a move never breaks them. Includes and prefabs nest up to 64 deep, and a cycle is refused with its chain of files. [Resolve a scene](scene-resolve.md) and [Fill missing ids](scene-ids.md) have the details.

## A careful writer

`SceneEdit` edits a scene's files in place. A changed value keeps its place, its neighbours, the comments and the number spelling, so a diff shows only what changed. An edit the checker would refuse is refused before anything is written. Edits group into undo and redo steps by exact text, and `dry_run` computes an edit's patches without writing. [Edit a scene in place](scene-edit.md) walks through it.

## The API

```rust
use std::path::Path;

use pfx_scene::{Project, SceneEdit, Target, check};

fn main() {
    let project = Project::open(Path::new("my-game")).unwrap();
    for diagnostic in project.check() {
        println!("{diagnostic}");
    }

    let scene = project.scene(Path::new("scenes/room.scene.toml")).unwrap();
    for object in &scene.objects {
        println!("{} at {:?}", object.key, object.value.at);
    }

    let buffer = std::fs::read_to_string("my-game/scenes/room.scene.toml").unwrap();
    let marks = check(&buffer, Path::new("scenes/room.scene.toml"), &project);
    println!("{} marks for the editor", marks.len());

    let mut edit = SceneEdit::open("my-game/scenes/room.scene.toml").unwrap();
    edit.set(&Target::Object("right crate".into()), &["at"], [0.0, 0.5, 1.0]).unwrap();
    edit.undo().unwrap();
}
```

- `Project::check` checks every file of a project; `check` checks one unsaved buffer against it.
- `Project::scene` resolves a scene; `Project::read` returns one file as its typed data, plain serde structs.
- `SceneEdit` edits a scene's files in place, refusing any edit that the checker would refuse; `dry_run` computes an edit's patches without writing.
- `Project::scene_with` and `check_with` resolve and check with unsaved texts in place of the files.
- `Project::fix` fills the ids a scene's files lack, and `Project::migrate` gives a format 0 file's text as format 1.
- `json_schema` (the `schema` feature) gives the JSON Schema of a kind of file, for TOML editors ([Validate in a TOML editor](scene-schema.md)).
- `pack` puts a scene and every file it reads into one `.pitofx`, the same bytes every time; `unpack` writes them back out. `Project::open_pfx` opens one, and checking, resolving and `SceneEdit` work on the TOML inside it in place ([.pitofx files](pfx-file.md#as-a-scene)).

How the loaded scene reaches the engine's types is in [Scenes as data](scenes.md).
