# Editing a recipe in the window

A recipe opens in the window as a `.pitofx` document (`docs/editor.md`, "`.pitofx` is the document"): an old `.toml` is imported once into a `.pitofx` beside it, and the window edits the document's recipe on an unpacked copy, which Ctrl+S writes back into the `.pitofx`. What follows says how each edit reaches that recipe; the "file" below is the recipe inside the document. (The window's tests, and the offscreen tools, still work on a `.toml` in place, which is why the rules speak of a file.)

Every edit you make in `pfx gui` goes straight back into the recipe file. The window never holds a dirty copy, and there is nothing to save. The author's formatting and comments stay as they were, line for line, apart from the value that changed.

## Gestures

- **A gesture is one finished edit:** a rename, a field typed and committed with ⏎ or by clicking away, one drag on a number field (written on release, never per frame), one gizmo move, a camera set from the view, a text span changed.
- **A gesture is a list of patches.** A patch is a key path and a value. Panels never write files: they push gestures with `shell.edits().push(label, patches)`, and the shell drains that queue at the start and the end of every frame.
- **Paths name things.** An object, camera, light or text is addressed by its `name`, a mover by its `group`, so a patch still lands on the right entry after someone reorders the file. Positions (`Key::Index`) are used only to insert or remove.

## When the file is written

For each gesture, the window:

1. **Re-reads the file.** If it differs from what the window last read or wrote, the file changed outside the window; it is reloaded first (see below), and the gesture applies on top of it.
2. **Refuses params-driven values.** A value written as `"${params.…}"` is never patched, so a params file stays the single truth.
3. **Applies every patch** to the recipe's `toml_edit` document, then reads the result as a recipe once.
4. **Validates.** A gesture that adds a problem `pfx check` would report (a parent that doesn't exist, a name used twice) is refused.
5. **Saves atomically:** it writes `.<name>.pitofx-tmp` beside the recipe and renames it over the recipe.

If any step refuses, the whole gesture is rolled back, the file is untouched, and D1 says `refused` and why, until the next edit that lands. A gesture that lands puts its label and first key path in D1's `patch`, as `location · object[cup].at[0]`, with `+N` when it wrote more than one patch.

## Undo and redo

- **Keys:** Ctrl+Z undoes; Ctrl+Shift+Z or Ctrl+Y redoes. They work from any panel but not while a text field has the keyboard, where they edit the text. The hint bar lists them, and clicking its `ctrl z` or `ctrl ⇧ z` does the same; each is dimmed while its side of the stack is empty.
- **Each step is a gesture's inverse patches,** applied in reverse order. An edit followed by its undo leaves the file byte for byte as it was, comments and spacing included.
- **Undo and redo write the file** like any edit, through the same checks. A new gesture clears the redo side.
- **The stack lives as long as that recipe is open,** up to 256 steps. Opening another recipe starts a new stack. It is tool state: it never enters the recipe and is never saved.
- **The first save writes the objects' ids.** When the recipe writes a scene (`[output] scene`), a gesture's save also writes `id = "<the id the format 1 writer derives for the object>"` into every `[[object]]` that has none, into the file that holds the object (the recipe or an included rig), in the same step as the edit: one undo takes the ids away with it and redo puts them back, and the file keeps its formatting. An object that already has an `id` is never touched, a copy placed by `[[use]]` gets none, and a recipe without `[output] scene` gets none. An id is derived from the object's name before the gesture, so a rename that is the first save keeps it. A generator's object gets its id when the window can build it, with the generators' registry (`Editor::with_registry`); until then it has none.

## Who wins

- **The window's own saves** are known by the content hash of the text it wrote. When the file watcher sees one, the scene is rebuilt from it, but it is not treated as a change from outside. An older own save that arrives after a newer one is ignored.
- **A change on disk the window didn't make wins.** It is re-read before the next gesture, or as soon as the watcher reports it. The window shows the new file, and D1 says `reloaded` and how many undo steps were dropped.
- **Undo steps survive outside changes.** Patches address by name, so a step whose objects still exist stays valid. A step whose path is gone (the object was renamed or deleted outside) is dropped, and only that step: the stack is checked from the top down, step by step, on a scratch copy of the new file.
- **A file that doesn't read** (another editor halfway through a change) blocks every write, with the reason in D1, until it reads again. Nothing the window holds ever overwrites it.

So an agent or another editor writing the file and a person in the window never clobber each other: the last write on disk is always what both see.

## Tracing curves

The viewport has four orthographic views: `7` top, `1` front, `3` side and `9` rear (the number row or the numpad), framing the selection, or the whole scene with nothing selected. `5` goes back to perspective about the same point, and `0` back to the chosen camera; an orbit drag leaves the standard view. In a view that a `[[reference]]` names, its image is drawn behind the scene at its scale, offset and opacity: shaded, it shows only where no surface covers it; in wireframe (`z`) it lies over the whole dark ground, under the edges, which is the view to trace in. `i` hides and shows every reference.

With a generator object selected, the view shows its curve params as point handles joined by a line, in the info colour: every param of the object whose value is an array of at least two `[a, b]` points, anywhere in its params (`stations.0.curve`, `silhouettes.side`, `regions.1.curve`). A curve shows in the view it belongs to: the view a sibling `view = "..."` key names (as a region or a groove has), else the view its own key names (`silhouettes.top`), else the plane across the object's `axis` param when it has one (`x` or `[1, 0, 0]` is the side view, `y` the top view, `z` the front and rear views, as a `pfx/loft` station's cross-section across its axis), else the front and rear views, in `[x, y]`. A sibling `at` places the curve along the view's depth axis (a station's position along the axis). Points are in the object's own frame: the object's location, rotation and scale place them.

- **Drag** a handle to move the point within the view's plane, rounded to the millimetre; the drag is one undo step and Esc puts the point back.
- **Ctrl+click** on the line adds a point there, between the two it lies between.
- **Click** a handle to select it, and **Delete** (or Backspace) removes it; a curve keeps at least 2 points.

Each is one gesture, written through the same patches as `pfx curve`, keeping the file's comments; the object rebuilds as for any edit. Curve handles come before route handles and the gizmo under the pointer.

## Includes and uses

A recipe that includes rig files or places copies of part files (`docs/recipe-format.md`, "Includes and uses") opens and edits as one recipe.

- **The window watches every file the recipe read,** the recipe first, then each included rig and used part. An outside edit to any of them reloads the window as an edit to the recipe does, and a file that does not read keeps the last good view with the reason in D1.
- **Gizmo locks hold for included objects:** an `at_px` or `${params.…}` lock on an object an include brings in locks its gizmo as it does in the recipe.
- **A copy reads as read-only in B1** before any edit is refused: its row is dimmed and carries a small `use` badge, and hovering it says which use places it and from which file (`a copy, read only: use[left_] places it from parts/knob.toml`).

## C2, the properties panel

For the selected object, every field is a gesture:

- **name:** ⏎ or clicking away commits, Esc cancels. A rename refuses an empty name or a name another object has. It updates every reference in the same gesture, so one undo puts them all back: `parent`, a pocket's `into`, a text's `on`, and a mover's `parts`.
- **location, rotation, scale:** a drag commits on release; a typed value on ⏎. The patch goes to the key the recipe uses, one element at a time, so the other two keep their text:
  - `at` in metres, or `at_px` in plate pixels (x and y in pixels, z in metres) when the object uses `at_px`, or uses neither under a `[plate]`;
  - `rotate` in degrees;
  - `scale`; a uniform `scale = 2.0` becomes a list when one axis changes.
  - Values are written rounded: metres and scale to 4 decimals, pixels and degrees to 2.
- **material:** pick one of the recipe's material names from the list, or type a new name and press ⏎. An object with several slots (`materials = [...]`) shows them read-only.
- **smoothing:** `flat` removes `smooth_angle` and `smooth`; `smooth` writes `smooth = true`; `by angle` writes `smooth_angle`, with the angle beside it.

A value written as `"${params.…}"` shows in C2 as its params path, read-only.

A text object has neither a material nor a smoothing, so C2 shows its name, its placement and its spans, and leaves the material and smoothing rows out.

## The hour in B3

The slider in B3 is a view control: dragging it, or ←→ with B3 focused, moves the sun and the sky live and never writes the recipe. It is disabled, with the reason on hover, in a scene file (`a scene file's sun is not moved by the hour; edit it in its file`) and with no recipe open (`open a recipe to move its sun by the hour`), where ←→ do nothing too. The button beside it, **Use hour**, writes the shown hour to `[sun] hour` as one gesture, labelled `hour`, so D1 shows `hour · sun.hour` and one undo puts the recipe back.

- **The value** is the slider's hour rounded to 2 decimals, so `hour = 6.5` and `hour = 14.26`. The recipe's own line and comment stay as they were.
- **It is off** while the shown hour equals the recipe's (to 2 decimals), and hovering it says `the recipe has this hour already`.
- **It says why** when there is nothing to write: `no [sun]` for a recipe without a `[sun]`, `authored sun` for a `model = "authored"` sun, which has no hour of its own, and `scene file` for a scene. Hovering gives the sentence. An authored sun asked anyway is refused in D1 the same way.
- **After the edit** the slider and the recipe agree and the stage shows the recipe's hour. Undo and redo do the same: the slider follows the recipe whenever the recipe's hour changes, including when the file changes outside the window.
- **A params-driven hour** (`hour = "${params.…}"`) is refused with the params message and the slider keeps the hour it shows.

## Timeline

A recipe with a `[timeline]` (`docs/recipe-format.md`) shows a transport bar along the bottom of C1, under the picture. Like the hour slider, it is a view control. The current time is window state: nothing is written to the recipe and there is nothing to save.

- **The bar, left to right:** play or pause (`space`), one frame back (`,`), one frame forward (`.`), the previous and next marker (`[` and `]`, shown when the timeline has markers), **loop**, **Chase cam** (`f`), the scrubber, and the readout: the time in seconds, the frame, the range `0–<frames>` and the timeline's fps. A lit button is on, and each button carries its key.
- **The scrubber:** a click or a drag on it sets the time and pauses playback. The scene is posed at that time as you drag: every animated node's instance is moved to its track at that time, interpolated between the two frames around it as glTF LINEAR does (translation and scale linear, rotation slerped). No mesh is rebuilt and nothing is uploaded again apart from the moved instances.
- **Markers:** each `[[marker]]` (`docs/recipe-format.md`, "Markers") is a tick on the scrubber with its name above it. `[` jumps to the marker before the current time, `]` to the one after, and pauses. An animated scene file opened in the window reads its markers from the GLB's `timeline` animation.
- **Playback** runs at the timeline's fps from the window's own frame clock. It shows whole frames and never runs ahead of real time, so a slow frame skips frames rather than slowing down. It stops on the last frame, or wraps to frame 0 with **loop** on. Space on the last frame starts again from 0. The steps `,` and `.` land on whole frames; from between two frames, `,` goes to the frame before.
- **Chase cam:** with it on, the viewport looks through the timeline's camera node (`camera.<name>`) at the current time, with its lens's vertical field of view, framed at the timeline's `size` like a recipe camera's render frame. C1's header says `chase cam`. An orbit drag or the wheel leaves it for the free orbit, starting from where the chase camera was. With no camera track it shows the orbit as before.
- **Curves:** when a recipe camera has a `[camera.rig]`, the bar shows a **curves** button (`k`) that opens a strip above the bar with the rig's keyed tracks (`[camera.rig.tracks]`: distance, lateral, height, bank, lag, shake, look_ahead), one row each with its own value range, on the scrubber's time axis, with the current time as a line and each row's value at that time. Keys are points. Drag one to move it in time and value, snapped to whole frames and kept between its neighbours (hold Shift to keep its time). Ctrl+click a row to add a key there at the curve's value, and click a key then press Delete to remove it (a track keeps at least 2 keys). The header's `+ <track>` adds a track that isn't keyed, with two keys at the rig's static value at 0 and at the end, and its camera buttons pick the rig when there are several. Each gesture is one undo step written to `camera.<name>.rig.tracks.<track>` with the file's comments kept, and Esc cancels a drag. The build runs again as for any edit, and the time is kept.
- **Edits keep the time.** A rebuild after an edit (a route drag, a field in C2, an outside save) keeps the time, the play state and the chase toggle, and poses the rebuilt scene at that time. A timeline made shorter clamps the time to its new end. Opening another recipe starts at 0, paused.
- **Tapes and scripts** drive it like any control: `click` and `drag` on the scrubber and the buttons, `key Space`, `key Comma`, `key Period`, `key f`, `key OpenBracket`, `key CloseBracket`, and `seek <marker|seconds>`. In a tape the clock is the tape's own fixed step, so the same tape poses the same frames byte for byte.

## Opening a recipe

Ctrl+O, or Open… in the File menu, opens the picker over the viewport. Esc closes it, and so does a click on the viewport beside it. In a window with no recipe (the picker's own folder, `pfx gui <folder>`), Esc clears the field while the picker stays. The hint bar lists `ctrl o`, and clicking it opens the picker; the picker's own `enter` and `esc` caps do what those keys do.

The picker lists the recipes in the folders it knows, sorted by name: `samples/` beside the repo the binary was built from, when that folder is there, or, for an installed `pfx`, a copy in `${XDG_DATA_HOME:-~/.local/share}/pfx/samples`, which the install step writes without overwriting (the window's own `workshop.toml` and `gallery.toml`, `samples/README.md`), and the folder of the recipe last opened. An app's scenes are opened from its own repo, with its own build binary, so their generators are registered. The last ten recipes opened are listed first, newest at the top. They are kept in `$XDG_STATE_HOME/pfx/recent`, or `$HOME/.local/state/pfx/recent` when that variable is unset or empty, one path a line. On Windows both folders are `%LOCALAPPDATA%\pfx` and on macOS `~/Library/Application Support/pfx`, so the recent list is `recent` and the installed samples `samples` in it. Opening a recipe again moves it to the top. The file is written atomically, as `.<name>.pitofx-tmp` beside it and then a rename, the same way a recipe is saved.

The path field completes files and folders as it is typed. Enter opens the typed path when it is a recipe file, or the selected row when the field is empty. A typed folder, or Enter on a folder row, lists that folder. The arrow keys move the selection. Clicking a row opens it.

`pfx gui` with no path opens the start scene, and the picker stays closed: see "The start scene" below. `pfx gui <recipe>` opens that recipe directly. `pfx gui <folder>` opens the picker on that folder. A path that is not there is still treated as a recipe.

Opening a recipe rebuilds the stage, starts undo empty, and watches that file. The previous recipe's undo stack is dropped, and it is not applied.

A script drives it like any other key: `key ctrl+o`, then `text` for a path, then `key Enter`.

## The start scene

`pfx gui` with no path opens a blank scene the way Blender's or Unity's default scene opens: pfx's own mark stands upright at the world origin with the pfx wordmark beside it and the version under the wordmark's end, on a floor in the theme's black, seen in perspective from a three-quarter view looking slightly down. The mark is selected, so its move gizmo sits in the middle of the view, ready to drag. It is live like any recipe, and the hint bar's `ctrl o` opens the picker.

- **The mark is two objects under one parent,** as Blender's default cube is one: the hidden object `pfx` holds `mark` and `wordmark`, each a `mesh` of one GLB, and the `version` text. They are the meshes `pfx new` puts in a game's first scene (from `assets/marks/pfx.svg` and `assets/marks/wordmark.svg`), held once in `pfx_project::marks` for both. Each keeps its own materials (the mark's tile, dome, lines, lime, violet and white; the wordmark's light grey), defined in `crates/workbench/start/start.toml`. The two front faces lie on one plane, `pfx`'s origin is the centre of the mark's bounds, and dragging `pfx` moves the whole mark.
- **The version stands under the wordmark's end,** its right edge at the wordmark's, on the wordmark's front plane, in the embedded mono and the theme's `TICK` grey, unlit. The shipped `start.toml` says `vX.Y.Z`; the window writes the running version (`CARGO_PKG_VERSION`) into each copy, so the start scene always shows the binary that drew it, as the status bar's `pfx vX.Y.Z` does. The floor takes the theme's `BG` the same way.
- **The floor grid:** the viewport draws a faint grid on y = 0, with the x axis along it in red and the z axis in blue, in every live recipe view. Its lines are a power of ten apart, picked from the eye's distance, and they fade with their distance from the point the eye looks at. It is drawn over the picture, under the gizmo, from the theme, and only in the window: no still, render or clip carries it. The overlay has no depth buffer, so the depth it uses is the built parts' world boxes: a line is cut where a ray from the eye meets a visible part's box before the floor, and the boxes of what a gizmo drag moves follow the drag.
- **The scene is the binary's own.** `start.toml` and the two GLBs are compiled in, so any install shows it, with no data folder.
- **The window edits a copy.** Each time it starts with no path it writes `start.toml` into `$XDG_STATE_HOME/pfx/start/` (`~/.local/state/pfx/start/` when that variable is unset or empty; `%LOCALAPPDATA%\pfx\start\` on Windows, `~/Library/Application Support/pfx/start/` on macOS), and the two GLBs into `start/` beside it, overwriting what the last session left, and opens it. Moving or deleting the mark edits the copy, the shipped files never change, and the next start is the start scene again. Save asks for a `.pitofx` to keep it in, as Save As does.
- **A shot:** `pfx gui --shot <png> --start` draws it offscreen, with the grid and the selected mark's gizmo, writing the copy into a `start/` folder beside the PNG.

## Opening a scene file

A scene (a `.pitofx` holding a scene, or an old `*.scene.toml`, imported once into a `.pitofx` beside it) is the engine's runtime scene (`docs/scenes.md`). `pfx gui room.pitofx`, `pfx gui --shot <png> room.scene.toml` and the picker open one; the picker lists scene files beside the recipes, marked `scene` in front of their folder. A scene is edited through pfx-editor's data layer (`SceneEdit`, the inspect patch, one undo history), shown in the window's own panels: `docs/editor.md`, "Scenes in the window", says how.

## Files

The top bar's File menu, beside the A header, opens, saves, imports and exports. Every dialog is the window's own picker, never the system's, so it works the same offscreen and in a tape.

| Item | Keys | Does |
| --- | --- | --- |
| Open… | Ctrl+O | opens a document (`.pitofx`, a recipe or a scene), or a `.glb`, which opens as an imported document: a new recipe holding one `kind = "mesh"` object at the origin with that file; an old `.toml` recipe or `.scene.toml` is imported once into a `.pitofx` beside it, which then opens |
| Save | Ctrl+S | writes the open `.pitofx` in place (a scene `.pitofx` is already saved, since it takes every edit as it is made); the start screen and an imported `.glb` ask for a name as Save As does |
| Save As .pitofx… | Ctrl+Shift+S | packs the open recipe or scene, with the files it reads, into a `.pitofx`, and goes on editing that document |
| Import glTF… | | adds a `.glb` into the open scene as a new `kind = "mesh"` object at the origin, named from the file (`car.glb` is `car`, then `car_2`), as one edit Ctrl+Z puts back; into a `.pitofx` the file is copied into the document, into a `.toml` its path is written relative to the recipe |
| Export glTF… | | writes the scene as a standalone `.glb`, or only the selected object when "the selected object only" is ticked |

- **The save dialog** is the picker with a folder field over a name field. It opens on the document's folder with its name filled in (`chase.pitofx`, `chase.glb`), lists the folder's subfolders and the files of the type it writes, and a click on a file takes its name, so Enter overwrites it. A name without the extension gets it.
- **A `.pitofx` is edited on an unpacked copy.** Opening one unpacks its recipe and files into `$XDG_STATE_HOME/pfx/documents/<name>-<hash>/` (`~/.local/state/pfx/documents/` when the variable is unset), and every edit (a gizmo, a route, a field, a set) patches that copy. The `.pitofx` itself changes only on Save, through the container's atomic writer. While edits are unsaved the top bar shows ● after the name, and the window's title carries it: `pfx · chase.pitofx ●`.
- **Drop a file on the window** to open it, or hold Shift while dropping a `.glb` to import it into the open scene.
- **A `.pitofx` pfx edited** (its scene no longer the one its recipe built) opens with a note in the status bar: `pfx scene fold <file>` lists those edits and folds them into the recipe (`--apply`) or keeps them as overrides (`--keep`); a save from the window rebuilds the scene from the recipe without them.
- **The recent list** keeps `.pitofx` documents beside recipes and scenes, marked `pfx`, with the document's thumbnail when it has one.
- **D1 says what happened:** `file saved …`, `file exported …`, or the error.
- `pfx gui chase.pitofx` and `pfx gui car.glb` open a document directly, and `--shot` and `--clip` take them too.

In a tape, each item is one step, taking one frame, played as the menu would: `open <path>`, `save`, `save-as <path>`, `import <path>`, `export <path> [--selected]`, and `drop <path> [shift]`, which drops the file on the window the way a file manager does. A path is the rest of the line, relative to where the clip plays (its `.work/` copy).

## Scripts for `--shot`


`pfx gui --shot <png> --input <script>` plays a script of one step a line, then draws. The script and the offscreen harness that plays it are `pfx-editor-shell`, the same for every editor built on it. A line starting with `#` is a comment. Coordinates are points from the window's top left.

| Step | Does |
| --- | --- |
| `key <name>` | presses and releases one key by egui's name (`b`, `Escape`, `Enter`, `ArrowDown`). `ctrl+`, `shift+` and `alt+` in front hold a modifier: `key ctrl+o`, `key ctrl+z`, `key ctrl+shift+z`, `key ctrl+y` |
| `text <words>` | types the rest of the line |
| `move x y` | moves the pointer; it takes effect on the next pass |
| `click x y` | presses on one pass and releases on the next |
| `press x y [ctrl] [shift] [alt]` | moves to the point and presses the primary button, which stays down, with those modifiers held, until a `release` |
| `release x y` | moves to the point and lets go, then drops the modifiers |
| `drag x y x y [middle] [ctrl] [shift] [alt]` | press, four steps of motion and release, in one line |
| `scroll x y lines` | the wheel, positive to come nearer |
| `frame` | draws one pass |

`press` plus `move` and `frame` pairs plus `release` is a drag a script can stop inside: a shot of a gizmo mid-drag ends before the `release`, and Esc mid-drag is `key Escape` between two moves. A `key` that carries a modifier draws a pass of its own with the chord held; a plain `key` waits for the next pass like `text` does.

## Clips of the window

`pfx gui [<recipe>|<folder>] --clip <dir> --tape <file>` films the window offscreen while it plays a tape, frame by frame, into an H.264 master, a poster and a manifest. `--size WxH` sets the window (1586x992 by default), `--fps` the rate (30, 60 or 120; 30 by default), `--poster` the poster's frame (`first`, `last`, or the frame at a time in seconds; `first` by default), `--start` plays on the start scene, and `--bare` draws the viewport alone over the whole frame, with no panel, header, hint bar or timeline bar (the pointer still draws, and the tape's points are the viewport's). It writes the frames as PNGs into `<dir>/<stem>.partial/`, encodes them with pito-codecs' `pcod frames` on the CPU and removes them; it needs pcod on the PATH and an even `--size`. With pgpu it queues itself as a clip job and turns between frames as the engine's pacer does.

- **A tape** is the script's steps above, with time added. One frame is one pass of the window at the clip's rate, and `wait <seconds>` holds still. The pointer glides to each point a step names (0.18 to 0.6 s), a `drag` moves over 0.3 to 1.2 s, `text` types 15 characters a second, and a `key` or a `frame` takes one frame; a click presses on one frame and lets go on the next. The clip draws the pointer, with a lime ring while a button is held. Frame 0 is the window before the first step.
- **It is deterministic.** Between frames the clip waits, without recording, for everything the window started to finish: a build, the rebuild after an edit, the stage, a pick, a render. Each frame shows the settled window, the viewport draws as a shot draws it, and the status bar shows no measured GPU time, so the same tape gives the same frames byte for byte, and pcod encodes the master from them.
- **It plays on a copy.** The folder, or the recipe's folder, is copied into `<dir>/<stem>.work/` (without `.git` and `target`, at most 256 MiB) and played there, so the tape's edits never reach the original and a second run starts from the same files. The picker lists only that folder, and no recent recipes. The copy is removed when the clip is made.
- **What it writes** in `<dir>`, each named `<tape>-<W>x<H>-<fps>fps`:
  - `.master.mp4`: H.264, CRF 14, 8-bit 4:2:0, BT.709, with a silent stereo AAC track, made by `pcod frames <frames> --fps <F> --codec h264 --route cpu --crf 14`;
  - `.poster.png`: the poster's frame as an 8-bit sRGB PNG, taken before encoding;
  - `.json`, the manifest: `tool` (`name`, `version`), `encoder` (`name` `pcod`, `version`), `kind` (`window-clip`), `target` and `tape` as given, `params` (`size`, `fps`, `poster`, `bare`), `frames` (`master`, the frame count; `seconds`; `poster`, the poster's frame), `outputs` (the master's and the poster's formats in words), `files[]` (`file`, `role` `master` or `poster`, `bytes`, `fps`, `size`, and the poster's `frame`) and `encodes[]` (`file`, `role`, `fps`, `size`, `encoder`, and `args`, pcod's options). Readers ignore keys they don't know.
- **The README's clips:** the first is `samples/tour.tape`, for the default size, on the picker of `samples/`: it opens the workshop, gives the vase brass, brings the sun down to 19:00 and keeps that hour, moves the lamp and renders the hero camera. `pfx gui samples --clip <dir> --tape samples/tour.tape --poster last` films it, and `pcod <dir>/tour-1586x992-30fps.master.mp4 docs/images/tour.gif --size 1280 --fps 15` makes the README's GIF. `samples/edit.tape` (on `samples/workshop.pitofx`) and `samples/render.tape` (on `samples/hero.pitofx`) make its other window clips the same way. A change to the window's layout moves the tapes' points with it.
