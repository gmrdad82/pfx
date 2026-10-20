# Projects: one binary per game

**The scaffold builds once a pfx tag carries `pfx-game` and `pfx-project`, from v0.23.0.** It pins the tag pfx was built from; v0.22.0 and earlier have neither crate, so a game scaffolded by a pfx before v0.23.0 does not build until its pins move to v0.23.0 or later.

A game on pfx is its own repo and one binary, `<game>`. With no arguments it plays the game; `<game> edit` opens the same game in pfx's editor; `<game> help` lists its commands. The Steam build is the same binary built without the `tools` feature: the game alone. `pfx new` scaffolds such a repo, and `pfx gui <project folder>` opens any project in the editor.

The crate `pfx-project` (`crates/project`) reads a project and, with its `scaffold` feature, writes a new one. It carries nothing of any game: the scaffold is pfx's own empty world.

## A project

A project is a folder with pfx-scene's `project.toml` at its root ([the format](scene-format.md#projecttoml), `project.toml`). Its scenes, libraries, meshes and images are found under that root. `project.toml` is the game repo's manifest, beside its `Cargo.toml`; its scenes are `.pitofx` documents, each holding a scene with its prefabs and material libraries ([.pitofx files](pfx-file.md#the-user-file)), and the meshes, images and fonts they use stay files.

```toml
format = 1

[project]
name = "my-game"
scene = "content/first.pitofx"

[authoring.pfx]
play = "content/first.pitofx"
select = "title"
```

| Key | What it says | Read by |
|---|---|---|
| `[project] name` | the project's name | the editor's title and log |
| `[project] scene` | the scene the editor opens first | `pfx gui <folder>`, `<game> edit` |
| `[authoring.pfx] play` | the scene the game plays; without it, `scene` | the game's `main` (`Project::play`) |
| `[authoring.pfx] select` | the object the editor selects when it opens, if that scene has it | the editor |
| `[authoring.pfx.screen]` | the game's `[screen]` policy (`docs/screens.md`): its aspects per device, the bars, the safe area and the UI scale | the game's `config()` (`Config::screen`) and the editor, which frames the scene camera with the policy's first aspect for the device |
| `[authoring.pfx.graphics]` | the game's default quality (`quality = "appearance"` or `"performance"`) and effect switches (`[authoring.pfx.graphics.effects]`, [settings.md](settings.md#effects)); an unknown key is refused | the game's `config()` (`Project::graphics`, as `Config::settings`) |
| `[authoring.pfx] flat` | `true` for a game with no 3D scene (`Config::flat`): the editor opens such a project without any scene on an empty world, and play draws no 3D pass; a value that is not a boolean is refused | the editor |
| `[authoring.pfx] redraw` | `"on-demand"` (the default: the window draws a frame only when something changed) or `"always"` (every frame drawn, `Config::redraw(Redraw::Always)`); any other value is refused | the game's `config()` (`Project::redraws_always`), [game.md](game.md#frames-on-demand) |
| `[authoring.pfx] modules` | the game's gameplay modules: the built `.wasm` of each, relative to the root; a module's id is its file's stem | the game (`Project::modules`, `pfx_mod::Modules`), and the editor's live reload |
| `[project] ignore` | folders (or files) not to look in, relative to the root, besides `target/` and `tmp/` | pfx-scene's `Project::open`, and `Project::scenes` and `folders` here |
| `[tunables.<name>]` | the game's tunables (`docs/play.md`, "Tunables") | pfx-play, through pfx-scene's `ProjectFile` |

`[authoring.pfx]` is pfx's authoring table: pfx-scene checks only that it is TOML and keeps it byte for byte.

`pfx_project::Project`:

- `Project::open(folder)` reads the folder's `project.toml` and refuses a missing file, a missing `[project]` or `name`, and a `scene`, `play` or `select` that is not a string, naming the file. `Project::folder(folder)` is the same, and for a folder without `project.toml` (the engine's fixture folders) it is a bare project named after the folder.
- `Project::find(path)` walks up from a file or folder to the nearest `project.toml`. `Project::locate(manifest_dir)` is what a game's `main` calls: the folder beside the running exe when it holds a `project.toml` (a shipped build packs `project.toml` and `content/` beside the exe), else the crate's own folder (`cargo run`).
- `scenes()` lists every scene `.pitofx` (one holding a scene's `SMET` chunk) and every old `*.scene.toml` (imported once into a `.pitofx` when it is opened), and `folders()` every content folder (one that directly holds a scene, prefab, material library, proxies file, glTF, glb, buffer, PNG, HDR, WAV, Ogg, font, SVG, KTX2 or EXR), each with its files. Both are sorted by name and skip dot folders, folders with their own `project.toml`, the root's `target/` and `tmp/` (cargo's output and scratch) and the paths `[project] ignore` lists, as pfx-scene's own walk does. The scaffold's `project.toml` lists nothing in `ignore`: `target/` and `tmp/` are skipped by default.
- `flat()` is `[authoring.pfx] flat`, false without it.
- `redraws_always()` is `[authoring.pfx] redraw = "always"`, false without it or with `"on-demand"`.
- `modules()` lists the gameplay modules as `Module { id, path }`, in the order `modules` names them; a `modules` that is not a list of `.wasm` paths is refused, naming the file.
- `first()` is the scene the editor opens: `scene`, else `play`, else the first scene listed. `play()` is `play`, else `scene`, else the first scene listed.

## pfx new

```sh
pfx new <game> [--path <dir>] [--with-module] [--mark <png>]
```

scaffolds a game into an existing, empty repo folder (default `./<game>`). You make the repo; pfx never creates a git repo, writes nothing outside the folder and never overwrites a file. It refuses:

- a folder that does not exist, or holds anything but `.git`;
- a name that is not a lowercase slug (letters a to z, digits and single dashes, starting with a letter), and `pfx`, `pfx-*` and the names cargo or Rust reserve (`test`, `self`, `std`, …);
- a `--mark` that is not a PNG, before it writes anything.

It writes:

| File | What it is |
|---|---|
| `Cargo.toml` | one package `<game>`; `pfx-game`, `pfx-project` and, optional, `pfx-workbench`, all from `https://github.com/gmrdad82/pfx.git` at the tag pfx was built from (`v` and its own version); `default = ["tools"]`, `tools = ["pfx-game/tools", "dep:pfx-workbench"]` |
| `src/lib.rs` | the game: a unit struct named after the game (`my-game` gives `my_game::MyGame`) implementing `pfx_game::Game`, doing nothing yet; `project()` (`Project::locate`); and `config(&project)`, the game's `pfx_game::Config` on `project.play()` with the project's screen policy, its build date (`.build_date(option_env!("PFX_BUILD_DATE"))`, below) and `Exposure::Fixed(1.0)`, the exposure the editor's viewport draws with, instead of auto exposure (delete the line for auto) |
| `build.rs` | the game's build date, below; std only, no build dependencies |
| `src/main.rs` | `#![cfg_attr(not(feature = "tools"), windows_subsystem = "windows")]`, then `pfx_game::launch(<Game>::new, config(&project), …)`; with `tools` it sets the launcher's edit hook to `pfx_workbench::edit_project(project.root(), factory, config(&project)?, args)`, the hook's factory, the game's own config and the arguments after `edit` |
| `tests/first.rs` | a GPU test, ignored by default: the first scene runs headless (`pfx_game::Headless::with_gpu`) for half a second at 960 × 540, writes `tmp/first-frame.png` and checks the name's light and the X axis's red |
| `project.toml` | the project, as above: `scene` and `play` name `content/first.pitofx`, `select` names `title`, and `[authoring.pfx.screen]` holds 16:9 on a desktop and 16:10 on a Deck |
| `content/first.pitofx` | the empty world, below, with inside it `content/first.scene.toml`, `content/prefabs/title.prefab.toml` (the game's title as a group: its name, its mark and its version, placed by the scene's `title`) and `content/materials/base.materials.toml` and `title.materials.toml` (the grid's and the axes' materials; the name's and the mark's); `pfx scene print content/first.pitofx <file>` shows each |
| `content/meshes/name.glb`, `mark.glb`, `grid.glb` | the meshes, below; `name.glb` and `mark.glb` are made for the game when it scaffolds |
| `content/marks/mark.png` | with `--mark` only: the game's mark as given |
| `content/fonts/IBMPlexMono-Regular.ttf`, `IBMPlexMono-OFL.txt` | the name's and the version's face, IBM Plex Mono as IBM ships it, and its SIL Open Font License (Reserved Font Name "Plex", so it is never subset or renamed here) |
| `content/credit/powered-by-128.png`, `powered-by-light-128.png` | pfx's Powered by lockup, 128 px tall, for a dark ground and a light one; the README shows it, and the game may show it in its own credits |
| `AGENTS.md`, `CLAUDE.md`, `prompts/RESUME.md` | the repo's own session starts there |
| `README.md` | the game's name as its title, one line, the one `pfx new` also prints last: "This game links pfx: publish its source under the AGPL-3.0, or use pfx's free licence (closed use under $200k a year, education, non-profits) or a commercial one: https://pitomd.com/pfx/license", and the Powered by lockup, a `<picture>` of the two PNGs above that shows the lime one on a dark theme and the violet one on a light one. The scaffold writes no licence file and no other legal text; the game chooses its own licence |
| `.gitignore` | `/target` and `/tmp` |
| `.gitattributes` | `*.pitofx diff=pitofx`, so `git diff` shows a `.pitofx` as its TOML; in a folder that is already a git repository, pfx new also sets the repository's git config ([.pitofx in git](git.md)) |

### The build date

A pfx Pro key hides the splash only for a build made while it was paid ([The splash](splash.md), "Hiding it"), so the game needs its own build date. The scaffold's `build.rs` sets `PFX_BUILD_DATE` (`YYYY-MM-DD`, UTC) at compile time from, in order:

1. `SOURCE_DATE_EPOCH`, when the build sets it (a reproducible or release build);
2. else the committer time of the game repo's `HEAD` (`git log -1 --format=%ct`), only when the game's folder is a git work tree. It reruns when `HEAD` or the branch it points to moves. A work tree with uncommitted changes still takes `HEAD`'s time, so commit before a build that needs today's date;
3. else nothing: a build with no git and no `SOURCE_DATE_EPOCH` has no date, and a Pro key cannot hide its splash.

It never falls back to the clock. `config(&project)` passes the value to `Config::build_date`; a game made before the scaffold had a `build.rs` copies the file and adds that line.

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

`content/first.pitofx` opens like a 3D tool's default scene and reads like Blender's cube:

- **The game's title is one group, `title`,** at the origin, standing on the floor (`at = [0, 0.495, 0]`, half the group's height). It is a prefab placement (`content/prefabs/title.prefab.toml`, inside the `.pitofx`), pfx-scene's group: it draws nothing itself and is the parent of the prefab's two objects, so the outliner shows `title` with `title/name` and `title/mark` under it. Its origin is the centre of the group's bounds, so the move gizmo sits in the middle of the group, and `select = "title"` opens the editor on it.
- **The name** (`name.glb`, material `name`) is the game's name as `pfx new` was given it, in 3D: each letter's outline from the scaffold's IBM Plex Mono (ttf-parser), extruded 7 cm with a rounded 6 mm edge by `pfx_geom::mark` (the extrusion pfx's SVG marks use, so a counter such as the `e`'s is a hole through one closed solid). Its capital height is 36 cm, and a long name shrinks to fit 2.4 m. `pfx_project::title::build` makes it.
- **The mark** (`mark.glb`) is a rounded 99 cm square plate, 8 cm thick, in the material `plate`, to the name's left. With `--mark <png>` it also carries a face, a quad just in front of the plate fitted to 80 % of it at the picture's aspect, in the material `face`, whose content layer shows `content/marks/mark.png` (`content = "mark"` and `[content.mark]` in the prefab); without it the plate stands alone, a neutral shape to replace.
- **The version** is the prefab's `[text.version]`, `v{version}`: the game's own version, bound when the game or its editor opens the scene (`docs/scenes.md`, "Bound text"), in IBM Plex Mono at 7 cm, in a dim grey, under the name. It is part of the group, so it moves with `title`.
- **The floor grid and the axes** (`grid`, `grid.glb`, from `recipes/grid.toml`): ten by ten one-metre squares, the X axis in red and the Z axis in blue through the origin.
- **One light** and a room sky, and a **three-quarter perspective camera** from the front right that looks at the group's origin.

All of it is data. Delete the objects, the prefab, meshes and materials, or point `play` and `scene` at another scene, and nothing of the starter is left. Nothing of pfx's own brand reaches a game: no mark, wordmark or pfx version (the scaffold's unit tests fail on one). The scaffold's files sit in `crates/project/src/templates/`, the grid's glb as built.

### --mark

`pfx new <game> --mark <png>` copies the PNG to `content/marks/mark.png`, shows it on the title's plate, and sets `config(&project)`'s `.mark(project.root().join("content/marks/mark.png"))`, so the editor's header shows the same mark beside the game's name ([The game runtime](game.md)). To change the mark later, replace the PNG; to add one to a game made without it, add the face to the prefab and the `.mark(...)` line to `config`.

### pfx's own start meshes

`crates/project/src/templates/badge.glb` and `wordmark.glb` are pfx's mark and wordmark in 3D, held in `pfx_project::marks` for pfx's own start page and never written into a game. They were built once and are kept as built: the wordmark from `recipes/wordmark.toml` (the wordmark's outlines extruded 7 cm with a 6 mm round edge), the mark from pfx's mark recipe with its materials averaged into five groups, both merged into one group with the wordmark 12 cm right of the mark's tile, its face flush with the tile's front. `grid.glb` is `recipes/grid.toml` built with `pfx build` ([Recipes](recipes.md)). A change to them is made by editing the recipe and rebuilding; a one-off step the build can't do is written in `tmp/` and deleted.

**Stack.** pfx-scene resolves without recursion, in about 250 KB of stack at any depth, and refuses a chain deeper than `pfx_scene::MAX_DEPTH` (64) with `include-depth` or `prefab-depth`. pfx gui boxes the editor crates' viewport and the play view, so `Session::open_project` opens the scaffold in under 512 KB in a debug build; tests run it on an ordinary 2 MB test thread.

## <game> edit

The scaffold's `main` sets the launcher's edit hook (`docs/game.md`, "The launcher"):

```rust
commands
    .edit(
        "opens the game in pfx's editor; F5 plays it; --shot and --clip draw it offscreen",
        move |factory, args| {
            let config = my_game::config(&project)?;
            pfx_workbench::edit_project(project.root(), factory, config, args)
        },
    )
    .usage("edit", pfx_workbench::EDIT_USAGE);
```

`usage("edit", pfx_workbench::EDIT_USAGE)` gives `edit`'s shape, and `edit_project` refuses anything else (`my-game edit extra`) in clap's words under it, the way every PITO MD command line refuses input it cannot parse (`docs/game.md`, "Refusals"). With no arguments `edit_project` opens the window; `--shot` and `--clip` draw it offscreen on a copy of the project (`docs/game.md`, "The edit hook"). The hook gets a factory, `pfx_game::Factory` (`Rc<dyn Fn() -> Box<dyn Game>>`, the constructor `launch` was given), and the arguments after `edit`; no game is built until a play needs one. `run_project(folder, factory, config)` opens the project in pfx's window as `pfx gui <folder>` does (a pfx-editor `Session::open_game` under the window's panels), and **F5 plays the game's own logic**: `factory()` at each F5, a fresh game every play, drawn by pfx-game's own painter from `config` (its fonts, icons, sprites, screen policy, exposure and settings), so it looks and sounds as its window does (`docs/editor.md`, "Play mode"). A `Config::flat()` game plays on an empty world, and its project opens even with no scene (below). Pause, step, stop and its exact restore, F9's input recording, sound (the game's own PCM streams included), the HUD and the pgpu hooks work as `docs/editor.md` says; a failing game stops play with its error in the log. `pfx_editor::factory(make)` (the same as `pfx_game::factory`) builds a factory from a constructor, for a `Session::open_project` or `open_game` of one's own.

## Live reload

While a game plays in `<game> edit` (F5), the editor watches the files of the game's gameplay modules: the ones `Game::modules()` lists (`Reload::files`). Each frame it compares their modified time and size; when a module is rebuilt it hands the file to the game (`Reload::changed`), and the game's `Modules` swaps it in at its next tick, with its state through `save` and `restore` or clean with a notice. The log says each step:

```
modules: watching 1 built module while the game plays
modules: module `turns` started at tick 0
modules: modules/turns.wasm was rebuilt; it swaps in at the next tick
modules: module `turns` swapped at tick 12 and kept its state (4 bytes through save and restore)
```

A build that does not load (cut short mid-write, or wrong) leaves the running module and logs a warning; a module that fails is stopped with its error in the log, and the game plays on. While play is paused, a rebuilt module swaps in at the next step or once play resumes. The window's file watcher also wakes the editor when a module changes. `pfx gui <folder>` plays the scene without the game's logic, so it has no modules to reload, unless it plays a module of its own ([play-modules.md](play-modules.md)), which reloads the same way. No Rust is hot-patched: rebuilding the game itself still means stopping and running it again.

## pfx gui <project folder>

`pfx gui <folder>` opens a project on its first scene; `pfx gui <scene.pitofx>` opens one scene. In project mode:

- the outliner starts with two sections: **scenes**, every scene of the project by its path from the root (the open one marked `· open`), and **content**, every content folder with its files;
- selecting a scene shows its path and an `open` button; **Enter**, the button or a double-click opens it. The editor stops play, ends any edit group, and opens that scene in a fresh editor, keeping the project, the log, mute and the pgpu state;
- selecting a folder shows how many content files it holds, and a file its size;
- the window's title is `pfx gui · <project> · <scene file>`;
- the object `select` names is selected when the project opens.

A folder without `project.toml`, like the engine's fixture folders, opens as a bare project: its scenes and folders, no `select`. A folder with no scene is refused, unless the project is flat (`[authoring.pfx] flat = true`, or the game's config is `Config::flat()` in `run_project`): then the editor opens an empty world, `tmp/pfx/flat.pitofx` under the project's root (an empty scene document, written once in the project's scratch folder, which `scenes()` skips), the log says `project <name> is flat and has no scene`, and F5 plays the game on `Scene::empty()`. A flat project with scenes opens its first scene as any other, and still plays flat.

## Tests

- `crates/project/src/tests.rs` (in the gate): reading `project.toml`, the fallbacks of `play` and `first`, refusals, the scene and folder listing and what it skips, bare folders, `find`; the name rule, the templates' fill, and a scaffold into an empty repo folder (every file, the pins and features, `main`'s first line, the ids written, the meshes, the same bytes twice) and its refusals; the scaffold's `build.rs` and `config` passing its build date, and the `build.rs` date conversion (the template itself, compiled into the tests).
- `src/main.rs`: `pfx new`'s arguments.
- `crates/editor/src/tests.rs` (in the gate): the scenes and content sections, the selection on open, Enter, the `open` button and a double-click asking to open a scene, and the switch keeping the project and the log; a folder without scenes refused and a bare fixture folder opened. A project whose prefab places itself, whose scenes include each other, or whose prefabs nest 70 deep is refused with pfx-scene's finding, never a crash: by pfx new's project check (`crates/project/src/tests.rs`), by `Session::open_project` and `Session::open` (`crates/editor/src/tests.rs`) and by a game's start (`crates/game/src/tests.rs`). pfx new's check runs pfx-scene's `check` before it fixes the ids, so its error names the finding.
- `crates/project/tests/scaffold.rs`, run by hand (each `#[ignore]`):
  - `the_scaffold_builds_with_and_without_tools_and_only_tools_links_the_editor` scaffolds into `tmp/scaffold/build`, points every pfx crate at this workspace through a `[patch]` written only for the test, builds with `--no-default-features` and checks the binary carries nothing of `pfx_editor`, `pfx_workbench`, `egui`, `pfx_editor_shell` or pfx-game's tools marker, then builds with the defaults and checks it carries all five, and runs `help`, `version` and a refused `edit extra`;
  - `the_scaffolded_games_tests_build` builds the scaffold's own tests, so the GPU run below only compiles the game;
  - `the_scaffolded_game_runs_headless_and_shows_its_world` runs the scaffold's `tests/first.rs` under pgpu and copies its frame to `tmp/scaffold/first-frame.png`.
- `crates/editor/tests/project.rs` (GPU, ignored): the editor's offscreen harness opens a scaffolded project, shows the group `title` selected with the move gizmo's centre within 5 % of the viewport's middle, the name and the X axis; the 3D view covers the same rect through the scene's camera and the editor's, with the scene camera's 16:9 frame shown only through the scene's camera, and the scene-camera shot is compared with the stored `crates/editor/tests/shots/project-scene-camera.png` (`PFX_EDIT_BLESS=1` rewrites it); F5 without a game plays the scene as it is, inside the project's 16:9 frame with black bars; F5 with a game factory plays the game's own logic (it hides the name, the mark and the grid on its first tick; `start` once, ticks counted); F9 saves its input; F8 stops it (`stop` once) and the viewport comes back; no file changes. The shots go to `tmp/editor-shots/project-*.png`.
- `crates/project/src/tests.rs` also checks `modules` and its refusals, and `--with-module`'s files, workspace, feature, world, project entry and docs, and that a plain scaffold has none of them.
- `src/main.rs`: `pfx new`'s arguments, `--with-module` included.
- `crates/editor/src/tests.rs` (in the gate): the scenes and content sections, the selection on open, Enter, the `open` button and a double-click asking to open a scene, and the switch keeping the project and the log; a folder without scenes refused and a bare fixture folder opened.
- `crates/project/tests/scaffold.rs`, run by hand (each `#[ignore]`):
  - `the_scaffold_builds_with_and_without_tools_and_only_tools_links_the_editor` scaffolds into `tmp/scaffold/build`, points every pfx crate at this workspace through a `[patch]` written only for the test, builds with `--no-default-features` and checks the binary carries nothing of `pfx_editor`, `pfx_workbench`, `egui`, `pfx_editor_shell` or pfx-game's tools marker, then builds with the defaults and checks it carries all five, and runs `help`, `version` and a refused `edit extra`;
  - `the_scaffolded_games_tests_build` builds the scaffold's own tests, so the GPU run below only compiles the game;
  - `the_scaffolded_game_runs_headless_and_shows_its_world` runs the scaffold's `tests/first.rs` under pgpu and copies its frame to `tmp/scaffold/first-frame.png`;
  - `the_scaffold_with_a_module_builds_and_its_module_compiles` builds a `--with-module` scaffold with the defaults and checks its module crate;
  - `the_scaffolds_module_builds_to_wasm_and_the_host_swaps_it_keeping_its_state` (needs the `wasm32-unknown-unknown` target) builds the module to wasm, loads it through `Host::load_module`, ticks it twice and swaps it keeping its count.
- `crates/project/src/tests.rs` reads `[authoring.pfx] flat` and refuses a value that is not a boolean, and the scaffold's `main.rs` hands the edit hook's factory, the game's config and the arguments to `edit_project`. `crates/game/src/tests.rs` (with `tools`) checks the edit hook gets a factory that builds no game until it is called and one fresh game each call. `crates/editor/src/tests.rs` opens a flat project with no scene on the empty world, and `crates/editor/tests/play.rs` (GPU) plays it: see `docs/editor.md`, "Tests".
- `crates/editor/tests/project.rs` (GPU, ignored), `the_editor_swaps_a_rebuilt_gameplay_module_during_play_and_keeps_its_state`: a project whose game ticks one module, rebuilt during play at its tick 12 (the game writes the new build, as cargo would); the editor sees it, the module swaps at the next tick, its count carries on through `save` and `restore`, the rebuilt module's turn hides the title, and the log has each step. The shot is `tmp/editor-shots/project-module-reload.png`.
- `crates/editor/tests/project.rs` (GPU, ignored): the editor's offscreen harness opens a scaffolded project, shows the group `title` selected with the move gizmo's centre within 5 % of the viewport's middle, the name and the X axis; the 3D view covers the same rect through the scene's camera and the editor's; F5 without a game plays the scene as it is; F5 with a game factory plays the game's own logic (it hides the name, the mark and the grid on its first tick; `start` once, ticks counted); F9 saves its input; F8 stops it (`stop` once) and the viewport comes back; no file changes. The shots go to `tmp/editor-shots/project-*.png`.

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
