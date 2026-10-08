# Projects: one binary per game

**The scaffold builds once a pfx tag carries `pfx-game` and `pfx-project`, from v0.23.0.** It pins the tag pfx was built from; v0.22.0 and earlier have neither crate, so a game scaffolded by a pfx before v0.23.0 does not build until its pins move to v0.23.0 or later.

A game on pfx is its own repo and one binary, `<game>`. With no arguments it plays the game; `<game> edit` opens the same game in pfx's editor; `<game> help` lists its commands. The Steam build is the same binary built without the `tools` feature: the game alone. `pfx new` scaffolds such a repo, and `pfx edit <project folder>` opens any project in the editor.

The crate `pfx-project` (`crates/project`) reads a project and, with its `scaffold` feature, writes a new one. It carries nothing of any game: the scaffold is pfx's own empty world.

## A project

A project is a folder with pfx-scene's `project.toml` at its root ([the format](https://github.com/gmrdad82/pfx-scene/blob/v0.1.0/docs/format.md), `project.toml`). Its scenes, libraries, meshes and images are found under that root.

```toml
format = 1

[project]
name = "my-game"
scene = "content/first.scene.toml"

[authoring.pfx]
play = "content/first.scene.toml"
select = "incoming"
```

| Key | What it says | Read by |
|---|---|---|
| `[project] name` | the project's name | the editor's title and log |
| `[project] scene` | the scene the editor opens first | `pfx edit <folder>`, `<game> edit` |
| `[authoring.pfx] play` | the scene the game plays; without it, `scene` | the game's `main` (`Project::play`) |
| `[authoring.pfx] select` | the object the editor selects when it opens, if that scene has it | the editor |
| `[authoring.pfx.screen]` | the game's `[screen]` policy (`docs/screens.md`): its aspects per device, the bars, the safe area and the UI scale | the game's `config()` (`Config::screen`) and the editor, which frames the scene camera with the policy's first aspect for the device |
| `[authoring.pfx] flat` | `true` for a game with no 3D scene (`Config::flat`): the editor opens such a project without any `*.scene.toml` on an empty world, and play draws no 3D pass; a value that is not a boolean is refused | the editor |
| `[authoring.pfx] modules` | the game's gameplay modules: the built `.wasm` of each, relative to the root; a module's id is its file's stem | the game (`Project::modules`, `pfx_mod::Modules`), and the editor's live reload |
| `[project] ignore` | folders (or files) not to look in, relative to the root, besides `target/` and `tmp/` | pfx-scene's `Project::open`, and `Project::scenes` and `folders` here |
| `[tunables.<name>]` | the game's tunables (`docs/play.md`, "Tunables") | pfx-play, through pfx-scene's `ProjectFile` |

`[authoring.pfx]` is pfx's authoring table: pfx-scene checks only that it is TOML and keeps it byte for byte.

`pfx_project::Project`:

- `Project::open(folder)` reads the folder's `project.toml` and refuses a missing file, a missing `[project]` or `name`, and a `scene`, `play` or `select` that is not a string, naming the file. `Project::folder(folder)` is the same, and for a folder without `project.toml` (the engine's fixture folders) it is a bare project named after the folder.
- `Project::find(path)` walks up from a file or folder to the nearest `project.toml`. `Project::locate(manifest_dir)` is what a game's `main` calls: the folder beside the running exe when it holds a `project.toml` (a shipped build packs `project.toml` and `content/` beside the exe), else the crate's own folder (`cargo run`).
- `scenes()` lists every `*.scene.toml`, and `folders()` every content folder (one that directly holds a scene, prefab, material library, proxies file, glTF, glb, buffer, PNG, HDR, WAV, Ogg, font, SVG, KTX2 or EXR), each with its files. Both are sorted by name and skip dot folders, folders with their own `project.toml`, the root's `target/` and `tmp/` (cargo's output and scratch) and the paths `[project] ignore` lists, as pfx-scene's own walk does. The scaffold's `project.toml` lists nothing in `ignore`: `target/` and `tmp/` are skipped by default.
- `flat()` is `[authoring.pfx] flat`, false without it.
- `modules()` lists the gameplay modules as `Module { id, path }`, in the order `modules` names them; a `modules` that is not a list of `.wasm` paths is refused, naming the file.
- `first()` is the scene the editor opens: `scene`, else `play`, else the first scene listed. `play()` is `play`, else `scene`, else the first scene listed.

## pfx new

```sh
pfx new <game> [--path <dir>] [--with-module]
```

scaffolds a game into an existing, empty repo folder (default `./<game>`). You make the repo; pfx never creates a git repo, writes nothing outside the folder and never overwrites a file. It refuses:

- a folder that does not exist, or holds anything but `.git`;
- a name that is not a lowercase slug (letters a to z, digits and single dashes, starting with a letter), and `pfx`, `pfx-*` and the names cargo or Rust reserve (`test`, `self`, `std`, …).

It writes:

| File | What it is |
|---|---|
| `Cargo.toml` | one package `<game>`; `pfx-game`, `pfx-project` and, optional, `pfx-editor`, all from `https://github.com/gmrdad82/pfx.git` at the tag pfx was built from (`v` and its own version); `default = ["tools"]`, `tools = ["pfx-game/tools", "dep:pfx-editor"]` |
| `src/lib.rs` | the game: a unit struct named after the game (`my-game` gives `my_game::MyGame`) implementing `pfx_game::Game`, doing nothing yet; `project()` (`Project::locate`); and `config(&project)`, the game's `pfx_game::Config` on `project.play()` with the project's screen policy and `Exposure::Fixed(1.0)`, the exposure the editor's viewport draws with, instead of auto exposure (delete the line for auto) |
| `src/main.rs` | `#![cfg_attr(not(feature = "tools"), windows_subsystem = "windows")]`, then `pfx_game::launch(<Game>::new, config(&project), …)`; with `tools` it sets the launcher's edit hook to `pfx_editor::run_project(project.root(), factory, config(&project)?)`, the hook's factory and the game's own config |
| `tests/first.rs` | a GPU test, ignored by default: the first scene runs headless (`pfx_game::Headless::with_gpu`) for half a second at 960 × 540, writes `tmp/first-frame.png` and checks the wordmark's light, the mark's violet and the X axis's red |
| `project.toml` | the project, as above: `scene` and `play` name `content/first.scene.toml`, `select` names `incoming`, and `[authoring.pfx.screen]` holds 16:9 on a desktop and 16:10 on a Deck |
| `content/first.scene.toml` | the empty world, below |
| `content/prefabs/incoming.prefab.toml` | pfx's mark as a group: the badge, the wordmark and the game's version, placed by the scene's `incoming` |
| `content/materials/base.materials.toml`, `incoming.materials.toml` | the grid's and the axes' materials; the badge's and the wordmark's |
| `content/meshes/badge.glb`, `wordmark.glb`, `grid.glb` | the meshes, below |
| `content/fonts/IBMPlexMono-Regular.ttf`, `IBMPlexMono-OFL.txt` | the version's face, IBM Plex Mono as IBM ships it, and its SIL Open Font License (Reserved Font Name "Plex", so it is never subset or renamed here) |
| `AGENTS.md`, `CLAUDE.md`, `prompts/RESUME.md` | the repo's own session starts there |
| `.gitignore` | `/target` and `/tmp` |

### --with-module

`pfx new <game> --with-module` adds a gameplay module: game logic as a WebAssembly component that the editor swaps while the game plays (`docs/mods.md`, "Gameplay modules").

| File | What changes |
|---|---|
| `Cargo.toml` | a `[workspace]` of the game and `module` with `default-members = ["."]`, so `cargo build` and `cargo run` build the game alone; `pfx-game` gains `features = ["modules"]` |
| `module/Cargo.toml`, `module/src/lib.rs` | `<game>-module`, a `cdylib` on wit-bindgen 0.62: a counter of turns with `save` and `restore` |
| `wit/gameplay.wit` | the world, `game:gameplay@0.1.0`: `tick: func(tick: u64) -> u32`, `save: func() -> list<u8>`, `restore: func(state: list<u8>)` |
| `src/lib.rs` | the game binds the world with `pfx_game::modules::wasmtime::component::bindgen!`, opens `Modules` from the project, calls `tick()` then every module's `tick` each tick, and hands the modules to the editor through `Game::modules` |
| `project.toml` | `[authoring.pfx] modules = ["target/wasm32-unknown-unknown/release/<crate>_module.wasm"]` |
| `AGENTS.md` | a section on the module and its build |

The module builds with

```sh
rustup target add wasm32-unknown-unknown
cargo build -p <game>-module --target wasm32-unknown-unknown --release
```

and needs no other tool: the game makes the core module a component when it loads it. Until it is built the game runs without it and says so once. Rust's `wasm32-wasip2` target does not fit: its standard library imports WASI, which the host refuses. A shipped build carries the built module at the path `project.toml` names, beside the exe.

After writing, it fills the scene's ids through pfx-scene (`Project::fix`, as `pfx scene fix` does) and checks the project, so a fresh scaffold opens with no warning.

**One tag.** Every pfx crate the game takes, the editor's own included, comes from pfx at one tag, so a tools build holds one copy of each engine crate and `cargo tree -d` names no pfx crate twice. Move every pfx line in the game's `Cargo.toml` together.

### The empty world

`content/first.scene.toml` opens like a 3D tool's default scene and reads like Blender's cube:

- **pfx's mark is one group, `incoming`,** at the origin, standing on the floor (`at = [0, 0.495, 0]`, half the group's height). It is a prefab placement (`content/prefabs/incoming.prefab.toml`), pfx-scene's group: it draws nothing itself and is the parent of the prefab's two objects, so the outliner shows `incoming` with `incoming/badge` and `incoming/wordmark` under it. Its origin is the centre of the group's bounds, so the move gizmo sits in the middle of the group, and `select = "incoming"` opens the editor on it.
- **The badge** (`badge.glb`) is one mesh, one primitive per material: manfred's build of `brand/recipes/incoming-full.toml` (`manfred build`), merged by `crates/project/src/templates/recipes/merge.py` into one node and stood upright, with the recipe's `ground` left out. The recipe's 86 materials collapse into six by look, each the average of its members: `tile`, `dome` (the ball), `lines` (its lattice), `white` (the rim and the bright streaks), `violet` (the darker streaks, rim bands and dust) and `lime`.
- **The wordmark** (`wordmark.glb`) is one mesh with one material, `wordmark`: `assets/marks/wordmark.svg` extruded 7 cm with a rounded edge by manfred (0.16, which extrudes an outline with holes as one closed solid) from `recipes/wordmark.toml`, merged the same way. `recipes/make.py` writes that recipe: one object per glyph, its outline's quadratic curves in eight steps, 0.65 mm per SVG unit, and a glyph with a counter (the `p`'s bowl) as `difference = [outer, counter]`, so the `p` is one seamless solid.
- **One plane.** The badge's front face and the wordmark's stand on one plane: `merge.py` places the wordmark so its front meets the badge tile's front, its x-height centred on the badge, 12 cm to its right. The seven materials are in `content/materials/incoming.materials.toml`, the prefab's own library.
- **The version** is the prefab's `[text.version]`: the version of the pfx that made the game, written as text when it scaffolds (`text = "v0.25.0"` from pfx 0.25.0), in IBM Plex Mono at 7.5 cm, in the mark's violet, under the wordmark's `x`. It stays as written when the game moves to a newer pfx, until the game edits it; a `{version}` token in its place shows the game's own version instead (`docs/scenes.md`, "Bound text"). It is part of the group, so it moves with `incoming`.
- **The floor grid and the axes** (`grid`, `grid.glb`, from `recipes/grid.toml`): ten by ten one-metre squares, the X axis in red and the Z axis in blue through the origin.
- **One light** and a room sky, and a **three-quarter perspective camera** from the front right that looks at the group's origin.

All of it is data. Delete the objects, the prefab, meshes and materials, or point `play` and `scene` at another scene, and nothing of pfx's world is left; nothing of it is in pfx's code. The scaffold's files sit in `crates/project/src/templates/`, the glbs and libraries as built.

To rebuild them, from the repo's root:

```sh
mkdir -p tmp/marks3d
manfred build brand/recipes/incoming-full.toml --out tmp/marks3d
python3 crates/project/src/templates/recipes/make.py tmp/marks3d/recipes
manfred build tmp/marks3d/recipes/wordmark.toml --out tmp/marks3d
manfred build tmp/marks3d/recipes/grid.toml --out tmp/marks3d
python3 crates/project/src/templates/recipes/merge.py tmp/marks3d/pfx-incoming-full.glb tmp/marks3d/pfx-wordmark.glb tmp/marks3d/merged
```

`merge.py` prints the children's places in the group (`badge_at`, `wordmark_at`) and its half height, which the prefab's and the scene's `at` hold; copy `merged/` and `pfx-grid.glb` over the templates. With manfred 0.16 the chain gives the checked-in files byte for byte.

**Stack.** pfx-scene resolves without recursion, in about 250 KB of stack at any depth, and refuses a chain deeper than `pfx_scene::MAX_DEPTH` (64) with `include-depth` or `prefab-depth`. pfx edit boxes the editor crates' viewport and the play view, so `Session::open_project` opens the scaffold in under 512 KB in a debug build; tests run it on an ordinary 2 MB test thread.

## <game> edit

The scaffold's `main` sets the launcher's edit hook (`docs/game.md`, "The launcher"):

```rust
commands
    .edit("opens the game in pfx's editor; F5 plays it", move |factory, _args| {
        let config = my_game::config(&project)?;
        pfx_editor::run_project(project.root(), factory, config)
    })
    .usage("edit", "");
```

`usage("edit", "")` says `edit` takes no arguments, so `my-game edit extra` is refused before the hook runs, the way every PITO command line refuses input it cannot parse (`docs/game.md`, "Refusals"). The hook gets a factory, `pfx_game::Factory` (`Rc<dyn Fn() -> Box<dyn Game>>`, the constructor `launch` was given), and the arguments after `edit`; no game is built until a play needs one. `run_project(folder, factory, config)` opens the project as `pfx edit <folder>` does (`Session::open_game`), and **F5 plays the game's own logic**: `factory()` at each F5, a fresh game every play, drawn by pfx-game's own painter from `config` (its fonts, icons, sprites, screen policy, exposure and settings), so it looks and sounds as its window does (`docs/editor.md`, "Play mode"). A `Config::flat()` game plays on an empty world, and its project opens even with no `*.scene.toml` (below). Pause, step, stop and its exact restore, F9's input recording, sound (the game's own PCM streams included), the HUD and the pgpu hooks work as `docs/editor.md` says; a failing game stops play with its error in the log. `pfx_editor::factory(make)` (the same as `pfx_game::factory`) builds a factory from a constructor, for a `Session::open_project` or `open_game` of one's own.

## Live reload

While a game plays in `<game> edit` (F5), the editor watches the files of the game's gameplay modules: the ones `Game::modules()` lists (`Reload::files`). Each frame it compares their modified time and size; when a module is rebuilt it hands the file to the game (`Reload::changed`), and the game's `Modules` swaps it in at its next tick, with its state through `save` and `restore` or clean with a notice. The log says each step:

```
modules: watching 1 built module while the game plays
modules: module `turns` started at tick 0
modules: modules/turns.wasm was rebuilt; it swaps in at the next tick
modules: module `turns` swapped at tick 12 and kept its state (4 bytes through save and restore)
```

A build that does not load (cut short mid-write, or wrong) leaves the running module and logs a warning; a module that fails is stopped with its error in the log, and the game plays on. While play is paused, a rebuilt module swaps in at the next step or once play resumes. The window's file watcher also wakes the editor when a module changes. `pfx edit <folder>` plays the scene without the game's logic, so it has no modules to reload. No Rust is hot-patched: rebuilding the game itself still means stopping and running it again.

## pfx edit <project folder>

`pfx edit <folder>` opens a project on its first scene; `pfx edit <scene.toml>` is unchanged. In project mode:

- the outliner starts with two sections: **scenes**, every scene of the project by its path from the root (the open one marked `· open`), and **content**, every content folder with its files;
- selecting a scene shows its path and an `open` button; **Enter**, the button or a double-click opens it. The editor stops play, ends any edit group, and opens that scene in a fresh editor, keeping the project, the log, mute and the pgpu state;
- selecting a folder shows how many content files it holds, and a file its size;
- the window's title is `pfx edit · <project> · <scene file>`;
- the object `select` names is selected when the project opens.

A folder without `project.toml`, like the engine's fixture folders, opens as a bare project: its scenes and folders, no `select`. A folder with no scene is refused, unless the project is flat (`[authoring.pfx] flat = true`, or the game's config is `Config::flat()` in `run_project`): then the editor opens an empty world, `tmp/pfx/flat.scene.toml` under the project's root (`format = 1` and nothing else, written once in the project's scratch folder, which `scenes()` skips), the log says `project <name> is flat and has no scene`, and F5 plays the game on `Scene::empty()`. A flat project with scenes opens its first scene as any other, and still plays flat.

## Tests

- `crates/project/src/tests.rs` (in the gate): reading `project.toml`, the fallbacks of `play` and `first`, refusals, the scene and folder listing and what it skips, bare folders, `find`; the name rule, the templates' fill, and a scaffold into an empty repo folder (every file, the pins and features, `main`'s first line, the ids written, the meshes, the same bytes twice) and its refusals.
- `src/main.rs`: `pfx new`'s arguments.
- `crates/editor/src/tests.rs` (in the gate): the scenes and content sections, the selection on open, Enter, the `open` button and a double-click asking to open a scene, and the switch keeping the project and the log; a folder without scenes refused and a bare fixture folder opened. A project whose prefab places itself, whose scenes include each other, or whose prefabs nest 70 deep is refused with pfx-scene's finding, never a crash: by pfx new's project check (`crates/project/src/tests.rs`), by `Session::open_project` and `Session::open` (`crates/editor/src/tests.rs`) and by a game's start (`crates/game/src/tests.rs`). pfx new's check runs pfx-scene's `check` before it fixes the ids, so its error names the finding.
- `crates/project/tests/scaffold.rs`, run by hand (each `#[ignore]`):
  - `the_scaffold_builds_with_and_without_tools_and_only_tools_links_the_editor` scaffolds into `tmp/scaffold/build`, points every pfx crate at this workspace through a `[patch]` written only for the test, builds with `--no-default-features` and checks the binary carries nothing of `pfx_editor`, `egui`, `pfx_editor_shell` or pfx-game's tools marker, then builds with the defaults and checks it carries all four, and runs `help`, `version` and a refused `edit extra`;
  - `the_scaffolded_games_tests_build` builds the scaffold's own tests, so the GPU run below only compiles the game;
  - `the_scaffolded_game_runs_headless_and_shows_its_world` runs the scaffold's `tests/first.rs` under pgpu and copies its frame to `tmp/scaffold/first-frame.png`.
- `crates/editor/tests/project.rs` (GPU, ignored): the editor's offscreen harness opens a scaffolded project, shows the group `incoming` selected with the move gizmo's centre within 5 % of the viewport's middle, the badge's violet, the wordmark and the X axis; the 3D view covers the same rect through the scene's camera and the editor's, with the scene camera's 16:9 frame shown only through the scene's camera, and the scene-camera shot is compared with the stored `crates/editor/tests/shots/project-scene-camera.png` (`PFX_EDIT_BLESS=1` rewrites it); F5 without a game plays the scene as it is, inside the project's 16:9 frame with black bars; F5 with a game factory plays the game's own logic (it hides the badge, the wordmark and the grid on its first tick; `start` once, ticks counted); F9 saves its input; F8 stops it (`stop` once) and the viewport comes back; no file changes. The shots go to `tmp/editor-shots/project-*.png`.
- `crates/project/src/tests.rs` also checks `modules` and its refusals, and `--with-module`'s files, workspace, feature, world, project entry and docs, and that a plain scaffold has none of them.
- `src/main.rs`: `pfx new`'s arguments, `--with-module` included.
- `crates/editor/src/tests.rs` (in the gate): the scenes and content sections, the selection on open, Enter, the `open` button and a double-click asking to open a scene, and the switch keeping the project and the log; a folder without scenes refused and a bare fixture folder opened.
- `crates/project/tests/scaffold.rs`, run by hand (each `#[ignore]`):
  - `the_scaffold_builds_with_and_without_tools_and_only_tools_links_the_editor` scaffolds into `tmp/scaffold/build`, points every pfx crate at this workspace through a `[patch]` written only for the test, builds with `--no-default-features` and checks the binary carries nothing of `pfx_editor`, `egui`, `pfx_editor_shell` or pfx-game's tools marker, then builds with the defaults and checks it carries all four, and runs `help`, `version` and a refused `edit extra`;
  - `the_scaffolded_games_tests_build` builds the scaffold's own tests, so the GPU run below only compiles the game;
  - `the_scaffolded_game_runs_headless_and_shows_its_world` runs the scaffold's `tests/first.rs` under pgpu and copies its frame to `tmp/scaffold/first-frame.png`;
  - `the_scaffold_with_a_module_builds_and_its_module_compiles` builds a `--with-module` scaffold with the defaults and checks its module crate;
  - `the_scaffolds_module_builds_to_wasm_and_the_host_swaps_it_keeping_its_state` (needs the `wasm32-unknown-unknown` target) builds the module to wasm, loads it through `Host::load_module`, ticks it twice and swaps it keeping its count.
- `crates/project/src/tests.rs` reads `[authoring.pfx] flat` and refuses a value that is not a boolean, and the scaffold's `main.rs` hands the edit hook's factory and the game's config to `run_project`. `crates/game/src/tests.rs` (with `tools`) checks the edit hook gets a factory that builds no game until it is called and one fresh game each call. `crates/editor/src/tests.rs` opens a flat project with no scene on the empty world, and `crates/editor/tests/play.rs` (GPU) plays it: see `docs/editor.md`, "Tests".
- `crates/editor/tests/project.rs` (GPU, ignored), `the_editor_swaps_a_rebuilt_gameplay_module_during_play_and_keeps_its_state`: a project whose game ticks one module, rebuilt during play at its tick 12 (the game writes the new build, as cargo would); the editor sees it, the module swaps at the next tick, its count carries on through `save` and `restore`, the rebuilt module's turn hides the mark, and the log has each step. The shot is `tmp/editor-shots/project-module-reload.png`.
- `crates/editor/tests/project.rs` (GPU, ignored): the editor's offscreen harness opens a scaffolded project, shows the group `incoming` selected with the move gizmo's centre within 5 % of the viewport's middle, the badge's violet, the wordmark and the X axis; the 3D view covers the same rect through the scene's camera and the editor's; F5 without a game plays the scene as it is; F5 with a game factory plays the game's own logic (it hides the badge, the wordmark and the grid on its first tick; `start` once, ticks counted); F9 saves its input; F8 stops it (`stop` once) and the viewport comes back; no file changes. The shots go to `tmp/editor-shots/project-*.png`.

```sh
cargo test -q -p pfx-project --features scaffold
cargo test -q -p pfx-project --features scaffold --test scaffold -- --ignored the_scaffold_builds --test-threads=1
cargo test -q -p pfx-project --features scaffold --test scaffold -- --ignored the_scaffold_with_a_module_builds --test-threads=1
cargo test -q -p pfx-project --features scaffold --test scaffold -- --ignored the_scaffolds_module_builds_to_wasm
cargo test -q -p pfx-project --features scaffold --test scaffold -- --ignored the_scaffolded_games_tests_build
pgpu run --class clip --as pfx -- cargo test -q -p pfx-project --features scaffold --test scaffold -- --ignored the_scaffolded_game_runs_headless --test-threads=1
cargo test -q -p pfx-editor --test project --no-run
pgpu run --class clip --as pfx -- cargo test -q -p pfx-editor --test project -- --ignored --test-threads=1
```
