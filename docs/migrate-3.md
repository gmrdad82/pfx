# Moving a game from pfx 2.6.0 to 3.0.0

pfx 3.0.0 holds what used to sit in separate repositories: the scene format (pfx-scene), its container (manf, now pfx-file), the recipe builder and the scene window. A game that built on pfx 2.6.0 takes every crate from pfx's own tag now, renames a few items, converts its scene folders and recipes into `.pitofx` files, and opens its window through pfx-workbench. This page lists each step; every name on it is checked against the code at 3.0.0.

## The crates

Every crate is a package in pfx's workspace, taken by pfx's tag (`{ git = "https://github.com/gmrdad82/pfx", tag = "v3.1.3" }`) or by path inside a checkout. The ones a game uses most are first.

| Package | Holds |
|---|---|
| `pfx-game` | the one call a game's main makes: the window, pacing, input, sound, frame stats, the fixed tick and the game's own launcher |
| `pfx-play` | play mode: a game run live over a runtime copy of a scene, with pause, step and an exact stop |
| `pfx-project` | a game's `project.toml`, its scenes and content folders, and `pfx new`'s scaffold |
| `pfx-load` | asset loading: meshes, images, HDR skies, fonts and scenes, from files or from a `.pitofx` in memory |
| `pfx-scene` | the scene format: TOML scenes, materials and prefabs, typed tables, errors at file:line:column, stable ids, migrations, a comment-keeping writer, and scenes packed into one file |
| `pfx-file` | the container: magic `PFXFILE`, a chunk table, then checksummed chunks, each stored or compressed (LZ4 block or raw deflate) |
| `pfx-workbench` | the scene window: recipes, `.pitofx` documents and scenes in one layout of panels, the offscreen shot and the tape clips |
| `pfx-editor` | the editing session under the window: documents, play while editing, undo |
| `pfx-editor-doc`, `pfx-editor-shell`, `pfx-editor-style`, `pfx-editor-toml`, `pfx-editor-viewport` | the window's parts: dotted-path patches with one undo history, the egui host and offscreen shot harness, the theme and widgets, the TOML source panel, and the scene viewport |
| `pfx-theme` | the editor and dev HUD look: pfx-editor-style's tokens as a theme, and the text helpers that paint with it |
| `pfx-live` | the live renderer: a rasterizer for a steady 60 fps at 4K, with sun shadows, baked light and temporal anti-aliasing |
| `pfx-trace` | the offline path tracer: the reference for stills, clips and the bake |
| `pfx-materials` | one physically based material shared by the live renderer and the path tracer, procedural layers and ageing included |
| `pfx-input` | keyboard, mouse and controllers behind one action map, glyphs, rumble and input recording |
| `pfx-sound` | chimes and streams played live, and tracks mixed deterministically for rendered clips |
| `pfx-physics` | an SPH fluid with its height field, ripples, a rope and bead chain, flocks, wind, and 3D rigid bodies |
| `pfx-mod` | the mod host: sandboxed, deterministic WASM components, metered by fuel and capped in memory |
| `pfx-steam` | achievements, stats, Cloud saves, the overlay, rich presence, leaderboards, lobbies and lockstep |
| `pfx-text` | glyphs from vector outlines at any screen density |
| `pfx-post` | the shared finish chain and image styles |
| `pfx-gpu` | device and adapter, offscreen targets and readback, GPU timing |
| `pfx-core` | springs and motion, daylight and the sun, deterministic time, and the refusal every command line prints |
| `pfx-recipe` | scene recipes: parse, validate, patch and write |
| `pfx-build` | a recipe to GLB nodes, sidecar facts and a bake recipe, headless and deterministic |
| `pfx-recipe-scene` | a recipe to an engine scene: cameras, sun, sky, lights, materials by preset, text and the headless still |
| `pfx-runtime` | a GLB and its sidecar into a scene, and that scene into the live renderer's instances, lights and content |
| `pfx-glb` | the deterministic GLB writer, the facts sidecar, and the reader and diff behind `pfx diff` |
| `pfx-mesh` | outlines, 2D booleans, profiled extrusion, pockets, lathe, sweep and primitives |
| `pfx-geom` | levels of detail by projected error, analytic shapes, deformable surfaces, SVG marks as solids |
| `pfx-motion` | routes as centripetal Catmull-Rom splines, seeded smooth noise, the timeline and poses |
| `pfx-generators` | the kit for a generator: gradient noise, ports of CPython's random and numpy's RandomState, params helpers |
| `pfx-bake` | path-traced light the live renderer shows at raster cost, as probe grids and lightmaps |
| `pfx-still` | path-traced stills, equirect and ortho, PNG, EXR and HDR out |
| `pfx-denoise` | a port of Intel Open Image Denoise's RT filter to Rust and WGSL |
| `pfx-film` | scripted films rendered offline at a fixed frame rate |
| `pfx-source` | asset sources: search and fetch CC0 assets, check their licences and build a scene's credits |
| `pfx-assets` | asset renders: logos, wordmarks, lockups, tiles, cards and icon sets from SVG masters |
| `pfx-run` | process plumbing for the renderers: children, job progress and the master encode |
| `pfx-bench` | a repeatable headless load for any game or scene, with the engine's frame stats |

## What moved in

| Was | Is now |
|---|---|
| the pfx-scene repository, `pfx-scene = { git = ".../pfx-scene", tag = "v1.x" }` | `crates/scene`, package `pfx-scene`, lib `pfx_scene`, taken by pfx's tag or by path |
| the `manf` crate inside pfx-scene | `crates/file`, package `pfx-file`, lib `pfx_file` |
| `pfx_scene::manf` (the re-export) | `pfx_scene::pfx_file` |
| `pfx-editor`'s window (`pfx_editor::run`, `pfx_editor::run_project`, `pfx_editor::USAGE`) | `pfx_workbench::run_project`; the `pfx` binary opens the window with `pfx gui` |

A game's `Cargo.toml` drops the pfx-scene git line and, if it named pfx-scene directly, takes it from pfx's tag like the rest:

```toml
pfx-game = { git = "https://github.com/gmrdad82/pfx", tag = "v3.1.3" }
pfx-project = { git = "https://github.com/gmrdad82/pfx", tag = "v3.1.3" }
pfx-scene = { git = "https://github.com/gmrdad82/pfx", tag = "v3.1.3" }
pfx-workbench = { git = "https://github.com/gmrdad82/pfx", tag = "v3.1.3", optional = true }
```

The `tools` feature that used to turn on `dep:pfx-editor` turns on `dep:pfx-workbench` instead: `tools = ["pfx-game/tools", "dep:pfx-workbench"]`.

## Renamed items

Found by diffing the public items of pfx-scene 1.6.0 (with manf 0.1.0) and pfx 2.6.0 against 3.0.0. Everything not listed keeps its name.

| 2.6.0 / pfx-scene 1.6.0 | 3.0.0 |
|---|---|
| `manf::*` | `pfx_file::*` (`Reader`, `Writer`, `Entry`, `Named`, `Codec`, `Tag`, the tag constants, `scene_hash`, `check_name`, `tag_text`, `lz4`, `sha256`) |
| `manf::MAGIC`, `b"MANFRED\0"` | `pfx_file::MAGIC`, `b"PFXFILE\0"`: a file with the old magic does not open; pack it again from its sources |
| `manf::Error::NotManf` ("not a .manf file") | `pfx_file::Error::NotPfx` ("not a .pitofx file") |
| `manf::crc32` (private) | `pfx_file::crc32` (public) |
| `Project::open_manf(file)` | `Project::open_pfx(file)` |
| `Project::manf_file()` | `Project::pfx_path()` |
| `Project::manf()` | `Project::pfx()` |
| `pfx_editor::run_project(root, factory, config)` | `pfx_workbench::run_project(root, factory, config)` |
| `pfx_theme::DISPLAY`, `pfx_theme::DISPLAY_LICENCE` (Exo 2) | `pfx_theme::SANS`, `pfx_theme::SANS_LICENCE` (Jost); the colour tokens and `MONO_*` keep their names, now defined in pfx-editor-style and re-exported by pfx-theme |

New at 3.0.0, beside the renames: `Project::from_pfx_bytes`, `Project::in_memory` and `Project::pfx_bytes` in pfx-scene, `pfx_load::scene::from_pfx_bytes` and `Scene::from_pfx_bytes` in pfx-load, and `Config::packed` in pfx-game (below).

## Converting a scene folder

A scene folder (a `.scene.toml` with its includes, `.materials.toml` files, prefabs, GLBs or split glTF files, images, fonts and sounds) becomes one `.pitofx` file:

```sh
pfx scene migrate content/room.scene.toml
```

It imports the scene and every file it reads into `content/room.pitofx` beside it, once, and rewrites any format 0 file as format 1 inside it, keeping its comments; the `.toml` is never written again. `pfx scene print content/room.pitofx [file]` shows each TOML file inside it. The packed scene opens from its file alone: `Scene::open("content/room.pitofx")`, `pfx gui content/room.pitofx`, `pfx shot content/room.pitofx`.

## Converting a recipe

A recipe (a TOML file that starts `recipe = 1`) becomes a `.pitofx` document with:

```sh
pfx scene migrate render/room.toml
```

It writes `render/room.pitofx` beside the recipe, once. `pfx scene print <file> [file]` shows the recipe or any file it holds, and `pfx scene fold <file> --apply` folds the edits made in the window back into its recipe (`--keep` keeps them as overrides on later builds). `pfx build`, `pfx align` and `pfx export --glb` take the document; given the old `.toml` they refuse with one line naming `pfx scene migrate`.

## Shipping one binary

A game that ships one binary embeds its `.pitofx` and starts from the bytes; the scene's meshes, images and fonts resolve from the same bytes, and nothing is read from disk:

```rust
const WORLD: &[u8] = include_bytes!("../content/world.pitofx");

fn main() -> std::process::ExitCode {
    let config = pfx_game::Config::packed("world", env!("CARGO_PKG_VERSION"), WORLD);
    pfx_game::launch(world::Game::new, config, |_| {})
}
```

`Config::packed(name, version, bytes)` plays the scene the bytes hold; its messages name it `<name>.pitofx`. Without a file beside it there is no `project.toml` to read, so the game's tunables and rigs start empty. `pfx_load::scene::from_pfx_bytes(name, bytes)` gives the same `Scene` to code that loads scenes itself. A file the scene names and the `.pitofx` does not hold is refused, even when a file of that name exists in the working folder.

## Opening the window

A game's tools build opens its project in pfx's window through `pfx_workbench::run_project`, or `pfx_workbench::edit_project`, which does the same with no arguments and draws the window offscreen with `--shot` or `--clip` ([game.md](game.md), "The edit hook"). The scaffold's `main.rs` (from `pfx new`) does it as the game's `edit` command:

```rust
#[cfg(feature = "tools")]
fn tools(commands: &mut pfx_game::Commands, project: pfx_project::Project) {
    commands
        .edit(
            "opens the game in pfx's editor; F5 plays it; --shot and --clip draw it offscreen",
            move |factory, args| {
                let config = world::config(&project)?;
                pfx_workbench::edit_project(project.root(), factory, config, args)
            },
        )
        .usage("edit", pfx_workbench::EDIT_USAGE);
}
```

`<game> edit` opens the window on the game's project; `pfx gui <scene, .pitofx, recipe or folder>` opens pfx's own window with no game linked in.

## Play: a WASM module or the game's own logic

There are two ways to play a scene in the window, and a game picks by how it opens it.

- **The game's own Rust logic.** In `<game> edit`, F5 plays the open scene with the game itself: `run_project` hands the window the game's factory (the `Factory` from `pfx_game::factory`), so Play runs the same `Game` the shipped binary runs, and Stop (F8) returns the editor exactly where it was. No module is used.
- **A WASM module.** In `pfx gui`, F5 plays the scene with a game's WebAssembly component, hosted in pfx-mod's sandbox. The module implements the `game` world of `pfx:play/game@0.1.0` (`crates/game/wit/play.wit`): it imports `scene` (`find`, `position`, `move-to`, `set-hidden`), `input` (`held`), `clock` (`ticks`, `step`), `hud` (`text`) and pfx-mod's `log`, and exports `start` and `tick`. The scene names it with `[play] module = "<path>.wasm"`, or the window picks one (`pfx gui <scene> --play <module.wasm>`, the module chip, F4, or a dropped file). `crates/game/examples/marker` is a neutral module to start from:

```sh
cargo run -p pfx-game --features modules --example marker -- tmp/play/marker.wasm
pfx gui content/room.pitofx --play tmp/play/marker.wasm
```

[play-modules.md](play-modules.md) has the module's world, limits and API; [projects.md](projects.md) has the game's project and its commands.
