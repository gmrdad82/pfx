# Editor actions on the command line

<p align="center"><img src="images/cli-coverage.png" width="800" alt="A table, editor actions on the command line. In pfx gui on recipe documents: save, export glTF, import a glTF as an object, new and set cameras, location, rotation and scale, material and text spans, lights, sun and hour, route points and curve handles, and rendering a still each have the same command; cut, duplicate and delete, and camera rig tracks, have a command that does part of it; paste, rename with its references, and undo are not a command today. In a game.s editor on scenes: the TOML panel is pfx scene write, the whole file; the inspector, gizmo, level tools and undo are not a command today. 54 actions that write: 17 the same, 14 part of it, 23 not a command today."></p>

This page lists every action that changes a document in pfx's editor (`pfx gui`, and `<game> edit` for a game's scenes), with the command that does the same from a shell, so a script or an agent can make the edit without the window. Each command was checked against its `--help` at this revision. Where no command does the action, the row says so: not a command today.

Actions that only change the view (selection, orbiting, standard views, shading, x-ray, overlays, gizmo modes, snapping, the transport, the log and the frame stats) write nothing and are not listed.

The commands write into the `.pitofx` at once, in one atomic replace, and check the result before they write; the window edits a working copy and writes it on Save. `pfx <command> --help` prints each command's flags, and most take `--json` for one JSON object on stdout.

## Recipe documents (`pfx gui`)

| Action in the window | Command |
|---|---|
| Save | `pfx scene save <file.pitofx>` |
| Save as a `.pitofx` | `pfx scene migrate <file.toml>` for a plain TOML recipe; `pfx build <file.pitofx>` writes a document's build |
| Import a glTF into the recipe | `pfx add <file.pitofx> object <name> kind="mesh" path=<file.glb>` adds it as an object; `pfx import <file.glb> --as <kind/name>` writes it into the object library under the current folder |
| Export glTF, all or the selection | `pfx export <file.pitofx> --glb <out.glb> [--only <object>]` |
| Paste | not a command today |
| Cut | `pfx remove <file.pitofx> <table> <name>` removes one entry; the window also removes its children |
| Duplicate | `pfx duplicate <file.pitofx> <table> <name> <new-name> [key=value]...` copies one entry; the window also copies its children and their materials |
| Delete | `pfx remove <file.pitofx> <table> <name>`, refused while another entry refers to it; the window removes the children too |
| New camera from the view | `pfx add <file.pitofx> camera <name> position=[x,y,z] target=[x,y,z] fov_y_deg=<degrees>` |
| Set a camera from the view | `pfx set <file.pitofx> camera.<name>.position=[x,y,z] camera.<name>.target=[x,y,z]` |
| Delete a camera | `pfx remove <file.pitofx> camera <name>` |
| Location, rotation and scale fields, gizmo drags | `pfx set <file.pitofx> object.<name>.at=[x,y,z]`, `.rotate=`, `.scale=` |
| Rename an object | not a command today: `pfx set object.<name>.name=` renames the object alone and is refused while other entries name it, where the window rewrites those references too |
| An object's material | `pfx set <file.pitofx> object.<name>.material=<material>` |
| Smoothing | `pfx set <file.pitofx> object.<name>.smooth=true` or `false` and `object.<name>.smooth_angle=<degrees>`; taking an angle off again is not a command today |
| Text spans: text, size, colour | `pfx set <file.pitofx> object.<name>.spans[<i>].<key>=<value>` |
| Light, sun and sky dials | `pfx set <file.pitofx> light.<name>.<key>=…`, `sun.<key>=…`, `sky.<key>=…` |
| Use the hour | `pfx set <file.pitofx> sun.hour=<h>` |
| Route points: add, move, delete | `pfx route <file.pitofx> add --route <name> --point x,y,z`, and `set` or `remove` with `--at <i>` |
| Curve handles: add, move, delete | `pfx curve <file.pitofx> <object> <param> add --point x,y,z`, and `set` or `remove` with `--at <i>` |
| Camera rig tracks and their keys: add a track, add, move or delete a key | `pfx set <file.pitofx> camera.<name>.rig.tracks.<track>=[...]` writes a whole track; `pfx key` does not take rig tracks |
| Render a still | `pfx render --recipe <file.pitofx> --camera <name> --out <png> --size WxH`, the same command the window runs |
| Cancel a render | `pfx job stop <name>` stops a render started with `pfx job start` |
| Undo, redo, jump in the history | not a command today |
| Play, pause, step, stop | `pfx gui <file.pitofx> --play <module.wasm>` opens the window playing; `pfx bench <scene.pitofx> --seconds N` plays a scene headless with frame stats; `pfx shot` renders its frames |

The command line also does what the window has no action for: `pfx align` (align, distribute, ground and snap objects), `pfx add` and `pfx remove` for lights, `pfx key` and `pfx unkey` for an object's keyframes, `pfx bake`, `pfx film`, `pfx sound`, `pfx clip`, `pfx shoot`, `pfx library`, `pfx asset`, `pfx credits` and `pfx git setup`.

## Scenes (`<game> edit`)

A scene's inspector, gizmo and level tools edit the scene's TOML. On the command line, `pfx scene print <scene.pitofx>` shows it and `pfx scene write <scene.pitofx> < new.toml` replaces it whole, checked before it is written; no command edits one key of a scene.

| Action in the editor | Command |
|---|---|
| The TOML panel: typing, toggling a comment | `pfx scene write <scene.pitofx> [file] < new.toml`, the whole file |
| Inspector: an object's location, rotation, scale, material, shadow, two sides, hidden, clip and parent | not a command today |
| Inspector: physics body and character | not a command today |
| Inspector: the materials table | not a command today |
| Inspector: lights and emitters | not a command today |
| Inspector: sun, sky and haze | not a command today |
| Inspector: camera and the finish's exposure | not a command today |
| Inspector: a sound's volume, pan and loop | not a command today |
| Inspector: tunables | not a command today |
| Inspector: mesh file and node, node overrides | not a command today |
| Gizmo drag | not a command today |
| Place a prefab or a glTF | not a command today |
| Duplicate | not a command today |
| Delete | not a command today |
| Align to the first or last, distribute | not a command today; `pfx align` works on recipes |
| Paint or erase with the tile brush | not a command today |
| Record input (F9), keep play edits (F10) | not a command today |
| Undo, redo | not a command today |

## Count

Of 54 actions that write: 17 have a command that does the same, 14 have a command that does part of it, and 23 are not a command today, 20 of them in a scene (the rows above). A row that names several actions (route points, curve handles, rig tracks) counts each of them.
