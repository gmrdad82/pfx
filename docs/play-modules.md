# Play with a game's module

`pfx edit` plays the open scene with a game's WebAssembly module: press Play (F5) and the scene runs as a game whose logic is the module's, in the editor's viewport, with the editor's renderer, frame and input; press Stop (F8) and the editor is back exactly where it was. The game's logic stays in the game's repository, built there into a `.wasm` file; pfx knows nothing of any game. It hosts the module in [pfx-mod](mods.md)'s sandbox and gives it a small, neutral world: find objects by name, move and hide them, read the keys held, the tick and its step, and write a few lines of text over the frame.

Without a module, Play runs the scene with the default game, as [editor.md](editor.md) ("Play mode") says. In `<game> edit` the game's own factory plays and a module is not used ([projects.md](projects.md)).

## Choosing the module

| Where | How |
|---|---|
| the scene | `[play] module = "<path>.wasm"` in the scene file, a path from the project's root |
| the command line | `pfx edit <scene.toml or folder> --play <module.wasm>`, which wins over the scene's key for the session |
| the toolbar | the `module` button, or F4 while stopped: none, then each `.wasm` file the project holds, then none again |
| the window | drop a `.wasm` file on it while stopped |

```toml
format = 1

[play]
module = "modules/marker.wasm"
```

- The key takes one field, `module`; it is in the scene's [JSON Schema](scene-schema.md) (`docs/schema/scene.schema.json`), and the checker refuses a path that leaves the project or does not end in `.wasm`. A module that is not built yet is not a finding: it is a build output, read when Play starts.
- Only a scene file sets it, once per scene: a second `[play]` in an included scene is refused as held twice, like `[physics]`.
- The scene's key is read again at each Play, so a changed key plays from the next press. A module from `--play`, the button or a drop stays for the session, and across scene switches in a project.
- The button shows the chosen module's name (`module marker`, lit) or `module none`. It is drawn only when a module is chosen or the project holds a `.wasm` file, found once when a scene opens (up to 32 files, 6 folders deep, in name order, skipping hidden folders and cargo's `deps`, `build` and `incremental`), so the editor of a project without one keeps its toolbar.

The log says what F5 will play (`play: F5 plays the scene with module modules/marker.wasm`) and, on Play, ``play: module `marker` runs the game, under pfx-mod's limits``.

## The world

The module is a WebAssembly component of the world `game` in `crates/game/wit/play.wit` (`pfx_game::wasm::WIT`):

```wit
package pfx:play@0.1.0;

interface scene {
  find: func(name: string) -> option<u32>;
  position: func(object: u32) -> tuple<f32, f32, f32>;
  move-to: func(object: u32, x: f32, y: f32, z: f32);
  set-hidden: func(object: u32, hidden: bool);
}

interface input {
  held: func(key: string) -> bool;
}

interface clock {
  ticks: func() -> u64;
  step: func() -> f32;
}

interface hud {
  text: func(line: u32, text: string);
}

world game {
  import scene;
  import input;
  import clock;
  import hud;
  import pfx:mods/log@0.1.0;

  export start: func();
  export tick: func();
}
```

- **`start`** runs once when Play starts, after the world is built from the edited scene; **`tick`** runs every fixed tick of the play clock (the scene's physics rate, 60 by default), after the scene's own movers and bodies.
- **Objects** are the scene's, by name. `find` returns an object's number, or none; `position` reads its world position and `move-to` sets it; `set-hidden` hides or shows it. Moves and hides land in the world when the call returns, in the order they were made; `position` already reads a move made in the same call.
- **Keys** are named as pfx names them: `Left`, `Right`, `Up`, `Down`, `Space`, `Enter`, `A` to `Z`, `Digit0` to `Digit9`, `F1` to `F12`, and the rest of `pfx_input::Key`. `held` answers for the keys held at that tick; the input comes from the viewport, as the default game's does.
- **The clock:** `ticks` is the ticks done so far, `step` the seconds one tick covers.
- **The HUD:** `text` sets one of 8 lines, drawn top left in the editor's monospace font (JetBrains Mono, 28 units at the play view's UI scale). An empty line draws nothing.
- **The log:** `pfx:mods/log` lines (16 a tick, 256 bytes each) go to the editor's log as ``modules: module `marker`: <line>``.
- The module gets nothing else: no WASI, no files, clock, network or randomness of the host.

## Limits

The module is untrusted, so it runs with pfx-mod's limits ([mods.md](mods.md), "The limits"), the defaults with the log on (`pfx_game::wasm::limits()`):

| | |
|---|---|
| fuel | 10,000,000 a call; `start` and each `tick` are one call |
| memory | 64 MiB of linear memory, 4 memories, 16 tables of 10,000 elements, 32 instances |
| stack | 512 KiB, and 1,000 nested calls |
| the module | 16 MiB |
| strings | one fuel a byte for every name, key, HUD line and log line; a name or key over 256 bytes traps, a HUD line is cut to 256 bytes |

A call traps, too, on a key pfx does not name, on an object number `find` never returned, on a position that is not finite and on a HUD line past the eighth.

**When the module fails** (a trap, the fuel spent, a cap passed), play stops as F8 would, with one line in the log, and the editor keeps running:

```
play: the game failed: mod `marker` ran out of fuel after <fuel> fuel in function <n> at offset <offset>; it is stopped and disabled for this run; play stopped
```

**When it does not load** (a missing file, bytes that are not a module, an import outside the world, a missing `start` or `tick`), Play does not start, and the log says why:

```
play: module `marker` cannot be read: <path>: No such file or directory (os error 2); it stays off until its file loads; play did not start
```

## What Stop restores

Play never touches the edited scene. The world the module moves is a runtime copy of the scene's snapshot, the editor's viewport keeps its camera, selection and pixels, and nothing is written: the files are byte for byte as before play, and the inspector's play edits are discarded unless Keep (F10) writes them ([editor.md](editor.md), "Play mode"). The next Play starts again from the edited scene at tick 0, with a fresh module instance.

**Live rebuilds.** While the scene plays, the editor watches the module's file. A rebuilt module swaps in at the next tick ([mods.md](mods.md), "Gameplay modules"): with its state through `save` and `restore` when it exports them, or fresh, and then its `start` runs again on the running world. A build that does not load leaves the running module as it was, with a warning.

## The API

`pfx_editor::play::Play` is the hook a toolbar calls. It holds the chosen module and runs a play session headless; the editor's window hands the same game to pfx-game's `Driver` and `Painter` to draw it.

| Call | What it does |
|---|---|
| `Play::of(scene)` | the module the scene's `[play]` key names, or none |
| `module()`, `from()`, `set_module(module, from)` | the chosen module and where it came from (`From::Scene`, `Flag` or `Picked`) |
| `follow(scene)` | takes the scene's key again, unless the module came from the flag or a pick |
| `look(root)`, `found()`, `offered()`, `next()`, `name()` | the project's `.wasm` files, whether the toolbar offers a module, the next choice in the cycle, and the chosen module's id |
| `game(family)` | the module's game (`pfx_game::wasm::ModuleGame`) with its HUD in that font family, `None` without a module, or the one message why it does not load |
| `start(scene, staged, options)` | builds the game and a `PlaySession` (`docs/play.md`) on the scene; `staged` may be `None` |
| `feed(event)` | a viewport input event for the next tick |
| `frame(seconds)` | advances the clock; `Err(message)` when the game failed, and the session is then stopped |
| `stop()` | stops the session and hands back its `Stopped` (the snapshot, the queued scene and the pending patch) |
| `is_running()`, `session()`, `session_mut()` | whether a session runs, and the session, for the world, the UI and the inspector |

`pfx_game::wasm` (pfx-game's `modules` feature) is the host side, for any runtime that wants to play such a module:

| Item | What it is |
|---|---|
| `ModuleGame::open(path)`, `open_with(path, limits)` | a `Game` that hosts the module at `path`; `refused()` says why it did not load |
| `ModuleGame::font(family)` | the font family of its HUD lines |
| `WIT`, `VERSION`, `api()`, `limits()` | the world, its version (`0.1.0`), its `pfx_mod::Api` and the default limits |
| `embed(core)` | gives a core module built against the world its component type, as `wasm-tools component embed` does |
| `module_id(path)` | the module's id: its file's stem in lower case, other characters as `-` |

## The example

`crates/game/examples/marker/marker.wat` is a neutral module in WebAssembly text: the arrow keys move the object named `marker` at 2 units a second over the ground (Left and Right along x, Up and Down along z), and the HUD's first line counts the ticks it has moved, `score N`. A scene without a `marker` logs one line and plays on.

```sh
cargo run -p pfx-game --features modules --example marker -- tmp/play/marker.wasm
pfx edit content/room.scene.toml --play tmp/play/marker.wasm
```

The example parses the text with `wat` and calls `embed`. A game writes its module in Rust instead: a `cdylib` crate with `wit-bindgen` generating the `game` world from a copy of `play.wit` (and pfx-mod's `log.wit` in its `wit/deps/`), built with `cargo build --target wasm32-unknown-unknown --release`; the host makes the component itself. The `wasm32-wasip2` target is not usable, because its standard library imports WASI.

## Tests

- `crates/editor/src/play_tests.rs` (in the gate, no GPU): the example module, built from its source, plays a copy of the editor's fixture room headless through `Play` for 30 frames with Right held; the marker moves along x only and the HUD counts the ticks; Stop hands back the edited scene, the editor's scene is unchanged and every file is byte for byte as before; a second Play starts the marker where the scene put it, with `score 0`. A module that never returns stops play with `ran out of fuel` and hands back its session, and bytes that are not a module refuse to start with the module's name.
- `src/main.rs`: `pfx edit`'s `--play` argument and its refusals.

```
cargo test -q -p pfx-editor --lib play_tests
```
