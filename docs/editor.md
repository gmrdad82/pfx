# pfx edit

`pfx edit <scene.toml>` is the engine's editor: a window on a scene's own files, with the live renderer in the middle, an outliner, an inspector, a TOML panel, gizmos, a log, a trace preview and play mode. Every change lands in the files that define it, as a `SceneEdit` patch group (`docs/scenes.md`, "Editing") on one shared undo history, so a running app or game hot-reloads the same bytes the editor wrote. It is the one window the engine opens. Its tests run headless (below), and `bin/rig` captures it in a nested compositor.

`pfx edit <project folder>` opens a project: its scenes and content folders in the outliner, each scene editable as below; and `<game> edit` opens a game made by `pfx new` the same way, with F5 playing the game's own logic. Both are in `docs/projects.md`.

The editor is the crate `pfx-editor` (`crates/editor`), behind the root package's `editor` feature. That feature is a default one, so the installed pfx has it. The library crates apps link never pull egui or winit: apps build the engine with its default features off, and no engine library depends on the editor crate.

## The editor crates

The editor runs on five crates of pfx's own, `crates/editor-*`, which other editors on the engine share (Manfred among them), so they look and work alike; `docs/editor-crates.md` says what each holds and how an editor outside this repository takes them. pfx-editor takes all five by path, and pfx-theme takes `pfx-editor-style`, so the workspace holds one `Theme` type and every widget finds the theme pfx installs.

| Crate | What pfx edit takes from it |
|---|---|
| `pfx-editor-style` | the `Theme` every widget paints from (pfx brings its own, below); the header, keycap, pill and number widgets |
| `pfx-editor-shell` | the `App` trait, the window host, the file watch, the pgpu watch, the typed input script and the offscreen shot harness |
| `pfx-editor-viewport` | the `Viewport`: the engine frame in egui through pfx-live, the orbit camera, picking (the id buffer for clicks, a CPU ray for hover), the snapped move, rotate and scale gizmo, the selection and hover outlines, the grid and axes, and the gizmo's commit as a `ScenePatch` |
| `pfx-editor-doc` | `Files` (the open texts and their stamped saves), `History` (one labelled undo stack over any `Edit`) |
| `pfx-editor-toml` | the `Source` TOML panel: highlighted editing whose typing lands as text splices on the same `History`, with a `Check`'s problems underlined and listed |

pfx keeps only what is its own: the outliner, the inspector, the log, the toolbar, play, the trace preview, the pgpu status line and the theme. Nothing of the old viewport, camera or gizmo is left in pfx.

**One engine in the build.** The editor crates are workspace members and take `pfx-gpu`, `pfx-live` and `pfx-load` by path, so every build holds one copy of each engine crate and the viewport draws on this tree's `Gpu`; `cargo tree -d --workspace` names no pfx crate and one wgpu. The editor takes `pfx-scene` directly, at the rev pfx-load pins, for its `check`.

## The look: pfx's theme

The editor crates hold only what is common: `pfx-editor-style`'s `Theme` has no default and bundles no fonts. pfx's theme is complete in itself, in one place, the `pfx-theme` crate (`crates/theme`, `pfx_theme::pfx()`, which the dev HUD shares; `crates/editor/src/theme.rs` re-exports it), and `Session::theme` hands it to the shell, which applies it before the first pass in the window and in the shot harness. Every widget, pfx's own included, reads it back with `Theme::of(ctx)`; nothing in `ui.rs` names a colour. It follows one reference: near-black, violet light, one lime accent, hairline line work. Manfred keeps its amber-on-blueprint look in its own theme.

| Role | Colour | Used for |
|---|---|---|
| `bg` | `#0E0E0E` | the window, the panels and the viewport surround |
| `surface` | `#1D1928` | cards, chips, keycaps, the viewport badges |
| `well` | `#080808` | number fields and text wells |
| `line` | `#2E2D30` | separators and hairline borders |
| `track` | `#3B3A3A` | tracks and inactive strokes |
| `text`, `text2`, `muted`, `dim` | `#EEEDED`, `#9E9BA1`, `#6F6D70`, `#504E53` | text by weight; `dim` is file names and the status lines |
| `code` | `#D4D1D8` | values and a file's text |
| `accent` | `#5331A8` | hover borders; the viewport's selection and hover outlines, and their highlight (its linear colour × 0.35 selected, × 0.12 hovered, the editor crates') |
| `accent_bright` | `#542ACD` | active and selected fills |
| `accent_deep` | `#342850` | selection and hover backgrounds, striped rows |
| `on_accent` | `#EEEDED` | text on `accent_bright` fills |
| `secondary`, `secondary_soft` | `#9B80C6`, `#B6ABCB` | links, the gizmo's Z axis, TOML keys and tables |
| `hot`, `hot_dim` | `#CEF03B`, `#ACC33B` | the one hot accent, used sparingly: focus, the caret, the playing badge; `hot_dim` the paused and trace badges, TOML strings; `ok` the gizmo's Y axis |
| `error`, `warning` | `#FF4D6D`, `#FF9F43` | refusals and errors, the gizmo's X axis; warnings (added beyond the reference) |
| `ok_tint`, `error_tint` | `#23280F`, `#3A1820` | tinted backgrounds and the diff's added and removed lines |

A held or hovered gizmo axis lights in `text`. The metrics are the editor crates' usual ones (code 11, small 12, body 13, heading 16, title 20; radius 2, hairline stroke 1, row 19).

**Type.**
- Titles (`pfx edit`, the panel headers) are Exo 2 Light, uppercase, tracked 0.3 em (`Caps` with egui's `extra_letter_spacing`).
- Everything else is JetBrains Mono: labels, values, the log, the inspector, a file's text.
- Data labels (the inspector's keys, the outliner's sections, the viewport badges) are small, uppercase and tracked 0.12 em.

**Fonts.** Both are bundled in the theme crate under `crates/theme/fonts/`, each from its upstream release with its `OFL.txt` and a `SOURCE.md` (URL, version, sha256) beside it, and listed in `tests/agnostic/fixtures.txt`:
- `jetbrains-mono/`: JetBrains Mono Regular and Bold, release v2.304;
- `exo-2/`: Exo 2 Light 2.001, from NDISCOVER/Exo-2.0 at commit `182060cd` (the project has no releases). The licence text also travels in the theme's `Display`.

## The host

`Session` (`session.rs`) is the shell's `App`, and `pfx edit` hands it to `pfx_editor_shell::window`. The shell owns the window, winit, egui-wgpu, redraw on demand and the file watch; the session keeps only what is pfx's own:

- `ui` reads the pass's time step from egui (`unstable_dt`, capped at 0.25 s), feeds play's input, then draws the editor.
- `viewport` is true. `frame` hands `pfx-editor-viewport`'s `Viewport::frame` the shell's `Gpu` and egui renderer while the editor is stopped: the viewport opens pfx-live's renderer at its first frame (and again after `forget`), draws only when the scene, the camera, the size or a highlight changed (frames on demand, which also settle temporal AA), reads a pick back from the id buffer and registers its image as an egui texture. Beside it, `View` (`view.rs`) holds play (pfx-game's `Driver` and `Painter`, below) and the trace preview, drawn into their own surface: `frame` applies play's requests, advances play by the pass's time step and draws a play or trace frame. It returns true while either wants more frames.
- `name` is `pfx edit` (the window title, the stderr tag and the pgpu job name), and `label` the scene's file name, so the title reads `pfx edit · room.scene.toml`.
- `watched` is the scene's files (the scene, its includes and libraries), and `opened` tells the shell when that list changed after a reload. `changed` asks the next pass to `check` the viewport's `SceneWatch` at once, so an outside edit reloads without waiting for its debounce.
- `waker` gives the viewport's watch the shell's wake and starts the pgpu status line's reader (below).
- `timing` gives the GPU time and dropped timings of whichever renderer drew (the editor crates', or play's), and whether it has drawn, for the shell's 240-frame log line.
- `forget` stops play, drops the play and trace view and the editor crates' renderer, which the shot harness does before each shot on its own `Gpu`.
- Play mode's input: the shell hands the app egui's input, not winit's events, so `input.rs` maps egui's keys (the physical key when egui has one), buttons, wheel (40 points a line), pointer and focus to `InputEvent`s, and a change in Shift, Ctrl or Alt to their left keys. The pointer reaches the game in the play view's pixels (the window's, less the viewport panel's corner), and the play's `Driver` maps it through the frame's bars to layout units, as a game's window does.
- Closing the window drops the session, which stops play (`pgpu play off`) and ends an open edit group.

### pgpu: one owner for each call

| Call | Owner | What it does |
|---|---|---|
| `pgpu play on`, `pgpu play off` | pfx, through the editor's `PgpuPlay` hook (`pgpu.rs`) | holds pgpu's render jobs while the editor plays, from F5 to stop or close |
| `pgpu status --json` every second | the shell's pgpu watch | tells the session `Free`, `Play` or `Queued` |
| `pgpu status --json` every 3 s | pfx's status line (`pgpu.rs`), read through the editor crates' `pgpu::fetch` | the log's `gpu 42% · 2 queued · …` line |

The shell's watch says `Play` whenever any game is in front, the editor's own play included. The session decides what that means: a `Play` that arrives while the editor is not playing is another game, and the viewport waits (the session hands the editor crates' viewport `Pgpu::Play`, so it draws no live frames and shows its "paused while a game plays" notice; no trace samples; the log says `pgpu: a game is playing; the viewport waits`) until the watch says otherwise. While the editor plays, `Play` is its own and nothing waits, and pressing play takes the viewport out of a wait. `PFX_EDIT_PGPU=off` turns off `PgpuPlay`, the status line and the wait; the shell's read-only watch still runs. They are also off when no `pgpu` is on `PATH`, so a stranger's `pfx edit` or `<game> edit` never tries to spawn a tool they don't have.

## The window

```
┌ toolbar: G move · R rotate · S scale · L space · C camera · T trace · F5 play · F6 step · F8 stop · undo · redo · F10 keep ┐
├ 1 outliner ─┬──────────── viewport ────────────┬─ 2 inspector ┤
│ objects     │                                  │ at   x y z   │
│ meshes      │   the live renderer on the scene │ rotate       │
│ lights      │   gizmo over the selection       │ material ▾   │
│ materials   │                                  │ …            │
│ world       │                                  │ 3 source:    │
│ files       │                                  │ a file's TOML│
├ 4 log ──────┴──────────────────────────────────┴──────────────┤
│ file:line errors · edits · play · pgpu status      pfx v0.25.0 │
└─────────────────────────────────────────────────────────────────┘
```

- **Outliner** (`outline.rs`): objects as a tree by `parent`, meshes with their glTF nodes, lights and emitters, the scene's sounds (when it has any), the library's materials, the world (sun, sky, haze, camera, finish), the project's tunables by group (when it declares any) and the files (the scene, its includes and libraries, and the file that holds the tunables). A scene with `[[tiles]]` gets a `tiles` section, one row per layer with its cell count (`ground  ·  7 cells`); the layers' cells are not listed among the objects. A project's `content` section lists its assets; a `*.prefab.toml`, `.gltf` or `.glb` there can be dragged into the viewport (Level building, below). Double-click selects an item and frames it in the editor camera (an object; anything else frames the whole scene).
- **Inspector** (`inspect.rs`): one row per key of the selection, each bound to a `SceneEdit` target and path:
  - an object's `at`, `rotate`, `scale`, `material`, `shadow`, `two_sided`, `hidden`, `clip` and `parent`, and with an `[object.body]` its `body.mass` (written as the `density` that gives it), `body.friction`, `body.restitution`, `body.damping`, `body.velocity` and its shape's size (`body.half`, or `body.radius` and `body.half_height`);
  - a sound's `volume`, `pan` and `loop`;
  - a group of tunables: each by name, a number (ranged by its min and max), a switch or a vector; a stopped edit rewrites its `default` in `project.toml` with the text kept, as one undo step;
  - a material's `family`, `base`, `roughness`, `metalness`, `specular`, `clearcoat`, `clearcoat_roughness`, `sheen`, `transmission`, `ior`, `thickness`, `subsurface`, `absorption` and `emission`, in its library;
  - a light's and an emitter's keys; a mesh node's overrides;
  - the sun (its model's keys), the sky (its kind's keys), the haze, the camera (with the finish's `exposure` among its rows), and every number, switch and text of `[finish]`.

  Each row shows the file that defines its item. A number is dragged or typed (click, type, Enter). A drag is one undo step. A value that is already equal writes nothing, and a ranged key is clamped (`roughness` 0 to 1). Choosing `(none)` removes the key.
- **TOML panel** (`3 source`, in the inspector's place when a file is selected in the outliner): the editor crates' `Source` on that file's text in `Files`. Typing lands after a short pause as a text splice on the shared `History` (a run of typing is one undo step), saved as below, but only when the text parses and pfx-scene's `check` (`check.rs`, `SceneCheck`, which runs `pfx_scene::check` on the buffer with the project's other files) finds no error; until then the problems are underlined and listed under the text at line and column, and nothing is written. Problems of the scene's other files are listed above the text as `file:line:column: error: message`, and logged when the editor opens. A file that changed underneath an unapplied edit offers "take theirs" or "keep mine". The panel's toolbar (diff, open in `$EDITOR`) is off. Play makes it read-only.
- **The scene camera's frame**: in a project with a screen policy (`[authoring.pfx.screen]`, `docs/projects.md`), looking through the scene's `[camera]` shows the frame the game renders, Blender-style: the policy's first aspect for the device (`screens::detect`, desktop unless a Deck or `PFX_DEVICE` says otherwise) goes to the editor crates' `Viewport::set_frame_aspect`, the view widens to fit that frame inside the panel, and the bands outside it are dimmed. The editor's own camera (C) fills the panel with no frame, and a project with no policy, or a plain `pfx edit <scene.toml>`, shows none. An invalid policy is logged and shows none. Points are projected with the editor crates' `viewport.lens()`, which follows the framed view.
- **Viewport**: the editor crates' `Viewport`, drawn by pfx-live only when something changed (an edit, a reload, the camera, a resize, the selection or hover). The selection and the hovered object are highlighted through `Staged::set_override` (the theme's accent) and outlined, and no file changes for it. The gizmo's badge is top right, the axes bottom left, a notice (a broken scene, a refusal, a wait for pgpu) top left, and pfx's badges (the camera, trace, playing or paused) bottom right.
- **Log**: edits and undo steps by their labels, refusals as `file:line: key: reason`, `SceneWatch` errors as `file:line: message` while the last good scene keeps drawing, pfx-scene's problems at open, play and stop, and the pgpu status line (load, queue, the game playing), read every 3 s. Its header row ends with the version, dim at its right end (`Editor::app`): `pfx v<version>` in `pfx edit`, and the game's name and version from its `Config` in `<game> edit`.

## Controls

| Input | Does |
|---|---|
| left click in the viewport | picks the object under the pointer (the renderer's id buffer once it has drawn, else a CPU ray); a click that picks nothing clears the selection |
| hover in the viewport | outlines and lightly highlights the object under the pointer (CPU ray) |
| left drag on a gizmo handle | moves, rotates or scales along that axis, previewed live and written once on release; Ctrl snaps |
| left drag elsewhere, or middle drag | orbits the editor camera; with Shift (or middle with Shift), pans |
| wheel | dollies |
| G, R, S (pointer in the viewport) | move, rotate, scale gizmo; L switches world and local space |
| arrows, + and − (pointer in the viewport) | orbit and dolly from the keyboard |
| F | frames the selection (the whole scene when nothing is selected) |
| Home (pointer in the viewport) | looks through the scene's `[camera]` again |
| C | the scene's `[camera]` or the editor camera; moving the view takes the scene's camera over as the editor camera |
| J, K | the next and previous item in the outliner |
| T | the trace preview |
| Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y | undo, redo on the one history, whichever panel made the step (inside the TOML panel its own chords do the same) |
| Ctrl+/ (TOML panel) | comments or uncomments the selected lines |
| Escape | cancels a gizmo drag (back to the scene before it, nothing written), else clears the selection |
| M | mutes or unmutes play's sound |
| F5, F7, F6, F8 | play or resume (F5 pauses while playing), pause, step one tick while paused, stop |
| F9 | while a scene plays or is paused, save the play session's input so far (see Play) |
| F10 | while a scene plays or is paused, stop and keep the play edits (see Play) |
| drag a prefab or glTF from `content` into the viewport | places it under the pointer (Level building) |
| P (pointer in the viewport) | places the stamp, the selected or last placed asset, under the pointer |
| N, U, V | grid, surface and vertex snapping on and off |
| [ and ] | halve and double the grid step |
| Shift+click (outliner or viewport) | adds an object to the selection, or takes it out |
| Shift+D | duplicates the selected objects by the duplicate offset |
| B | the tile brush on and off; with it on, E switches paint and erase and I picks a cell's prefab |
| drag, Shift+drag, Ctrl+drag (brush on) | paints freehand, a line or a rectangle of cells; middle drag turns the view, Shift+middle pans, the wheel dollies |

The editor's own keys are off while a text field or the TOML panel has focus; the viewport's focus does not count, so F5 or Ctrl+Z work after a click in it.

## Editing and hot reload

The editor holds the scene's files twice over, each for its own job: `SceneEdit` (`Editor::edit`) is the model the outliner and inspector read and the maker of patch groups, and the editor crates' `Files` (`Editor::files`) holds the texts every panel edits through the one `History` (`Editor::history`).

- **One history.** The inspector's edits, the gizmo's drags and the TOML panel's typing all land on `History` as edits of `Files`: an inspector or gizmo edit as a `ScenePatch` (the editor crates' `Edit` adapter over a `PatchGroup`), typing as a `TextSplice`. Undo and redo take back the last step whichever panel made it, and a step another file has moved past refuses with the file's name.
- **The dry run, then one write.** An inspector edit is `SceneEdit::dry_run` of the row's change: a `PatchGroup` with each touched file's text before and after, checked as a whole and written nowhere. `History::apply` lands it in `Files`, and the editor's save (`Files::save`, a temporary file and a rename, stamped) is the only write, once per touched file. The viewport's gizmo does the same with its own `SceneEdit`, and the TOML panel's splices are saved the same way. Every write is listed in `Editor::saves`.
- **A number drag** is one undo step: each step is a dry run applied to `Files` and saved at once, so the viewport follows, and on release the whole drag is folded into one `ScenePatch` from the texts before it to the texts after it.
- **After a write** the editor reloads `SceneEdit` from the files, refreshes the outliner and inspector, and asks the viewport's `SceneWatch` to `check` at once, so the viewport shows the edit on the next frame and `Staged::apply` touches only what changed.
- **Hot reload never takes the editor's own writes twice.** The viewport's watch reports every reload, the editor's own included; `Files::disk` tells a file holding the text the editor last saved (its own write, nothing to do) from one changed outside. A change made outside (a text editor, another tool) is read into `Files`, the undo steps that no longer apply are dropped (`History::prune`; the log says how many), and `SceneEdit` reloads. A TOML panel holding unapplied typing over such a file shows "the file changed underneath".
- **Refusals.** A refused edit writes nothing, and the log names the file, line, key and reason; the viewport shows a refused drag as a notice.

## Level building

`level.rs` and `ui_level.rs` hold it; `snap.rs` the snapping math. Every change lands as one `SceneEdit` dry run on the shared `History`, so each is one undo step written once, whichever way it was made.

### Placing

A `*.prefab.toml` becomes a placement (`[[object]]` with `prefab`), a `.gltf` or `.glb` an object on its mesh (an existing `[mesh.*]` of that file and no `node` is reused; otherwise a mesh entry is added in the same step). The new object takes the file's stem as its name (`block`, then `block 2`, …), an id derived from the scene's path and that name (`Id::derive`, a salt added if the id is taken), and is selected. The label is `place <name>`.

- **By drag:** drag the asset's row from the `content` section onto the viewport. While it is held over the viewport a ring and a short normal mark where it would land; release places it there.
- **By key:** selecting a placeable asset makes it the stamp; P places it under the pointer, again and again. The last placed asset stays the stamp; Escape clears it.
- **Where it lands** (`Editor::aim`): the pointer's ray meets the surface under it (below), or the ground plane `y = 0` when surface snapping is off or the ray meets nothing; then vertex and grid snapping apply, and the asset is lifted along the surface's normal by its own extent (`pfx_load::scene::tiles::extent`, the asset's bounds), so a block whose origin is its centre sits on the floor instead of in it. Positions are written to four decimals.

### Snapping

The level strip, top left of the viewport, shows the snapping and the brush whenever any of them is away from its default, a stamp is set or several objects are selected; with everything at rest the window looks as before.

| Snap | Default | What it does |
|---|---|---|
| grid (N), step (`[`, `]`, or the strip's number) | off, 0.5 | rounds a placement to the step on the two axes across the surface's normal; on the floor, X and Z |
| surface (U) | on | places onto the nearest triangle under the pointer: `Surfaces` casts the pointer's ray against every drawn triangle of the scene (the same exact cast the editor crates' CPU pick makes) and returns the object, point and normal |
| normal | off | turns the placement so its +Y is the surface's normal (`snap::upright`) |
| vertex (V) | off | moves the placement onto the nearest mesh vertex within 12 points of the pointer, as Blender's vertex snap |

**The gizmo.** The viewport's gizmo snaps a move by a step while Ctrl is held, and its step is the viewport's own (0.01 m). With grid snapping on, pfx sets the gizmo's step to the grid step, so a Ctrl-drag moves by whole grid steps. Snapping a gizmo move to a surface or a vertex, and snapping without Ctrl, need the editor crates' drag to take a snap from its host; pfx has the math (`Surfaces::cast`, `Surfaces::vertex`, `snap::grid`) ready for that hook, which pfx asks of the kit.

**The id buffer.** The viewport reads its GPU id buffer only for a click's pick, and does not expose it; the surface cast runs on the CPU over the same triangles, so it finds the same object the id buffer would.

### Tile layers and the brush

A scene's `[[tiles]]` layers (`docs/scenes.md`) are painted with the brush. B turns it on; the inspector then shows the brush: the layer (the one selected in the `tiles` section, else the first), the tool (paint, erase, pick) and the palette, with the prefab it paints. The brush paints the prefab chosen in the palette, else the stamp when it is a prefab, else the layer's first palette entry. A prefab not in the palette joins it, under the first free key of `#@%&*+=A…`, in the same undo step.

- A drag paints every cell it passes, Shift+drag a line (Bresenham) from the first cell to the last, Ctrl+drag a filled rectangle, and a click one cell; the cells are outlined while the stroke is held, and nothing is written until release. The whole stroke is one undo step: `paint 6 tiles in ground`, `erase 1 tile in ground`.
- Erase (E) empties cells; pick (I) takes a cell's prefab for the brush and goes back to paint.
- The pointer's ray meets the layer's plane, and the cell is the one whose position is nearest (`tiles::cell_at`).
- The brush writes through `SceneEdit` with `Target::Tiles`: a changed row as that row's whole new string, a grown layer's `rows` or `cells` as a whole, a new palette key, and, when a `rows` layer grows down or left, its new `origin`, so nothing already painted moves.
- While the brush is on, the left button belongs to it; the middle button turns the view (Shift pans) and the wheel dollies.
- A cell's objects can be clicked in the viewport; the inspector then names the cell and points to the brush instead of offering rows no file holds.

### Duplicate, align and distribute

Shift+click in the outliner or the viewport builds a selection of several objects, in the order they were picked; the last is the one the gizmo and the inspector show. With two or more, the inspector shows the arrange panel:

- **Duplicate** (Shift+D, or the panel's button) copies each selected object with `SceneEdit::duplicate_object` and moves the copy by the duplicate offset (default `[1, 0, 0]`, set in the panel), all in one step; the copies are named from the original's stem (`block 4`) and become the selection. An object a prefab placed is duplicated through its placement.
- **Align to first or last** sets every selected object's world X, Y or Z to that of the first or last picked, written as each object's own `at` in its parent's space.
- **Distribute** spaces three or more objects evenly along X, Y or Z between the two outermost, which stay.
- Tile cells are skipped with a note: they belong to their layer and are painted.

### Ten thousand cells

A 100 × 100 layer of one prefab (10,002 objects with the floor and the ledge) against the same level without it, measured by `crates/editor/tests/level.rs` under pgpu on the RX 9060 XT:

| | 10 objects | 10,000 cells |
|---|---|---|
| open the editor (debug build) | 14 ms | 97 ms |
| GPU time a frame | 0.22 ms | 0.58 ms |
| an offscreen pass while the camera turns, debug build | 34.6 ms | 37.9 ms (+3.3 ms) |
| the same, release build | 4.70 ms | 4.88 ms (+0.2 ms) |

The budget the test holds: below 2 s to open, a GPU frame below 8 ms, and the cells adding below one 60 fps frame (16.7 ms) to a pass. A pass's own cost is mostly the harness (a fresh offscreen frame of the whole window); the cells add little because they are instances of one uploaded mesh, the outliner lists the layer and not its cells, and the UI pass with the editor crates' hover cast stays under a quarter of a millisecond. Run-to-run noise on a busy machine is several milliseconds, which is why the test compares the two scenes in one run.

## Trace preview

T traces the scene the viewport shows from its current camera with `pfx_trace`: one paced sample per frame (the `Pacer` keeps every submission within 2–6 ms), up to 256 samples, shown with a simple preview tonemap over the viewport (the camera still moves under it). Any change of the camera or the scene starts it again. The editor runs outside pgpu, its trace preview included, and keeps its submissions short.

## Play mode

Play runs the game exactly as its runtime does: the editor's play is pfx-game's own `Driver` and `Painter` (`docs/game.md`), the same two a game's window and `Headless` run, drawing into the play view's surface. Without a game factory (`pfx edit <scene.toml>`, `pfx edit <folder>`) the game is `SceneGame`, the default game of `pfx-play` (`docs/play.md`): movers, bodies and sounds as the scene defines them. With one (`run_project`, `<game> edit`, `docs/projects.md`) it is the game's own logic, a fresh game from the factory at each F5.

- **Play** (F5) builds a `Painter` on the play view's `Gpu` and size from the game's `Config` (its fonts, icons, sprites, environment, exposure, filter, render scale and budget; without a game, a config named after the project with its screen policy), stages the edited scene on the painter's own renderer beside the editor crates', and starts `PlaySession::play_hooked(…, PgpuPlay::editor(&scene))` inside a `Driver`, so pgpu holds its render jobs while the editor plays (`pgpu play on --pid <pid> --name "pfx edit: <scene file>"`). The viewport shows the game's frame (its badge says game camera) with no gizmo or highlight, and the window's keys, buttons and wheel go to the game as `InputEvent`s.
- **The frame** is `Painter::draw`, the one function pfx-game's window and `Headless` call too, so the editor and the runtime cannot draw differently: the bars or the backdrop, the framed view (the 3D frame, or a flat game's clear colour), `Game::paint`, then `Game::ui` (draws, labels, prompts, groups, icons, sprites, the look and the backdrop) laid out for the play view, then the HUD. The play view fits the game's screen policy (`Config::screen`; without a game, the project's `[authoring.pfx.screen]`), so a 16:9 game in a wider panel plays pillarboxed in its own frame, at the layout its window would have; a plain `pfx edit <scene.toml>`, or a project with no policy, fills the panel. The settings the game keeps (`Game::settings`, or the config's) apply as in its window: volumes to the mixer, binds to the action map.
- **Keycap labels** follow the player's layout as a game's window does (`docs/game.md`, `docs/input.md`). The play's `Driver` gets a `LayoutSource`: the game's `Config::layout` when it sets one (taken once when the session opens a game and shared by every play), else `WindowsLayout` on Windows, and none elsewhere. Where there is none, the editor learns each label from egui's key events: a key press that carries a physical key, followed by the text it typed, goes to `KeyLabels::learn`, the same call `pfx_input::winit::learn` makes for winit's events, so the caps follow the layout in the same way (Q reads A under AZERTY). A chord (Ctrl or Alt held), a repeat, an event with no physical key, and a shifted key that is not a letter teach nothing. The learned labels are kept for the session, so the next F5 starts with them.
- **Warm-up:** before the first frame each play runs `Game::warm` through pfx-game's `Warmup`, one call a pass until `Done` or the config's `warm_budget` (measured on play's clock), and nothing new shows until it is done; the viewport keeps its last frame meanwhile. Play's clock starts at the pass the warm-up finishes, so the first frame advances by that pass's time step and the warm-up makes no catch-up ticks. The painter waits for each frame (`Painter::wait_for_frames`), so auto exposure meters every frame and a script plays the same pixels every run; a game's window pipelines its frames instead.
- **A flat game** (`Config::flat()`, or a project with `[authoring.pfx] flat = true`) plays on an empty world (`Scene::empty()`, no tunables or rigs read), with no 3D pass and nothing staged, whatever scene the editor shows, as its window would.
- **The game ends play.** When the game fails (`world.fail`, or an `Err` from a `try_` hook, `warm` or `paint`), the error goes to the log as `play: the game failed: <error>; play stopped` (or `play: <error>; play stopped` for `warm` and `paint`) and play stops as F8 would: `Game::stop` runs, the edits are discarded and the viewport comes back. `world.quit(code)` stops play the same way with `play: the game quit with code N`. Nothing goes to stderr and the editor stays open.
- **Record** (F9, while playing or paused) saves the session's input recording, a `pfx_input::Recording` as JSON, to `tmp/pfx/recordings/<scene>-<UTC yyyymmdd-hhmmss>.json` under the project's root (a second save in the same second gets `-2`, and so on), and logs the path. Every play session records its input from its first tick (`Options::record`), and F9 copies what it holds so far without stopping it. `pfx bench <scene> --seconds N --replay <that file>` feeds it back one step per tick (docs/frame-stats.md).
- **Pause** (F5 or F7) and **step** (F6, one tick) keep pgpu's play on.
- **Stop** (F8, or closing the window, which drops the session) runs `pgpu play off`, stops the game (`Game::stop`) and drops play's driver and painter. The viewport was never touched during play, so its pixels, camera and selection come back exactly, and the reloads that arrived during play are taken then.
- **Sound** (`audio.rs`): the first play opens the default output device once (`pfx_sound::Output`) and keeps it for the session. Each play starts the session with `audio_rate` and `audio_channels` set to the device's (the sender does no resampling), opens one PCM stream (`Output::pcm_stream`, 100 ms of room) and pushes every frame's `take_audio()` into it.
  - **Pause** pushes nothing and is silent; **step** pushes that tick's audio; **resume** carries on; **stop** pushes what is left and finishes the stream, so the tail plays out without a click. Play again opens a fresh stream on the same device.
  - **Buffering:** the stream is kept about 100 ms ahead of the device. A stream is primed with 50 ms before it starts, so a steady game does not run dry between frames. When the stream is full the oldest waiting frames are dropped and counted, and the UI thread never blocks. An underrun (the stream ran dry while audio was due) is counted and the stream primes again.
  - **The log** shows `sound  …` beside the pgpu status line: `muted`, the underruns and the dropped frames of the current or last play, nothing when all is well.
  - **No device, no failure:** with no output device, or one that fails to open, play runs exactly as before, its audio drained to nothing, with one `sound: …; play runs silent` warning in the log for the whole session. The offscreen shot harness runs with sound off (`Setup::audio` false) and logs nothing, so its shots do not change.
  - **Mute** (M, or the `mute` button after stop in the toolbar) silences the device's master and lights the button. It starts off each session and is not saved. The button shows only while sound is available (a device that has not failed to open, and the harness with sound on), so the offscreen shots keep their toolbar.
  - The device and stream sit behind two small traits (`Sink`, `Stream`) so the tests use a stand-in; the real `Output` is never opened in tests.
  - **A game's own PCM** (`world.sounds.stream`, `docs/game.md`) plays the same way: its stream is a channel of the world's mixer, so its samples are in each frame's `take_audio()` beside the scene's sounds, under the settings' volumes.
- **Nothing is written during play.** The inspector's edits go to the running world as live edits (`PlaySession::edit`, [play.md](play.md), "Live edits during play"): an object's transform, visibility and material, every key of a material, a light's keys, the camera's pose, field of view and exposure, a body's mass, friction, restitution, damping, velocity and size (in Rapier, without a restart), a sound's volume, pan and loop, and the project's tunables. Each reaches the game on the next tick, and the view redraws while paused. A key play cannot take is refused with a note in the log (`play mode: light warm range: needs a number of 0 or more`). Files changed on disk during play queue until stop.
- **Pending until Keep.** Every live edit is held as the session's pending patch. Its rows show the played value and are marked: the label in the hot accent with a `•`. The toolbar shows `F10 keep N` while any are held.
  - **Stop** (F8) discards the patch, as before: the files stay as they were and the rows show them again.
  - **Keep** (F10, or the button) stops and writes the whole patch as one `ScenePatch` on the shared `History`: the scene keys through `SceneEdit::dry_run` (to their defining files, the author's text kept), and the tunables as their new `default`s in the file that holds them. One Ctrl+Z takes it all back. The log says `keep N play edits`.
- `PFX_EDIT_PGPU=off` skips pgpu's play mode, its status line and the wait for another game; `bin/rig` sets it, so a captured editor never holds anyone's jobs. Without a `pgpu` on `PATH` they are skipped the same way.

## Tests

- **Play as the runtime does** (`crates/editor/tests/play.rs`, GPU, ignored; shots in `tmp/editor-shots/play-*.png`, not compared with stored ones): a flat project with a game of only `ui`, `warm` and `paint` plays under F5 in the offscreen harness, with its painted square, its UI card, its label's ink, its clear colour and the 16:9 frame's bars in the play view, laid out at 1080 units, warmed once before its first UI; F8 then F5 builds a second game from the factory (one per play) and stops the first; a flat project with no game opens on its empty world and F5 plays it; `world.fail` at tick 5, and a `paint` error, stop play with the error in the log and `Game::stop` run once. On the CPU (`src/tests.rs`, `src/audio.rs`): a flat project with no scene opens on the empty world, a project neither flat nor with a scene is still refused unless its game's config is flat, and a game's PCM stream reaches the sound device through the play's `Driver`, the same samples as a run of its own.
- **Keycap labels** (`crates/editor/tests/play.rs`, GPU, ignored; shots in `tmp/editor-shots/play-keycap-*.png`, not compared): a prompt for Q in a game's UI in the play view, with no layout (Q), with a configured AZERTY stand-in (A), and with the key press and its text injected into the editor's own egui input (A); the learned shot is pixel-identical to the configured one. On the CPU (`src/input.rs`): events teach labels by physical position and refuse chords, repeats, unmapped keys and shifted punctuation, and an AZERTY stand-in teaches the play `Driver` that Q reads A, and a configured source labels each play's driver.
- **Unit tests** (`crates/editor/src/**`, in the fast gate; `src/tests/live.rs` for play's live edits: the outline's sounds and tunable groups and the body, sound, tunable and exposure rows; every kind of play edit reaching the world with its row marked and nothing written; Keep as one undo step that one undo takes back to the byte; Stop discarding; F10 and the keep button; a stopped tunable row and a mass row): the outliner tree of a scene, inspector rows and their `SceneEdit` paths, row changes (unchanged, clamped, unset), edits written to the defining file with its text kept and undone to the byte, the dry-run save (a dry run writes nothing; landing it writes each touched file once; a number drag writes once a step and is one undo step; the viewport reloads its own write once and the editor never takes it as an outside change), one undo stack across the inspector, the gizmo and the TOML panel, the TOML panel's pfx-scene problems at line and column with nothing written, outside changes (the files reload and the steps they break are dropped), world edits during play, the log, the pgpu status line, `PgpuPlay`'s commands against a stand-in `pgpu` (on at play, off at stop or drop, once; a missing one reported) and finding `pgpu` on `PATH`, play's sound (the frames pushed match the session's `take_audio` in order across play, pause, step, resume and stop; no device logs once and plays silent; the drop and underrun counts; mute; a fresh stream on each play), play's input from egui, pfx's theme (its roles, fonts and caps), the named points and the PNG compare.
- **Whole-window flows on the CPU** (unit tests too): the editor crates' typed script drives the real egui UI headless through `pfx_editor_shell::drive` on one context, with named points from `Layout::find` (`@object:crate`, `@material:clay`, `@row:roughness`, `@button:move`, `@viewport:0.1,0.9`, `@source:0.5,0.2`). They open the fixture scene, select from the outliner, type a material value and undo it to the byte, pick in the viewport by the CPU ray, drag the editor crates' move gizmo in one undo step, type into the TOML panel, and use the keys.
- **Offscreen shots** (`crates/editor/tests/shots.rs`, `#[ignore]`, under pgpu; one of them plays, holds D and presses F9, and checks the saved file reads back as a recording and replays to the same tick count): the same scripts through the editor crates' `pfx_editor_shell::shot`, the whole editor with the editor crates' viewport and pfx-live, painted offscreen to PNG on the harness's own `Gpu`. They open (the same script gives the same bytes), pick the crate, show the gizmo on the selection and mid-drag (the preview moves, no file changes until release, then one undo step), edit a material and undo, edit a file in the TOML panel (and a refused edit with its problem listed, `source-problem`, where the panel names the file relative to its folder, so no absolute path is in the shot), play, pause, step and stop (stop restores the viewport's pixels and no file changes), a material typed while paused (the game's pixels change, no file changes, the row is marked; F10 writes it in one undo step; written to `tmp/editor-shots/live-edit.png` and `live-kept.png`, not compared with a stored shot), and trace. Each shot is compared with its stored `crates/editor/tests/shots/<name>.png` (mean ≤ 1.5, worst ≤ 96 per channel) and is always written to `tmp/editor-shots/`. A missing stored shot fails. After a change that moves the pixels on purpose, `PFX_EDIT_BLESS=1` writes the stored shots instead of comparing. Look at every one, and update their lines in `tests/agnostic/fixtures.txt`.
- **Level building** (`src/tests/level.rs`, unit tests on the neutral level project `crates/load/tests/level`: a floor, a ledge, two prefabs and a `[[tiles]]` strip): the outliner's `tiles` section without the cells; a prefab dragged from `content` onto the floor through the typed script, one undo step undone to the byte; grid snapping (N, `[` and `]`, the gizmo's step) and the P key; surface snapping onto the ledge's top and front with the normal; vertex snapping; a Shift-drag line painted in one undo step, erase and pick; a rectangle with a prefab that joins the palette and a paint that grows the layer down and left; Shift-click picking, align, distribute and duplicate (and Shift+D), one undo step each, and tile cells refused.
- **Level building on the GPU** (`crates/editor/tests/level.rs`, `#[ignore]`, under pgpu): the same flow through `pfx_editor_shell::shot` on the level project (drag, grid, surface, a tile line; each one undo step and visible in the viewport's pixels; written to `tmp/editor-shots/level-*.png`, not compared), and a 100 × 100 layer that loads and draws within its budget (below 2 s to open, a GPU frame below 8 ms, and the cells adding below 16.7 ms to an offscreen pass against the same level without them), printing what it measured.
- **The transform override** (`crates/live/tests/override_transform.rs`, under pgpu): an override moving the crate draws the same pixels as a restaged copy of the moved scene, and an override without a transform moves nothing and changes no pixel.

```sh
cargo test -q -p pfx-editor --test shots --test play --no-run
pgpu run --class clip --as pfx -- cargo test -q -p pfx-editor --test shots --test play -- --ignored --test-threads=1
```

The script language is the editor crates' (`pfx_editor_shell::Script::parse`), one step per line (`#` starts a comment):

| Step | Passes |
|---|---|
| `frame` | one pass with what is waiting |
| `move x y` | waits for the next pass |
| `click x y` | moves and presses in one pass; the release waits for the next |
| `press x y [ctrl] [shift] [alt]`, `release x y` | one pass each |
| `drag x y x y [middle] [ctrl] [shift] [alt]` | press, four moves, release |
| `key <name>` | press and release wait for the next pass (egui key names: `Z`, `F5`, `Enter`, `ArrowLeft`) |
| `key ctrl+<name>` (and `shift+`, `alt+`) | its own pass, with the modifiers held |
| `text <text>` | the rest of the line, typed; waits for the next pass |
| `scroll x y <lines>` | one pass, positive lines come nearer |

Points are `x y` in points. pfx adds named points: `script::place` replaces each `@name` with its centre from the editor's `Layout`, and `script::resolve` lays out a fresh editor on the CPU with the lines before each named point, so a shot names what it clicks. Keep named points before any step that writes a file, since `resolve` replays those lines. The harness starts with one pass and ends with two more; passes are 1/60 s apart on a fixed clock, so a script plays the same every run. Keys that wait go together into the next pass, so put a `frame` between two keys whose second depends on the first (F7 then F6), and after a click that opens a field before typing into it.

## bin/rig

`bin/rig` runs `pfx edit` on the rig contract: a headless cage, a private `/tmp` under `tmp/rig-<name>/`, and muted. It opens a copy of the scene's folder in the rig, so the tracked files never change.

```sh
build-step pfx-build tmp/build.log -- 'cargo build --release --bin pfx'
bin/rig up neutral edit          # the engine's neutral fixture scene; or a path to a scene.toml
bin/rig keys edit -k j -k j      # select the second outliner item
bin/rig shot edit selected       # tmp/rig-edit/tmp/selected.png
bin/rig down edit
```

`keys` takes `wtype` arguments. A shot taken right after `keys` can race the frames those keys cause; take it again if it looks a step behind. `pointer` needs `RIG_COMPOSITOR=labwc` (cage has no virtual pointer). `ls` lists the running rigs.
