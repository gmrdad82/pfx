# The scene format, version 1

This is the specification of format 1: one TOML format for 3D scenes, prefabs, material libraries and their projects, which an engine and an editor both read and write. It is pure data: no engine type, no GPU and no clock. `pfx-scene` parses it, checks it, writes it back keeping comments, spacing and order, and migrates older files. An engine loads a checked project into its own types; an editor edits it through the crate's writer; a build tool emits it.

In pfx these files reach a user inside a scene's `.pitofx`, compressed in one file ([Scenes in a .pitofx file](#scenes-in-a-pfx-file), [.pitofx files](pfx-file.md#the-user-file)): this page is the format of the TOML inside it, which `pfx scene print <scene.pitofx>` shows and pfx's editor edits in its TOML panel.

## Files

| file | holds |
|---|---|
| `project.toml` | the project root's marker: `format`, `[project]` (`name`; `scene`, the scene an editor opens first; and `ignore`, the folders not to look in), `[tunables.<name>]` (the game's tunables), `[layers]` (the collision layers) and `[authoring]`. It lists no files: editors find the scenes as the scene `.pitofx` files under the root (and any old `*.scene.toml`, imported once), outside the folders skipped (see `Project::open`). |
| `*.scene.toml` | a scene entry: its includes, material libraries, meshes, objects, lights, emitters, movers, content, text, sounds, camera rigs, tile layers, sun, sky, haze, camera, finish, trace, plates and physics. |
| `*.prefab.toml` | a reusable piece: the entry tables a scene has (meshes, objects, lights, emitters, movers, content, text, sounds, rigs) and its own `materials` and `include`, with no `[[tiles]]`, `fallback`, `[sun]`, `[sky]`, `[haze]`, `[camera]`, `[finish]`, `[trace]`, `[plates]` or `[physics]`. |
| `*.materials.toml` | a material library: `[materials.<name>]` tables. |
| `*.proxies.toml` | a plate's invisible stand-ins: `[[proxy]]` entries. |

The kind of a file comes from its name: `project.toml`, `*.scene.toml` (and a format 0 `scene.toml`), `*.prefab.toml`, `*.materials.toml` and `*.proxies.toml`. A format 0 material library may keep any name (`materials.toml`); it is read as a library because a scene's `materials` names it. Inside a `.pitofx` each TOML entry keeps such a name (`scenes/room.scene.toml`), and that is how its kind is told; a user opens and shares the `.pitofx`, never a loose entry.

Every file starts with `format = 1`. A file without `format` is format 0 and is read through the migration below.

Assets are referenced by path, never embedded: glTF 2.0 (`.glb`, or `.gltf` with its buffers beside it) for meshes and animation, PNG for images and sprite atlases, WAV or Ogg Vorbis for sound, TTF or OTF for fonts, Radiance `.hdr` for skies.

## Paths

- A relative path resolves from the folder of the file that names it first, then from the project root (`Project::locate` gives the path it resolves to). An empty path, an absolute path and a path with a backslash are refused (`bad-path`), and so is a path that leaves the root with `..` past it (`outside-root`). A `..` that stays inside the root is allowed.
- The project root is the nearest folder, from the opened file upward, that holds a `project.toml`. Without one, the root is the opened scene's own folder.
- Paths use `/` and keep their case. A path the checker finds only with another case is refused (`path-case`), so a project reads the same on Linux, Windows and macOS.
- A named file must exist (`missing-file`). The exceptions are `[plates] dir`, a folder a bake writes, and the paths of `[project] ignore`: they may not exist yet.

## Ids

- Every entry that something can refer to carries an `id`: each mesh, object, light, emitter, mover, content, text, sound, rig, tile layer, material, prefab placement and proxy.
- An id is 10 lowercase Crockford base32 characters (`0-9`, `a-z` without `i`, `l`, `o` and `u`), 50 bits, such as `id = "7k2m9q4xzr"`. It has no clock part. Any other text is refused (`bad-id`).
- A writer makes an id random (an editor, when a person adds an entry, passes its own random bits: `Id::from_bits` keeps the low 50), or derives it from a stable key it owns (a build, which must give the same bytes for the same input): `Id::derive` takes the first 50 bits of the SHA-256 of that key, such as the authoring file's project-relative path and the entry's name. The crate reads no clock and no system randomness. Either way the writer checks that the id is unique in the project.
- An id is written once, when the entry is made, and never changes: renaming, moving or re-parenting keeps it. A tool that derives ids keeps them across renames by recording the id in its own source.
- The checker refuses an id used twice anywhere in the project, naming both places (`duplicate-id`).
- A hand-written entry may leave `id` out. The checker warns (`missing-id`), and the first write of that file through `pfx-scene` (an editor's save, a fix) adds one, derived as the migration derives them (below). Nothing else writes ids.
- Names are labels. `name` on a list entry, and the key of a keyed table (`[mesh.<name>]`), are free text for people and may change. A list entry's `name` is required, non-empty, and has no space at either end; a material name also holds no control character (`bad-name`). Names stay unique within their kind across a scene and its includes (`duplicate-name`), so that a name can still stand for an id.

## References

A key that refers to another entry takes that entry's id or, while it is unique in its kind, its name. Editors write ids. A reference that matches neither is refused (`bad-reference`), and one that matches a name used twice is refused with both places (`ambiguous-reference`). The keys that refer:

| key | refers to |
|---|---|
| `fallback` | a material |
| `[mesh.*.nodes.*] material` | a material |
| `[[object]] mesh` | a mesh |
| `[[object]] parent` | an object |
| `[[object]] material`, the values of `materials` | materials |
| `[[object]] content` | a content |
| `[[object]] prefab` | a prefab file's path, or a placed prefab's id (the same prefab as that placement) |
| `[[mover]] objects` | objects |
| `[sound.*] object` | an object with a body |
| `[[rig]] target` | an object |
| `[[tiles]] palette` values | a prefab file's path, or a placed prefab's id (the same prefab as that placement) |
| `[[proxy]] object` | a static object |

A layer name (`[object.body] layer`, `[object.character] layer`, `[object.trigger] layer` and `mask`) is not an entry: it names a layer of `[layers]` in `project.toml`, by its name only, and one that names none is refused (`bad-reference`), as is any layer name in a project without `[layers]`.

A clip name and a tile are not entries either. A sprite's `clip` and an event's `clip` name a clip of its `[object.animation.clips]`, and a character of a tile layer's `rows` or a `tile` of its `cells` names a key of its `palette`; one that names none is refused (`bad-reference`). A glTF clip name is the loader's to check, against the object's glTF file.

A reference resolves among the entries of the scene and its includes. Materials resolve among every library the scene, its includes and its placed prefabs name. A proxy's `object` resolves among the objects of each scene whose `[plates] proxies` names its file, as that scene resolves (includes merged, prefabs placed): by id, a placed object's namespaced id included, or by its key in that scene (its name, or `"<placement name>/<inner name>"` for a placed object). A proxies file that no scene names resolves across the project instead, by id or by a name unique in the project.

## Prefabs

A prefab is a `*.prefab.toml` file, placed as an object:

```toml
[[object]]
id = "q8w2e6r4ty"
name = "left lamp"
prefab = "props/lamp.prefab.toml"
at = [-1.2, 0.0, 0.4]
rotate = [0.0, 30.0, 0.0]
parent = "desk"

[object.set]
"shade.material" = "brass"
"bulb.intensity" = 3.0
```

- A placement takes only `id`, `name`, `prefab`, `set`, `at`, `rotate`, `scale`, `parent`, `hidden`, `dynamic`, `tint`, `recolor` and `authoring`; any other key is refused (`placement-key`). An object without `prefab` needs `mesh`.
- A placement is a group with no mesh. Its transform is the parent of every root object in the prefab: the prefab's root objects take the placement as their parent. Its `hidden` applies to every object it places. Its `dynamic`, like any object's, reaches the objects under it that don't set their own.
- A placement carries no body, character, trigger or animation of its own. A body or a character in a prefab is placed with it and starts at its world transform, the placements' transforms times its own (`[object.body]` and `[object.character]`, under `[[object]]`).
- `[object.set]` overrides any key of any entry in the prefab: its keys are `"<entry>.<key path>"`, and its values replace the prefab's. `<entry>` is the inner entry's id, or its name when that name is unique in the prefab and holds no `.`; an inner entry of a nested placement is written `"<placement id>/<inner id>"`. The key path is the entry's keys joined by `.`, where a number picks an entry of a list, counting from 0 (`"lamp.layers.1.frequency"`), as the writer's key paths do. An override may set a key the entry takes but does not hold yet. An override that names no entry, names a key that entry doesn't take, or gives a value of the wrong type is refused (`bad-override`).
- Override values are read as the prefab's own: a reference in an override resolves inside the prefab. A relation an override breaks is refused at the override's key (`bad-override`), with the entry it changed in `related` where that helps: a reference that names no entry of the prefab or names two, a parent cycle, an object two movers move, a hit sound's object without a body, or a second looping sound.
- An override only replaces values. Adding entries to a placed prefab is what a nested placement or the prefab's own include is for; format 1 has no other way.
- A prefab is self-contained: its references resolve inside the prefab and its includes, and its materials among its own libraries.
- Inside a placement, inner ids are namespaced `"<placement id>/<inner id>"`, so the same prefab placed twice gives distinct, stable ids. A reference from outside reaches an inner entry by that namespaced id.
- A prefab may place other prefabs, nested at most 64 deep (`MAX_DEPTH`): a scene's placement is one deep, a placement inside it two. A placement past that depth is refused at its `prefab` key (`prefab-depth`), and so is a cycle, a prefab that places itself through any chain of placements (`prefab-cycle`). Both diagnostics name the chain of files in their message and list each placement of it in `related`.
- A build tool's own composition (a file placed with a prefix, `at`, `rotate`, `scale` and `parent`) maps onto a placement one to one; the prefix becomes the placement's `name`.

## Includes

`include = ["…", …]` merges other files in: an include's entries come before the including file's own, includes in order, depth first. Includes nest at most 64 deep (`MAX_DEPTH`), counting from the scene or prefab being read: an include past that depth is refused at its key (`include-depth`), and so is a cycle, a file that includes itself through any chain of includes (`include-cycle`). Both diagnostics name the chain of files in their message and list each include of it in `related`. A scene's `include` names `*.scene.toml` entries; a prefab's `include` names `*.prefab.toml` entries. At most one file of a scene and its includes holds `fallback`, `[sun]`, `[sky]`, `[haze]`, `[camera]`, `[finish]`, `[trace]`, `[plates]` and `[physics]`; a second is refused, naming the first (`held-twice`). A prefab is not an include: it is placed, transformed and namespaced.

## Unknown keys and `[authoring]`

- Every table refuses an unknown key, naming the file, line, column and the allowed keys (`unknown-key`).
- Two keys are the exception. A top-level `[authoring]` table in any file, and an `authoring` key (any TOML value) inside any entry, belong to the tool that authored the file. `pfx-scene` checks only that they are TOML, ignores them on load, and keeps them byte for byte on write.
- Only entries take `authoring` (a mesh, object, light, emitter, mover, content, text, sound, rig, tile layer, material, prefab placement or proxy). Node overrides, sky layers, room lights, finish passes, bodies, characters, triggers, animations, tile cells, tunables and layers take neither `id` nor `authoring`.
- Authoring tools keep their own sources (recipes, generators, variants, parameters) and emit this format; they may record their provenance in `[authoring]`, such as `recipe = "<path>"` and `variant = "<name>"`, so their files round-trip.

## Units

Colours are linear RGB under `color`. Angles are degrees. Lengths are scene units (metres by convention), speeds scene units per second, and times seconds, except where a key counts ticks of the play clock (`[physics] rate`) or a sprite's frames (`fps`, frames per second). Y is up, and a right-handed scene looks down −Z. A model is `T·R·S`, with `rotate` about X, then Y, then Z.

## Keys over time

`[[light]]`, `[camera]`, `[[particles]]`, `[finish]`, `[haze]`, `[sun]` and `[sky]` (and each `[[sky.layer]]`) take an optional inline table `keys` that animates some of their keys over the scene's time:

```toml
[[light]]
name = "lamp"
position = [0.0, 2.0, 0.0]
intensity = 2.0
keys = { intensity = [[0.0, 2.0], [1.5, 8.0]], color = [[0.0, [1.0, 0.2, 0.1]], [1.5, [1.0, 0.9, 0.8]]] }
```

- Each entry of `keys` is named after a key of its table, the *keyed key*, and holds `[seconds, value]` keys: seconds of scene time, at least 0 and strictly rising, at least one key. The value has the keyed key's shape and range: a number, `[r, g, b]` for a colour (three numbers; `"#rrggbb"` is not taken here) or two numbers for a pair such as `cone`.
- Between keys the value is linear in each component; before the first key it holds the first value, after the last the last. A keyed key overrides its static value.
- The table refuses a name it does not key, a value of the wrong shape, a value outside the keyed key's range, and times that do not rise, each at its line and column (`bad-value`). A keyed key keeps its static key's conditions: a point light refuses keyed `cone`, an orthographic camera keyed `fov`, `fstop` and `focus`, and keyed `focus` needs `fstop` (static or keyed).

| table | keys it keys |
|---|---|
| `[[light]]` | `intensity`, `color`, `range`, `cone` |
| `[camera]` | `fov`, `fstop`, `focus` |
| `[[particles]]` | `rate`, `speed`, `size`, `color`, `alpha` |
| `[finish]` | `exposure`, `lut_amount` |
| `[haze]` | `amount`, `color` |
| `[sun]` | `hour` |
| `[sky]` | `intensity`, `backdrop`, `horizon_color`, `horizon_width`, `horizon_amount` |
| `[[sky.layer]]` | `intensity` |

A particle system's keyed `alpha` is a number from 0 to 1 that scales its `alpha` curve over the scene's time (1 when unkeyed). `[camera] focus_keys` is another spelling of `keys.focus`, and `[[particles]] rate_keys` of `keys.rate`; a table writes one of the two. A mix sky keys `intensity` on its layers.

## Tables

Each table below lists every key it takes. *Type* uses TOML terms: a *number* is an integer or a float, and `[x, y, z]` is an array of three numbers. Where a table gives a range, a number outside it (or not finite) is refused (`bad-value`). *Default* is the value when the key is left out; *needed* means the key must be there. A key whose default is described ("the object's") is resolved by the loader.

The crate checks structure, values, references, ids, and that each asset path exists with its exact case. It does not read the assets' contents: glTF nodes and the node names in `node`, `nodes` and `materials`, and the decoding of images, sounds, fonts and HDR skies stay with the loader that reads them.

### `project.toml`

| key | type | default | notes |
|---|---|---|---|
| `format` | integer | 0 | `1`. |
| `[project]` | table | needed | |
| `[tunables.<name>]` | tables | none | the game's tunables. |
| `[layers]` | table | none | the collision layers. |
| `[authoring]` | table | none | the authoring tool's. |

`[project]`:

| key | type | default | notes |
|---|---|---|---|
| `name` | string | needed | the project's name. |
| `scene` | path | none | the scene an editor opens first: a scene's `.pitofx` (an old `.scene.toml` is imported once into one beside it). |
| `ignore` | array of paths | `[]` | folders (or files) `Project::open` skips, besides `target` and `tmp`; each relative to the root, so `"builds"` is the root's `builds` folder. |

`[tunables.<name>]` is one value the game reads at run time and a person tunes while it runs, such as a jump height or the gravity. The table's key is its name, which the game reads it by: it follows the name rules (`bad-name`), and a name with a space or a `.` is quoted (`[tunables."god mode"]`). A tunable takes no `id`: the game's code names it, so renaming one changes that code too.

| key | type | default | notes |
|---|---|---|---|
| `type` | `"float"`, `"int"`, `"bool"` or `"vector"` | needed | a number, a whole number, `true` or `false`, or `[x, y, z]`. |
| `default` | the tunable's type | needed | the value the game starts with. A float takes a whole number too, read as a float. |
| `min` | the tunable's type | none | float, int and vector only; at most `default` and `max`, per component for a vector. |
| `max` | the tunable's type | none | float, int and vector only; at least `default` and `min`, per component for a vector. |
| `group` | string | none | the heading an inspector lists the tunable under; a name, as above. |

A value of another type than `type` is refused (`bad-type`), as is a vector of other than three numbers. A `min` or `max` on a bool, a value that is not finite, a `min` above `max` and a `default` outside them are refused at that key (`bad-value`).

```toml
[tunables.jump_height]
type = "float"
default = 2.0
min = 0.5
max = 6.0
group = "Movement"

[tunables.gravity]
type = "vector"
default = [0.0, -9.81, 0.0]
group = "Movement"

[tunables.lives]
type = "int"
default = 3
min = 1

[tunables."god mode"]
type = "bool"
default = false
group = "Debug"
```

`Project::read` of `project.toml` gives them as `ProjectFile::tunables`, by name: each a `Tunable` whose `default`, `min` and `max` are `TunableValue`s of its `type` (`Float(f32)`, `Int(i64)`, `Bool(bool)` or `Vector([f32; 3])`).

`[layers]` names the project's collision layers, at most 32, and which of them meet. Each key is a layer's name (the name rules apply, `bad-name`), and its value lists the layers it collides with. A layer may list itself. Meeting goes both ways, so the table is symmetric: when `player` lists `pickup`, `pickup` lists `player`. A name the table doesn't hold (`bad-reference`), a name listed twice in one list, a pair listed on one side only (each at its place in the list) and a 33rd layer are refused (`bad-value`).

```toml
[layers]
world = ["world", "player", "enemy"]
player = ["world", "enemy", "pickup"]
enemy = ["world", "player"]
pickup = ["player"]
```

A body, a character and a trigger take a layer by its name (`layer`), and a trigger the layers it senses (`mask`). One without `layer` is on every layer: it meets every layer that lists any, as everything meets everything in a project without `[layers]`. Which bit a layer takes is the engine's; the crate gives the layers by name, as `ProjectFile::layers` and `Scene::layers` (a `BTreeMap` of each name to its list).

### A scene's top level

| key | type | default | notes |
|---|---|---|---|
| `format` | integer | 0 | `1`. |
| `include` | array of paths | `[]` | `*.scene.toml` entries merged in (Includes). |
| `materials` | array of paths | `[]` | material libraries. A material name in two libraries is refused, naming both files (`duplicate-name`). |
| `fallback` | reference | none | the material for a glTF material no library names; without it the loader's default material. At most one file of a scene and its includes holds it. |
| `[mesh.<name>]` | tables | none | meshes. |
| `[[object]]` | tables | none | objects and prefab placements. |
| `[[light]]` | tables | none | local lights. |
| `[[emitter]]` | tables | none | glowing spheres. |
| `[[mover]]` | tables | none | preview motions. |
| `[content.<name>]` | tables | none | images shown in a material's content layer. |
| `[text.<name>]` | tables | none | text blocks. |
| `[sound.<name>]` | tables | none | sounds. |
| `sound` | path | none | a `*.sound.toml` file holding the scene's sound, in place of `[sound.<name>]` tables: a scene gives one or the other. At most one file of a scene and its includes holds it. |
| `[[rig]]` | tables | none | camera rigs a game drives. |
| `[[tiles]]` | tables | none | tile layers: grids of prefab placements. |
| `[[particles]]` | tables | none | particle systems. |
| `[sun]`, `[sky]`, `[haze]`, `[camera]`, `[finish]`, `[flare]`, `[motion_blur]`, `[trace]`, `[shadows]`, `[live]`, `[plates]`, `[physics]`, `[play]` | table | none | each held by one file of a scene and its includes at most. |
| `[authoring]` | table | none | the authoring tool's. |

A glTF material resolves to the library entry of its name, else of its name before the first `.` (`.001` suffixes), else `fallback`.

### A prefab's top level

| key | type | default | notes |
|---|---|---|---|
| `format` | integer | 0 | `1`. |
| `include` | array of paths | `[]` | `*.prefab.toml` files merged in. |
| `materials` | array of paths | `[]` | the prefab's material libraries. |
| `[mesh.<name>]`, `[[object]]`, `[[light]]`, `[[emitter]]`, `[[mover]]`, `[content.<name>]`, `[text.<name>]`, `[sound.<name>]`, `[[rig]]`, `[[particles]]` | tables | none | as in a scene. A particle system without a `parent` rides its placement. |
| `[authoring]` | table | none | the authoring tool's. |

### `[mesh.<name>]`: geometry from glTF

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `file` | path | needed | a `.gltf` (buffers read beside it) or `.glb`. |
| `node` | string | none | the node whose subtree this mesh is. Its own placement in the file is dropped; the object places it. Without `node`, every root of the file's default scene, with their transforms. |
| `nodes` | tables | none | `[mesh.<name>.nodes."<node name>"]`: overrides of one node each. |
| `authoring` | any | none | |

`[mesh.<name>.nodes."<node name>"]` overrides one node of the file (an unknown node is refused by the loader):

| key | type | default | notes |
|---|---|---|---|
| `material` | reference | the glTF material's | a material. |
| `hidden` | bool | `false` | |
| `shadow` | `"cast"`, `"only"` or `"none"` | the object's | |
| `two_sided` | bool | the object's | |

The loader computes missing normals (smooth, area weighted); missing tangents are `[1, 0, 0, 1]` and missing UVs zero.

### `[[object]]`: an instance of a mesh, or a prefab placement

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `name` | string | needed | unique among the objects of a scene and its includes. |
| `mesh` | reference | needed without `prefab` | a mesh. A placement takes none. |
| `prefab` | path or id | none | places a prefab (Prefabs): a `*.prefab.toml` path, or the id of a placement of the same file that names a path (that placement's prefab); an id that names no such placement is `bad-reference`. |
| `set` | table | none | `[object.set]`: a placement's overrides, `"<entry>.<key path>" = value`. |
| `at` | `[x, y, z]` | `[0, 0, 0]` | |
| `rotate` | `[x, y, z]` | `[0, 0, 0]` | degrees about X, then Y, then Z. |
| `scale` | number or `[x, y, z]` | `1` | the model is `T·R·S`. |
| `parent` | reference | none | an object: the model is the parent's model times this one. A cycle is refused (`parent-cycle`). |
| `material` | reference | none | a material for every part of the mesh. |
| `materials` | table of node name = reference | none | `{ "<node name>" = "<material>", … }`: per node, over `material`. |
| `tint` | `"#rrggbb"` or `[r, g, b]`, each ≥ 0 | none | multiplies the base colour of every part the object draws, per instance: the mesh, its material and its maps stay shared. A hex colour is sRGB, three numbers are linear. On a placement it multiplies the tint of every object the placement places. |
| `recolor` | table of material name = `{ base, roughness, metalness }` | none | `[object.recolor.<material>]`: the parts that draw that material (by its library name or its glTF material name) take `base` (a colour as `tint` takes it) in place of the material's base colour, and `roughness` and `metalness` (0 to 1) in place of its own when given; at least one key. Per instance, like `tint`, which multiplies a recoloured base too. A placement's entries win over those of the objects it places. |
| `shadow` | `"cast"`, `"only"` or `"none"` | `"cast"` | `"only"` casts and is never drawn; `"none"` is drawn and casts none. |
| `two_sided` | bool | `false` | |
| `hidden` | bool | `false` | not drawn; its parts are not uploaded. |
| `clip` | array of `[nx, ny, nz, d]` | `[]` | at most two planes; a point `p` is cut where `n·p > d`. With the planes of the movers that move it, at most two. |
| `content` | reference | none | a content: the image shown in the object's material's content layer. Two objects showing different content through one slot are refused by the loader. |
| `pick` | unsigned integer | its 1-based place among the objects | the pick id. Format 0 called it `id`. |
| `alpha_cutoff` | number, 0 to 1 | `0.5` | texels of a cut-out material below this alpha are dropped. |
| `face_camera` | bool | `false` | a card: it keeps its position and scale, and turns every frame so its local +Z faces the camera with its +Y along the camera's up (its `rotate` is ignored). |
| `dynamic` | bool | derived | whether the object moves or changes at run time: a plate leaves a dynamic object out, and a live renderer draws it over the plate (`[plates]`). Without the key, an object is dynamic when a `[[mover]]` moves it, when it is a character, when it is animated, when it faces the camera, or when its parent is dynamic; every other object is static. `true` marks what a game moves itself; `false` keeps an object static, and a character or an animated object refuses it. |
| `body` | table | none | `[object.body]`: the object's rigid body. |
| `character` | table | none | `[object.character]`: a character controller that moves the object. Not with `body`. |
| `trigger` | table | none | `[object.trigger]`: a volume that senses what enters it. |
| `animation` | table | none | `[object.animation]`: the clips of its glTF, or a sprite, played on it. |
| `authoring` | any | none | |

Ten varied houses from one facade prefab: each placement keeps the kit's meshes and textures shared and draws them as instances, with its own colour.

```toml
[[object]]
id = "hm5e000001"
name = "house 1"
prefab = "kit/facade.prefab.toml"
at = [0.0, 0.0, 0.0]
tint = "#f2d8c0"

[[object]]
id = "hm5e000002"
name = "house 2"
prefab = "kit/facade.prefab.toml"
at = [8.0, 0.0, 0.0]

[object.recolor.brick]
base = [0.32, 0.12, 0.08]
roughness = 0.8

[object.recolor.trim]
base = "#e8e4dc"
```

`[object.body]` makes the object a rigid body in play mode. Every object above it is a prefab placement (or it has no parent), and no mover moves it or any of those placements; it does not face the camera. It starts at its world transform, the placements' transforms times its own, and from then on the simulation moves it; its children ride with it. A body placed under an ordinary object or under a placement a mover moves is refused (`bad-value`), at the key that puts it there: the body when it is in the file being checked, else the `parent` or the mover's `objects` that does, or the `[object.set]` key when an override does (`bad-override`). The finding names the body and the placement or object in its related locations.

| key | type | default | notes |
|---|---|---|---|
| `kind` | `"dynamic"`, `"kinematic"` or `"fixed"` | `"dynamic"` | simulated, moved by a game, or fixed. |
| `shape` | `"box"`, `"sphere"`, `"capsule"` or `"cylinder"` | `"box"` | upright along the object's local Y. |
| `half` | `[x, y, z]`, each > 0 | half the object's mesh bounds times its scale | box only. |
| `radius` | number > 0 | sphere: the largest of those halves; capsule, cylinder: the larger of the X and Z halves | sphere, capsule and cylinder. |
| `half_height` | number > 0 | capsule: the Y half less the radius; cylinder: the Y half | capsule and cylinder. |
| `offset` | `[x, y, z]` | the centre of the mesh bounds times the scale | the shape's centre in the object's frame. |
| `density` | number > 0 | `1` | |
| `friction` | number ≥ 0 | `0.5` | |
| `restitution` | number, 0 to 1 | `0` | |
| `velocity` | `[x, y, z]` | `[0, 0, 0]` | units per second at play. |
| `spin` | `[x, y, z]` | `[0, 0, 0]` | degrees per second at play. |
| `gravity_scale` | number | `1` | |
| `damping` | `[linear, angular]`, each ≥ 0 | `[0, 0]` | |
| `ccd` | bool | `false` | continuous collision detection for a fast body. |
| `layer` | layer name | none | its collision layer (`[layers]`). |

A shape takes only its own size keys: `half` for a box, `radius` for a sphere, `radius` and `half_height` for a capsule or a cylinder. The defaults that come from the mesh bounds are the loader's.

`[object.character]` makes the object a character in play mode: a kinematic capsule, upright along Y, that a game drives each tick with a move and a jump. It walks up slopes and steps, keeps to the ground, rides what it stands on, and pushes the bodies it walks into. A 2d character is the same controller held in a plane, for a platformer. A character sits where a body may (on a root object or under placements, moved by no mover, not a card), and the same findings refuse it elsewhere. It takes no `[object.body]`, and it is dynamic.

| key | type | default | notes |
|---|---|---|---|
| `kind` | `"3d"` or `"2d"` | `"3d"` | |
| `radius` | number > 0 | the larger of the X and Z halves of the object's mesh bounds times its scale | the capsule's radius. |
| `height` | number > 0, at least `2 · radius` | the Y size of those bounds | the capsule's whole height, both caps included. |
| `max_climb` | number, 0 up to (not including) 90 | `45` | degrees: the steepest slope it walks up; it slides down a steeper one. |
| `max_step` | number ≥ 0, below `height` | `0.3` | the highest step it walks onto. |
| `snap` | number ≥ 0 | `0.2` | how far below it the ground may drop, over a bump, a step down or the top of a slope, and it still keeps to it; `0` is off. |
| `coyote` | integer, 0 to 1000 | `6` | ticks after it leaves the ground in which a jump still takes off. |
| `jump_buffer` | integer, 0 to 1000 | `6` | ticks before it lands in which a jump pressed early is kept. |
| `jump_speed` | number ≥ 0 | `5` | its upward speed at take-off; `0` never jumps. |
| `layer` | layer name | none | its collision layer (`[layers]`). |
| `plane` | `"xy"` or `"yz"` | `"xy"` | 2d only: the plane it moves in, through its starting position: along X or along Z, with Y up. |
| `variable_jump` | number, 0 to 1 | `0` | 2d only: the share of its upward speed it loses when the jump is let go while it rises; `0` jumps the same height however long the jump is held. |
| `wall_slide` | number ≥ 0 | none | 2d only: the fastest it slides down a wall it presses into, in its fall; without it, a wall does not slow its fall. |
| `wall_jump` | `[away, up]`, each ≥ 0 | none | 2d only: the speeds of a jump off a wall it touches; without it, it jumps off no wall. |

A 3d character takes no `plane`, `variable_jump`, `wall_slide` or `wall_jump` (`bad-value`). The defaults that come from the mesh bounds are the loader's.

```toml
[[object]]
id = "h3r0000001"
name = "hero"
mesh = "hero"
at = [0.0, 0.9, 0.0]

[object.character]
kind = "2d"
radius = 0.3
height = 1.8
layer = "player"
variable_jump = 0.5
wall_slide = 2.0
wall_jump = [4.0, 6.0]
```

`[object.trigger]` makes the object a trigger: a volume, never solid, that tells a game when objects enter it, stay in it and leave it. It moves with its object, on any parent; an object with a trigger is often `hidden`.

| key | type | default | notes |
|---|---|---|---|
| `shape` | `"box"`, `"sphere"`, `"capsule"` or `"cylinder"` | `"box"` | upright along the object's local Y. |
| `half` | `[x, y, z]`, each > 0 | half the object's mesh bounds times its scale | box only. |
| `radius` | number > 0 | sphere: the largest of those halves; capsule, cylinder: the larger of the X and Z halves | sphere, capsule and cylinder. |
| `half_height` | number > 0 | capsule: the Y half less the radius; cylinder: the Y half | capsule and cylinder. |
| `offset` | `[x, y, z]` | the centre of the mesh bounds times the scale | the shape's centre in the object's frame. |
| `layer` | layer name | none | its own layer, which queries and other triggers see it on. |
| `mask` | array of layer names | the layers its `layer` lists; every layer without `layer` | the layers whose objects it senses. |

A shape takes only its own size keys, as a body's does.

`[object.animation]` plays an animation on the object, in play mode and in a live renderer: the clips of the object's glTF file (its skeleton's or its nodes' animation), or a sprite, the frames of an image atlas shown on the object's mesh (a quad, usually a camera-facing card), its UVs spanning one frame. An animated object is dynamic. A game plays other clips on it by name, and blends or crossfades them; these keys say what plays from the start.

| key | type | default | notes |
|---|---|---|---|
| `clip` | clip name | none | the clip it plays from the start: an animation of the object's glTF file, by its name, or a sprite's clip of `clips`. Without `clip` or `blend`, nothing plays until a game plays a clip: the object keeps its rest pose, a sprite shows its atlas's first frame. Not with `blend`. |
| `speed` | number ≥ 0 | `1` | a factor on the clip's rate; `0` holds it still. |
| `loop` | `"loop"`, `"once"` or `"ping-pong"` | `"loop"` | at the clip's end it starts again, stops on its last pose or frame, or plays back to its start and on, to and fro. A sprite's clip may set its own. |
| `start` | number ≥ 0 | `0` | seconds into the clip where play starts. |
| `blend` | array of `{ clip, weight }` | none | glTF only, instead of `clip`: clips played together, the pose each gives weighted by `weight` (a number ≥ 0, default `1`), the weights shared out by their sum, which is above 0. Not empty, each clip once. `speed`, `loop` and `start` apply to every clip of it. |
| `events` | array of `{ name, clip, time, frame }` | `[]` | marks a game is told of as play passes them, below. |
| `atlas` | path | none | a PNG: the animation is a sprite, its frames cut from this image. |
| `grid` | `[columns, rows]`, integers ≥ 1 | sprite: needed without `frames` | the atlas cut into equal cells; frames count from 0, left to right, then top to bottom. Not with `frames`. |
| `frames` | array of `[x, y, width, height]`, integers, `width` and `height` ≥ 1 | sprite: needed without `grid` | the frames' rectangles in pixels, from the atlas's top left corner; they count from 0 in the order written. Not empty. |
| `fps` | number > 0 | `12` | sprite: frames per second, for a clip that sets none. |
| `clips` | tables | none | sprite: `[object.animation.clips.<name>]`, its named clips. Without `clips`, a sprite is one clip, every frame of its atlas in order, which plays from the start. |

A glTF animation takes no `grid`, `frames`, `fps` or `clips`, and a sprite no `blend` (`bad-value`). A clip name follows the name rules (`bad-name`). The crate checks a sprite's clip names, frames and atlas path; the names of a glTF file's clips, and whether a sprite's frames lie inside its atlas, are the loader's to check, as other asset contents are.

`[object.animation.clips.<name>]` is one clip of a sprite: a run of its atlas's frames.

| key | type | default | notes |
|---|---|---|---|
| `from` | integer ≥ 0 | needed | its first frame. |
| `to` | integer, at least `from`, at most the atlas's last frame | needed | its last frame, included. |
| `fps` | number > 0 | the animation's `fps` | frames per second. |
| `loop` | `"loop"`, `"once"` or `"ping-pong"` | the animation's `loop` | |

An event of `events`:

| key | type | default | notes |
|---|---|---|---|
| `name` | string | needed | what the game is told, such as `"step"`; the name rules apply. |
| `clip` | clip name | the animation's `clip` | the clip it marks; needed when the animation plays no `clip` from the start, except in a sprite without `clips`, whose one clip takes no name. In a sprite, a clip of `clips`. |
| `time` | number ≥ 0 | none | seconds into the clip. A time past the clip's end never comes. |
| `frame` | integer ≥ 0 | none | sprite only: the frame of the clip, counting from 0 at its first (`from`), below its count of frames. |

An event takes `time` or `frame` (`missing-key`), not both (`bad-value`). An event fires each time play passes its mark, once per pass, whichever way a ping-pong clip runs.

```toml
[[object]]
id = "h3r0000001"
name = "hero"
mesh = "card"
face_camera = true

[object.animation]
atlas = "art/hero.png"
grid = [8, 4]
fps = 12.0
clip = "idle"
events = [{ name = "step", clip = "run", frame = 2 }, { name = "step", clip = "run", frame = 6 }]

[object.animation.clips.idle]
from = 0
to = 3

[object.animation.clips.run]
from = 8
to = 15
fps = 16.0

[[object]]
id = "w41ker0001"
name = "walker"
mesh = "walker"

[object.animation]
blend = [{ clip = "Walk", weight = 0.7 }, { clip = "Run", weight = 0.3 }]
events = [{ name = "step", clip = "Walk", time = 0.4 }]
```

### `[[light]]`: a local light

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `name` | string | needed | unique among the lights. |
| `kind` | `"point"` or `"spot"` | `"point"` | |
| `position` | `[x, y, z]` | needed without `parent` | the light's place in the scene. A light with a `parent` takes `at` instead. |
| `parent` | reference | none | an object the light rides, such as a car's headlight. |
| `node` | string | none | a glTF node inside the parent's mesh the light rides instead of the object itself; only with `parent`. The renderer refuses a node the mesh lacks. |
| `at` | `[x, y, z]` | `[0, 0, 0]` | the light's place in its parent's frame (the node's, with `node`); only with `parent`. |
| `direction` | `[x, y, z]`, not zero | `[0, −1, 0]` | where a spot light points, in its parent's frame (the node's, with `node`) when it has one. Spot only. |
| `cone` | `[inner, outer]` degrees | `[30, 45]` | a spot light's full strength inside `inner`, fading to nothing at `outer`; 0 ≤ inner ≤ outer ≤ 180. Spot only. |
| `color` | `[r, g, b]`, each ≥ 0 | `[1, 1, 1]` | |
| `intensity` | number ≥ 0 | needed | |
| `radius` | number ≥ 0 | `0` | |
| `range` | number > 0 | `10` | |
| `shadow` | bool | `true` | |
| `keys` | inline table | none | keyed values over time, see [Keys over time](#keys-over-time): `intensity`, `color`, `range`, `cone` (spot only). |
| `authoring` | any | none | |

### `[[emitter]]`: a glowing sphere

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `name` | string | needed | unique among the emitters. |
| `position` | `[x, y, z]` | needed | |
| `radius` | number > 0 | needed | |
| `color` | `[r, g, b]`, each ≥ 0 | `[1, 1, 1]` | |
| `intensity` | number ≥ 0 | needed | it radiates `intensity / (π radius²)` per channel. |
| `authoring` | any | none | |

### `[[mover]]`: a preview motion

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `name` | string | needed | unique among the movers. |
| `objects` | array of references | needed, not empty | what it moves, with their children. An object is moved by one mover at most. |
| `kind` | `"turn"` or `"slide"` | needed | degrees about `axis` through `pivot`, or units along `axis`. |
| `pivot` | `[x, y, z]` | `[0, 0, 0]` | turn only, in the scene's rest frame. A slide takes no pivot. |
| `axis` | `[x, y, z]`, not zero | needed | |
| `travel` | `[from, to]` | needed | degrees or units. |
| `period` | number > 0 | `4` | seconds. |
| `motion` | `"swing"`, `"loop"` or `"once"` | `"swing"` | swing: from, to and back, eased by `(1 − cos 2πu) / 2`; loop: from to to, then again; once: from to to, then stays. |
| `offset` | number | `0` | seconds added to the time. |
| `clip` | array of `[nx, ny, nz, d]` | `[]` | world planes that cut what it moves, fixed while it moves; at most two, and at most two with each moved object's own. |
| `authoring` | any | none | |

At time `t`, `u = (t + offset) / period` and the mover stands at `from + (to − from) · f(u)`. A child's own mover applies first, then its parent's, up the chain.

### `[content.<name>]`

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `image` | path | needed | a PNG. |
| `authoring` | any | none | |

### `[text.<name>]`

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `text` | string | needed | the words. |
| `font` | path | needed | a `.ttf` or `.otf`. |
| `size` | number > 0 | needed | in scene units per em. |
| `color` | `[r, g, b, a]`, each ≥ 0 | `[1, 1, 1, 1]` | |
| `at` | `[x, y, z]` | `[0, 0, 0]` | the block's centre. |
| `rotate` | `[x, y, z]` | `[0, 0, 0]` | as an object's. |
| `lit` | bool | `false` | unlit text glows its colour; lit text is shaded by the scene. |
| `dynamic` | bool | `false` | text a game changes or moves is drawn live over a plate; static text is baked into it. |
| `authoring` | any | none | |

The block reads along its local +X with +Y up and faces +Z.

### `[sound.<name>]`

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `file` | path | needed | a WAV or an Ogg Vorbis file. |
| `volume` | number ≥ 0 | `1` | |
| `pan` | number, −1 to 1 | `0` | |
| `loop` | bool | `false` | loops as the music track; one looping sound per scene at most. |
| `play` | `"start"`, `"hit"` or `"game"` | `"start"` | when play starts, when `object`'s body starts a contact, or when a game plays it. |
| `object` | reference | needed for `"hit"`, refused otherwise | an object with a `[object.body]`. |
| `authoring` | any | none | |

### `[[particles]]`: a particle system

Every key but `name` is optional; a key left out takes the renderer's default.

| key | type | notes |
|---|---|---|
| `id` | id | |
| `name` | string, needed | unique among the particle systems. |
| `parent` | reference | an object the system rides; its particles are born in the parent's frame. |
| `node` | string | a glTF node inside the parent's mesh the system rides instead of the object itself; only with `parent`. The renderer refuses a node the mesh lacks. |
| `at` | `[x, y, z]` | where particles are born: in the parent's frame (the node's, with `node`), or the scene's without a parent. |
| `rate` | number ≥ 0 | particles born per second. |
| `rate_keys` | array of `[seconds, rate]`, each ≥ 0, in time order | the rate over time, linear between keys; when present it drives the rate and `rate` is ignored. Another spelling of `keys.rate`; not with it. |
| `burst` | unsigned integer | particles born at once when the system starts. |
| `max` | integer ≥ 1 | the most particles alive at once. |
| `lifetime` | `[low, high]` seconds, low > 0 | each particle lives a time between the two. |
| `speed` | `[low, high]`, each ≥ 0 | each particle's starting speed, units per second. |
| `direction` | `[x, y, z]`, not zero | the direction particles leave in, in the same frame as `at`. |
| `cone` | number, 0 to 180 | degrees of spread about `direction`. |
| `inherit` | number, 0 to 1 | how much of the parent's velocity a particle keeps. |
| `gravity` | `[x, y, z]` | acceleration, units per second². |
| `drag` | number ≥ 0 | |
| `size` | two numbers ≥ 0 | sizes in scene units. |
| `alpha` | array of `[t, alpha]`, each 0 to 1 | opacity over a particle's life, `t` from birth (0) to death (1). |
| `color` | colour | one colour for the whole life. |
| `colors` | array of `[t, colour]`, `t` 0 to 1 | colour over a particle's life. |
| `spin` | two numbers | degrees per second. |
| `stretch` | number ≥ 0 | how far a particle stretches along its motion. |
| `blend` | `"alpha"` or `"add"` | `"alpha"` by default. |
| `lit` | bool | lit particles are shaded by the scene. |
| `seed` | unsigned integer | |
| `prewarm` | number 0 to 30 | seconds simulated before t = 0 with the parent and keys held at t = 0, so the effect is formed on the first frame; 0 by default. |
| `keys` | inline table | keyed values over time, see [Keys over time](#keys-over-time): `rate`, `speed`, `size`, `color` (`[r, g, b]`) and `alpha` (0 to 1, scaling the `alpha` curve). |
| `authoring` | any | |

A *colour* is `"#rrggbb"` (sRGB, read as linear) or `[r, g, b]`. `colour` and `colours` read as `color` and `colors`.

### `[[rig]]`: a camera rig

A rig describes how a game moves the camera, each frame, from its target and the game's look input. A scene may hold several. pfx-scene checks and resolves them (`Scene::rigs`); pfx's play mode does not read them, and takes its rigs from a `rigs.toml` at the project root instead ([camera-rigs.md](camera-rigs.md)). A renderer without a game uses `[camera]`.

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `name` | string | needed | unique among the rigs. |
| `kind` | `"follow"`, `"first-person"` or `"third-person"` | needed | |
| `target` | reference | none | the object it follows or looks from at the start. |
| `offset` | `[x, y, z]` | follow: `[camera] at` less the target's position at rest; first-, third-person: `[0, 0, 0]` | follow: the camera's place from the followed point; first-person: the eye from the target's origin; third-person: the point it orbits, from the target's origin. |
| `dead_zone` | `[across, up]`, each ≥ 0 | `[0, 0]` | follow: how far the target moves from the followed point, across and up the view, before the camera follows. |
| `look_ahead` | number ≥ 0 | `0` | follow: how far ahead of the target the followed point leads, in its direction of travel. |
| `damping` | `[x, y, z]`, each ≥ 0 | `[0, 0, 0]` | follow: seconds, per world axis, in which the camera closes most of its gap to where it should be; `0` keeps up at once. |
| `bounds` | `{ min = [x, y, z], max = [x, y, z] }`, `min` at most `max` on each axis | none | follow: the box the camera's position never leaves. |
| `orthographic` | number > 0 | none | follow: the camera views orthographically, this many units high; without it, it keeps `[camera]`'s projection. |
| `snap` | number > 0 | none | follow, with `orthographic`: the art's pixels per unit; the camera moves by whole art pixels. |
| `sensitivity` | number > 0 | `1` | first-, third-person: a factor on the look input. |
| `invert` | bool | `false` | first-, third-person: looking up and down is turned over. |
| `smoothing` | number ≥ 0 | `0` | first-, third-person: seconds in which the look closes most of its gap to the input; `0` is off. |
| `pitch` | `[min, max]`, each −90 to 90, `min` at most `max` | `[-85, 85]` | first-, third-person: the degrees the look turns down and up to. |
| `head_bob` | number ≥ 0 | `0` | first-person: the height of the eye's bob as the target walks, growing with its speed; `0` is off. |
| `distance` | number > 0 | `4` | third-person: how far behind the orbited point the camera sits. |
| `collide` | bool | `true` | third-person: the camera stops short of what lies between it and the orbited point. |
| `authoring` | any | none | |

A follow rig keeps `[camera]`'s view direction and up and moves the camera, from the target and its `offset`, `dead_zone`, `look_ahead`, `damping` and `bounds`; a 2d game's follow rig is usually `orthographic`. A first-person rig puts the eye at the target's position plus `offset` and turns the view with the look input. A third-person rig orbits the target's position plus `offset` at `distance`, turned by the look input. A rig refuses the keys of the other kinds (`bad-value`).

```toml
[[rig]]
id = "s1dec4m001"
name = "side"
kind = "follow"
target = "hero"
offset = [0.0, 1.0, 10.0]
dead_zone = [0.5, 0.25]
look_ahead = 1.5
bounds = { min = [-20.0, 0.0, 10.0], max = [20.0, 8.0, 10.0] }
orthographic = 9.0
snap = 16.0
```

### `[[tiles]]`: a tile layer

A tile layer is a grid in a plane whose cells place prefabs: a platformer's level, or a grid level's floor plan. Each filled cell is a placement of its tile's prefab, unturned and unscaled, at the cell's position, as an `[[object]]` with `prefab` places one (Prefabs). A scene and its includes may hold several layers; a prefab holds none.

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `name` | string | needed | unique among the tile layers. |
| `cell` | number > 0, or `[across, up]`, each > 0 | `1` | a cell's size along the plane's two axes. |
| `origin` | `[x, y, z]` | `[0, 0, 0]` | where cell `[0, 0]` places its prefab. |
| `plane` | `"xy"`, `"xz"` or `"yz"` | `"xy"` | the plane the layer lies in, below. |
| `palette` | table of tile = prefab | `{}` | each tile's key and the prefab it places: a `*.prefab.toml` path, or a placement's id (that placement's prefab). |
| `rows` | array of strings | none | the cells as a picture: one string per row, one character per cell, `.` for an empty one. The last string is row `j = 0` and each string above it one row further; a string's first character is `i = 0`. Not with `cells`. |
| `cells` | array of `{ at = [i, j], tile }` | none | the filled cells as a list: `at`, two integers, which may be negative, and `tile`, a palette key. Each cell at most once. Not with `rows`. |
| `authoring` | any | none | |

| `plane` | `i` runs along | `j` runs along | seen from |
|---|---|---|---|
| `"xy"` | +X | +Y | +Z: the side, Y up. |
| `"xz"` | +X | −Z | +Y: above, with −Z ahead. |
| `"yz"` | −Z | +Y | +X: the side, Y up. |

Cell `[i, j]` places its prefab at `origin + i · across · I + j · up · J`, where `I` and `J` are the directions `i` and `j` run along and `across` and `up` the sides of `cell`. Seen from where the table says, `i` runs right and `j` up, so `rows` reads as the layer looks.

A palette key holds no space and is not `.` (`bad-name`); in a layer with `rows`, each key is one character (`bad-value`). A character of `rows` or a `tile` of `cells` that names no palette key is refused at that character or key (`bad-reference`), and so is a palette id that names no placement of the scene. A palette path is checked as any path is, and must name a `*.prefab.toml`; each palette prefab is checked with the scene.

A cell's placed entries take no id or name the scene can refer to: no reference, override or mover reaches them. A body in a tile's prefab sits under its cell's placement, which no mover moves.

```toml
[[tiles]]
id = "gr0nd00001"
name = "ground"
cell = 0.5
origin = [-4.0, 0.0, 0.0]
palette = { "#" = "tiles/brick.prefab.toml", "^" = "tiles/spike.prefab.toml" }
rows = [
  "....^...",
  "########",
]

[[tiles]]
id = "f100r00001"
name = "floor plan"
cell = [2.0, 3.0]
plane = "xz"
palette = { wall = "tiles/wall.prefab.toml" }
cells = [{ at = [0, 0], tile = "wall" }, { at = [-1, 2], tile = "wall" }]
```

### `[sun]`

| key | type | default | notes |
|---|---|---|---|
| `model` | `"daylight"` or `"authored"` | needed | |
| `hour` | number, 0 to 24 | daylight: needed; authored: `reference_hour` | |
| `day` | number, 1 to 365 | `172` | day of the year. |
| `latitude` | number, −90 to 90 | `45` | |
| `heading` | number, 0 to 360 | `180` | |
| `reference_hour` | number, 0 to 24 | `12` | the hour of intensity 1. |
| `toward` | `[x, y, z]`, not zero | authored: needed | the direction towards the sun, as authored at `reference_hour`. Daylight refuses it. |
| `color` | `[r, g, b]`, each ≥ 0 | authored: `[1, 1, 1]` | Daylight refuses it. |
| `irradiance` | number ≥ 0 | authored: needed | Daylight refuses it. |
| `radius` | number, 0 to 10 | `0` | the sun's angular radius in degrees: a soft sun. |
| `keys` | inline table | none | keyed values over time, see [Keys over time](#keys-over-time): `hour`. |

An authored sun follows `hour`: at `hour = reference_hour` (the default) it is exactly as authored, and away from it the sun turns, lowers, warms and dims as the day does. Without `[sun]` the sun is dark.

### `[sky]`

| key | type | default | notes |
|---|---|---|---|
| `kind` | `"analytic"`, `"hdr"`, `"room"` or `"mix"` | needed | |
| `path` | path | hdr: needed | the Radiance `.hdr` file. hdr only. |
| `rotation_deg` | number | `0` | hdr and room: turns the sky by its negative. |
| `intensity` | number ≥ 0 | `1` | analytic, hdr and room: scales the sky's radiance everywhere, as seen and as the light it casts. |
| `backdrop` | number > 0 | `1` | scales only the sky as seen behind the scene, not the light it casts. Any kind; not on a layer. |
| `horizon_color` | `[r, g, b]`, each ≥ 0 | none | a tint blended into the seen sky near the horizon. Any kind; not on a layer. |
| `horizon_width` | number, above 0 and at most 90 | `10` | degrees above the horizon over which the tint fades out. |
| `horizon_amount` | number, 0 to 1 | `0` | the tint's strength at the horizon; without it there is no tint. |
| `turbidity` | number, 1 to 10 | `3` | analytic only. |
| `ground_albedo` | `[r, g, b]`, each ≥ 0 | `[0.2, 0.2, 0.2]` | analytic only. |
| `ambient` | number ≥ 0 | `1` | analytic only, with an authored sun: the sky's level. |
| `prepare` | table | none | `[sky.prepare]`, hdr only. |
| `room` | table | room: needed | `[sky.room]`, room only. |
| `layer` | tables | none | `[[sky.layer]]`, mix only: two or more. |
| `keys` | inline table | none | keyed values over time, see [Keys over time](#keys-over-time): `intensity`, `backdrop`, `horizon_color`, `horizon_width`, `horizon_amount`. |

`backdrop` and the `horizon_` keys shape only what the camera sees of the sky, after any mix; its lighting is untouched. An analytic sky takes `turbidity`, `ground_albedo`, `ambient` and `intensity` and follows the sun; an hdr sky takes `path`, `rotation_deg`, `intensity` and `[sky.prepare]`; a room sky takes `[sky.room]`, `rotation_deg` and `intensity`; a mix sky takes only `[[sky.layer]]`. Any other key of `[sky]` is refused for that kind (`bad-value`). Without `[sky]` the sky is black.

`[sky.prepare]` prepares the HDR before it turns, in this order; it needs at least one key:

| key | type | default | notes |
|---|---|---|---|
| `cap` | number > 0 | none | caps the luminance. |
| `balance` | `[r, g, b]`, each > 0 | none | balances to this colour. |
| `mean` | number > 0 | none | scales to this mean luminance. |

`[sky.room]` is a room lit by lamps, rendered into the sky:

| key | type | default | notes |
|---|---|---|---|
| `width` | integer, 2 to 8192 | `64` | the sky's width in texels; its height is half. |
| `floor` | `[r, g, b]`, each ≥ 0 | `[0.5, 0.5, 0.5]` | |
| `wall` | `[r, g, b]`, each ≥ 0 | `[0.5, 0.5, 0.5]` | |
| `ceiling` | `[r, g, b]`, each ≥ 0 | `[0.5, 0.5, 0.5]` | |
| `lights` | tables | none | `[[sky.room.lights]]`. |

`[[sky.room.lights]]`:

| key | type | default | notes |
|---|---|---|---|
| `name` | string | `""` | a label. |
| `az` | number | `0` | azimuth, degrees. |
| `el` | number | `0` | elevation, degrees. |
| `power` | number ≥ 0 | `1` | |
| `color` | `[r, g, b]`, each ≥ 0 | `[1, 1, 1]` | |
| `width` | number, above 0 and below 180 | `45` | degrees. |
| `height` | number, above 0 and below 180 | `45` | degrees. |
| `soft` | number ≥ 0 | `0.1` | edge softness. |
| `slats` | number ≥ 0 | `0` | blind slats across the lamp. |
| `open` | number | `1` | how open the slats are. |

`[[sky.layer]]` is one sky of kind `analytic`, `hdr` or `room`, with that kind's keys and its own `[sky.layer.prepare]` or `[sky.layer.room]`, and one more key:

| key | type | default | notes |
|---|---|---|---|
| `weight` | number ≥ 0 | `1` | the layer's share; the mix is the weighted sum, not renormalised. Only a layer takes it. |
| `keys` | inline table | none | keyed values over time, see [Keys over time](#keys-over-time): `intensity`. |

A layer is not a mix and holds no layers. The mix is baked once, on load, into one HDR.

### `[haze]`: a room or outdoor haze

A haze fills a box (a room haze, with `lo` and `hi`) or the open air (an outdoor haze, with `height` or `falloff`). An outdoor haze needs no box, and its `reach` is how far the march goes, in metres.

| key | type | default | notes |
|---|---|---|---|
| `lo` | `[x, y, z]` | needed for a room haze | the haze's lower bounds. |
| `hi` | `[x, y, z]` | needed for a room haze | its upper bounds; `lo` is below `hi` on every axis. |
| `height` | number | none | an outdoor haze: the height its density is measured from. |
| `falloff` | number ≥ 0 | none | an outdoor haze: how fast its density thins with height. |
| `shafts` | number, 0 to 1 | none | the strength of light shafts through the haze. |
| `steps` | integer ≥ 1 | none | march steps. |
| `bounces` | integer, 0 to 8 | `2` | an outdoor haze: the path tracer's scattering bounces inside the fog; `0` lights the fog by the sun, the sky and the lamps once and scatters no further. The live renderer ignores it. |
| `color` | colour | none | the haze's colour; `colour` reads the same. |
| `amount` | number, 0 to 1 | `0` | sets the four densities below; without it the haze is clear. |
| `fog` | number ≥ 0 | `amount / 6` | |
| `smoke` | number ≥ 0 | `amount · 20` | |
| `mist` | number ≥ 0 | `amount · 0.2` | |
| `floor` | number ≥ 0 | `amount · 2` | |
| `phase` | number, −0.9 to 0.9 | `0.3` | |
| `back` | number ≥ 0 | `0.5` | |
| `reach` | number ≥ 0 | `3` | |
| `gold` | `[r, g, b]`, each ≥ 0 | `[1, 1, 1]` | |
| `ambient` | `[r, g, b]`, each ≥ 0 | `[0.03, 0.03, 0.03]` | |
| `unmapped` | number ≥ 0 | `0` | |
| `seed` | unsigned integer | `0` | |
| `keys` | inline table | none | keyed values over time, see [Keys over time](#keys-over-time): `amount`, `color` (`[r, g, b]`). |

### `[camera]`

| key | type | default | notes |
|---|---|---|---|
| `projection` | `"perspective"` or `"orthographic"` | `"perspective"` | |
| `at` | `[x, y, z]` | `[0, 0, 5]` | differs from `look_at`. |
| `look_at` | `[x, y, z]` | `[0, 0, 0]` | |
| `up` | `[x, y, z]` | `[0, 1, 0]` | not along the view. |
| `fov` | number, above 0 and below 180 | `40` | perspective: vertical degrees. Not with `focal`. |
| `focal` | number > 0 | none | perspective, instead of `fov`: focal length in mm. |
| `sensor` | number > 0 | `24` | perspective, with `focal`: sensor height in mm. Without `focal` it is refused, unless `fov` is given. |
| `height` | number > 0 | orthographic: needed | the view's height in scene units. A perspective camera refuses it. |
| `near` | number > 0 | `0.05` | |
| `far` | number > `near` | `100` | |
| `shift` | `[x, y]` | `[0, 0]` | lens shift in halves of the view, right and up (`0.5` moves the picture a quarter of its width). |
| `fstop` | number > 0 | none | perspective: depth of field through a thin lens of the camera's focal length at this f-number. Without it everything is sharp. |
| `focus` | number | the distance from `at` to `look_at` | perspective, with `fstop` only: the focus distance in scene units, beyond the focal length. |
| `focus_keys` | array of `[seconds, distance]` | none | perspective, with `fstop` only: the focus distance over time, linear between keys and held past the ends; seconds ≥ 0 and in order, distances > 0. When present it drives the focus and `focus` is ignored. Another spelling of `keys.focus`; not with it. |
| `preset` | table | none | `[camera.preset]`. |
| `keys` | inline table | none | keyed values over time, see [Keys over time](#keys-over-time): `fov` (not with `focal`), `fstop`, `focus`. |

An orthographic camera refuses `fov`, `focal`, `sensor`, `fstop` and `focus`, static or keyed.

`[camera.preset]` tunes the camera director over its defaults. Angles are degrees; every value is a number unless said.

| key | keys | defaults |
|---|---|---|
| `gain` | a number | `1` |
| `orbit` | `stiffness`, `yaw`, `pitch`, `hold` | `4`, `0.5°`, `0.25°`, `2` |
| `look` | `stiffness`, `lean`, `yaw`, `pitch`, `sign` | `3`, `0.2`, `0.6°`, `0.4°`, `1` |
| `zoom` | `stiffness`, `hold`, `max`, `rate` | `3`, `2`, `2`, `1.1` |
| `wander` | `idle`, `every`, `hold` | `8`, `12`, `3` |
| `nudge` | `stiffness`, `pitch`, `hold`, `floor` | `30`, `0.1°`, `0.2`, `0` |
| `hush` | `floor`, `stiffness`, `blocks_cursor` (bool) | off; when given, all three are needed |
| `depth` | `focus`, `blur`, `floor` | off; when given, all three are needed |

Each sub-table is optional and each of its keys tunes one default.

### `[finish]`

The image's finish: a chain of post passes. `[finish]` takes either `file`, a finish file, or the finish keys inline; `[[finish.pass]]` may follow either.

| key | type | default | notes |
|---|---|---|---|
| `file` | path | none | a finish file. With `file`, `[finish]` holds no inline key. |
| the finish keys | below | none | inline. |
| `pass` | tables | none | `[[finish.pass]]`. |
| `keys` | inline table | none | keyed values over time, see [Keys over time](#keys-over-time): `exposure`, `lut_amount`; with `file` too. |

Inline keys build the chain as the finish file does: a style's passes first, then each key adds or tunes its pass, keys taken in name order, not in the order written. For an order of your own, list passes in `[[finish.pass]]`: they apply in the order written, after the base (the file's chain, or the style and inline keys). Each `[[finish.pass]]` is a table of finish keys that builds its passes from nothing, usually one key or the keys of one pass (`cel_bands` with `cel_shadow`). A pass that builds nothing (empty, or only `encode = false`), or names `style`, `seed`, `frame` or `file`, is refused. Without `[finish]` the engine's standard finish applies.

A finish file holds the finish keys at its top level, and nothing else: no `format`, no `file`, no `pass`.

```toml
[finish]
exposure = 1.1

[[finish.pass]]
warmth = 0.2

[[finish.pass]]
tone = "agx"
```

The finish keys. A *colour* is `"#rrggbb"` (sRGB, read as linear) or `[r, g, b]`. A *whole number* is a number ≥ 0 that is rounded. Every number is finite.

| key | type | notes |
|---|---|---|
| `aberration` | number | the red and blue channels pushed this many pixels apart, out from the frame's centre. |
| `angle` | number | halftone screen angle, degrees. |
| `black` | number | black level. |
| `black_threshold` | number | modern comic: black threshold. |
| `bloom` | number or table | a number is the bloom's strength; the table below. |
| `cavity` | number | cavity shading's strength. |
| `cavity_distance` | number | cavity shading's distance. |
| `cel_bands` | whole number | cel shading's bands. |
| `cel_shadow` | number | cel shading. |
| `cel_softness` | number | cel shading. |
| `cel_spec` | number | cel shading: specular. |
| `cel_spec_roughness` | number | cel shading. |
| `cel_spec_threshold` | number | cel shading. |
| `cel_threshold` | number | cel shading. |
| `chromatic` | number or table | a lens's radial chromatic aberration: a number is its `amount`; the table below. |
| `contrast` | number | contrast. |
| `crease_angle` | number | outline crease angle, degrees. |
| `crush` | number | noir: black crush. |
| `curvature` | number | screen curvature. |
| `darkening` | number | watercolour edge darkening. |
| `dither` | bool or number | dither amplitude; `true` is `1`, `false` is `0`. |
| `dither_kind` | `"blue"` or `"ordered"` | one-bit dither pattern. |
| `distortion` | number | lens distortion. |
| `dot` | number | halftone cell size. |
| `dust` | number | film dust. |
| `encode` | bool | `true` adds the encode pass; `false` adds nothing. |
| `exposure` | number | exposure. |
| `flicker` | number | film flicker. |
| `focus` | number | tilt-shift focus. |
| `frame` | unsigned integer | the frame for animated passes. Not in a pass. |
| `gain` | number | ACES tone gain. |
| `grain` | number or table | a number is the grain's strength; the table below. |
| `halation` | number | halation strength. |
| `highlight` | colour | duotone highlight. |
| `highlight_threshold` | number | modern comic. |
| `ink` | number or colour | a number is comic ink; a colour is halftone ink. |
| `interior_line_strength` | number | modern comic. |
| `keep` | colour | noir: the colour kept. |
| `keep_range` | number | noir: how much of it. |
| `levels` | number | posterize (or comic) levels. |
| `line_weight` | number | modern comic. |
| `lut` | path or table | a colour lookup table: a path to a `.cube` file, or the table below. |
| `lut_amount` | number, 0 to 1 | how much of the lookup table applies. |
| `mask` | number | aperture grille. |
| `outline` | bool or table | turns the outline on or off; the table below. |
| `outline_alpha` | number | outline opacity. |
| `outline_color` | colour | outline colour. |
| `outline_thickness` | number | outline thickness. |
| `palette` | bool | pixel art: palette on or off. |
| `paper_grain` | number | paper grain (or watercolour grain) strength. |
| `pixel` | unsigned integer | pixel size; `0` reads as `1`. |
| `posterize` | number | the same as `levels`. |
| `radius` | number | the radius of the bloom, kuwahara, halation, neon or tilt-shift pass already in the chain; without one, a kuwahara pass. |
| `rim` | number | rim light strength. |
| `rim_color` | colour | rim light colour. |
| `rim_width` | number | rim light width. |
| `saturation` | number | saturation. |
| `saturation_lift` | number | modern comic. |
| `scale` | number | paper grain or watercolour scale. |
| `scanlines` | number | scanline strength. |
| `scratches` | number or table | a number is the scratches' strength; the table below. |
| `seed` | unsigned integer | the chain's seed. Not in a pass. |
| `sepia` | number | sepia. |
| `shadow` | colour | duotone shadow. |
| `style` | a style name | a style's passes first. Not in a pass. |
| `tape` | `"forward"`, `"rewind"`, number or table | a tape look: a direction, a strength, or the table below. |
| `threshold` | number | bloom threshold. |
| `tilt_range` | number | tilt-shift range. |
| `tone` | `"aces"`, `"agx"`, `"neutral"` or table | the tone map; the table below. |
| `tone_steps` | number, 3 to 5 | modern comic tone steps. |
| `vignette` | number or table | a number is the vignette's strength; the table below. |
| `warmth` | number or table | a number is the warmth's amount; the table below. |
| `weave` | number | film weave. |
| `weights` | `[r, g, b]` | black-and-white or saturation weights. |

The style names: `cel`, `bw`, `noir`, `vignette`, `neon`, `rubber_hose`, `sepia`, `film`, `crt`, `one_bit`, `halftone`, `comic`, `modern_comic`, `watercolor`, `paper_grain`, `pixel`, `duotone`, `gradient_map`, `posterize`, `kuwahara`, `tilt_shift`.

The finish keys' tables (each key optional unless said):

| table | keys |
|---|---|
| `chromatic` | `amount` (0 to 0.25, default `0.02`); `falloff` (0 to 8, default `0`) |
| `bloom` | `strength`, `threshold`, `knee`, `knee_width`, `soft`, `sigma`, `clamp`, `unit` (numbers); `radius` (whole number); `down` (whole number; `0` reads as `1`); `linear_taps` (bool); `radii`, `gains` (four numbers each); `kind` (`"box"`, `"rings"` or `"tent"`) |
| `grain` | `strength` or its other name `amount` (one of the two), `response` (numbers); `size` (pixels, above 0 up to 64, default `1`); `seed` (unsigned integer, default `0`); `clamp` (bool) |
| `lut` | `size` (integer, 2 to 64, needed); `values` (`size³ · 3` numbers, red, green and blue per entry, needed) |
| `outline` | `enabled`, `local_color` (bools); `thickness`, `alpha`, `crease_angle` (numbers); `color` (colour) |
| `scratches` | `count` (unsigned integer); `strength` (number) |
| `tape` | `direction` (`"forward"` or `"rewind"`); `strength` (number ≥ 0, read up to 1); `bands`, `echo`, `grain`, `smear`, `split`, `wash` (numbers ≥ 0) |
| `tone` | `kind` (`"aces"`, `"agx"` or `"neutral"`, needed); `start` (number, neutral only, default `1`); `clamp` (bool, neutral only, default `false`) |
| `vignette` | `strength`, `power`, `aspect`, `scale`, `inner`, `outer` (numbers); `kind` (`"power"` or `"smoothstep"`) |
| `warmth` | `amount`, `shadow`, `highlight` (numbers); `low`, `high` (two numbers each); `kind` (`"linear"` or `"luma_curve"`) |

A `tape` number is a strength ≥ 0, read up to 1.

**Film grain** adds noise to every pixel, its spread set by `strength` (`amount`) and lowered in bright pixels by `response` (the noise times `1 − response × luma`). `size = 1` is a fresh value per pixel; a larger `size` is a smooth value noise with cells `size` pixels across, scaled so its spread matches the fine grain's. The grain is seeded by the chain's `seed`, the grain's own `seed` and the frame: a still or a clip frame rendered twice has the same grain, and a clip's grain moves from frame to frame, since the frame is the shot's time × 60 on the live path and the traced one alike. A crop of a traced still keeps the whole frame's grain at its pixels.

**Chromatic aberration** (`chromatic`) is a lens's radial colour split, ported from Bevy's (the Playdead *Inside* technique): each pixel averages up to 8 samples along the line from it toward the frame's centre, `amount` of the way there, weighted red at the pixel, green halfway and blue at the far end, so a flat frame stays flat and the frame's centre never moves. `falloff` shapes it across the frame: the length is scaled by r<sup>falloff</sup>, with r the distance from the centre over the corner's, so `0` grows the split evenly out from the centre and a larger number holds it to the edges. It runs before the bloom, after a leading `exposure`. A traced crop of a finish with `chromatic` is refused for a `.png` or a master, as the split reads past a crop's margin.

**Their cost at 3840x2160**, on an AMD Radeon RX 9060 XT and a Ryzen 9 9950X (32 threads) under Linux, through a GPU queue other jobs shared (load average 30 to 43), medians over 60 frames (`benchmark_4k_finish_breakdown` in `crates/post/src/gpu.rs`) and over 7 runs on the CPU:

| pass | live (GPU) | traced finish (CPU) |
|---|---|---|
| `chromatic`, the default amount | 0.56 to 0.95 ms, a compute pass of its own; ahead of a leading exposure, so the grade stays one fused pass | 135 ms |
| `grain`, `size = 1` | inside the fused grade | 62 ms |
| `grain`, `size = 2` | 0.1 to 0.3 ms added to the fused grade | 53 ms |

### `[flare]`

A lens flare from the image's bright parts. Every key is optional; a key left out takes the renderer's default.

| key | type | notes |
|---|---|---|
| `amount` | number, 0 to 1 | the flare's strength. |
| `ghosts` | unsigned integer | ghost count. |
| `ghost_spacing` | number ≥ 0 | |
| `halo` | number ≥ 0 | the halo's strength. |
| `halo_radius` | number ≥ 0 | |
| `starburst` | number ≥ 0 | |
| `threshold` | number ≥ 0 | the brightness a flare starts from. |

### `[motion_blur]`

Every key is optional; a key left out takes the renderer's default.

| key | type | notes |
|---|---|---|
| `shutter` | number, 0 to 4 | up to 1, the share of a frame the shutter stays open; above 1, a stylised streak of that many frames' motion. |
| `max_px` | number ≥ 0 | the longest streak at a shutter of 1, in pixels; a shutter above 1 multiplies it. |
| `samples` | integer ≥ 1 | |
| `position` | `"start"`, `"centre"` or `"end"` | where the shutter sits against the frame's time: it opens at the frame, is centred on it (the default) or closes at it, as Cycles' motion position. The live renderer and the tracer read it alike. |

### `[trace]`

Settings only a path tracer reads; a live renderer ignores them.

| key | type | default | notes |
|---|---|---|---|
| `transmissive_shadows` | bool | `false` | shadow rays pass glass and liquid surfaces, tinted, instead of stopping at them. |
| `clamp_indirect` | number, 0 to 10000 | `0` (off) | the luminance cap on each light contribution reached after the first bounce; a brighter one is scaled down to the cap, keeping its hue. Light at the first hit is never clamped. |
| `filter_glossy` | number, 0 to 1 | `0` (off) | the least roughness at every vertex after a diffuse bounce: once a path has bounced off a diffuse surface, sharper lobes are traced at least this rough. The first hit, and a chain of mirror, glossy or glass bounces from the camera, keep their own roughness. |

Both remove fireflies, the lone bright pixels a sharp reflection seen in diffuse light makes, at a cost to the look: `clamp_indirect` takes energy from bright caustics and from what a mirror or glass reflects, so it belongs well above the sunlit brightness of the brightest diffuse surface; `filter_glossy` keeps the energy but softens glints and caustics. `filter_glossy = 0.25`, with `clamp_indirect` as a backstop, is a good start for a small sun.

### `[shadows]`

The live renderer's sun shadows and its two screen-space terms: ambient occlusion on the indirect light, and contact shadows along the sun for the gap where an object meets what it rests on. A path tracer ignores the table. Without it a scene renders exactly as `quality = "medium"`, with both screen-space terms off. Every key is optional; a key left out takes the preset's value.

| key | type | default | notes |
|---|---|---|---|
| `quality` | `"low"`, `"medium"`, `"high"` or `"ultra"` | `"medium"` | the preset below. |
| `cascades` | integer, 1 to 3 | preset | sun shadow cascades drawn. |
| `resolution` | integer, 256 to 8192 | preset | the side of each cascade's shadow map, in texels. |
| `softness` | number, 0 to 8 | `1` | the filter radius, in shadow-map texels. |
| `bias` | number above 0, up to 16 | `1` | a scale on the normal, slope and depth bias. Lower keeps shadows closer to their casters, at the risk of acne. |
| `occlusion` | bool | preset | screen-space ambient occlusion. |
| `occlusion_radius` | number above 0, up to 100 | `0.5` | the occlusion's reach in scene units. |
| `contact` | bool | preset | screen-space contact shadows along the sun. |
| `contact_length` | number above 0, up to 10 | `0.3` | how far each pixel's ray toward the sun reaches, in scene units. |

| preset | cascades | resolution | occlusion | contact |
|---|---|---|---|---|
| `low` | 2 | 2048 | off (1 slice × 2 steps when on) | off |
| `medium` | 3 | 4096 | off (2 × 2 when on) | off |
| `high` | 3 | 4096 | on, 3 × 3 | on, 8 steps |
| `ultra` | 3 | 8192 | on, 9 × 3 | on, 16 steps |

`docs/live-renderer.md` ("Ambient occlusion and contact shadows") has what each term does and what it costs.

### `[live]`

How the live renderer samples over time. A path tracer ignores the table. Without it, or with a key left out, the renderer behaves as its defaults below.

| key | type | default | notes |
|---|---|---|---|
| `stochastic_reflections` | bool | `true` | screen-space reflections sample a GGX visible normal per pixel and frame on surfaces rougher than 0.18, start each ray's march at a per-pixel offset, and denoise the trace. `false` traces one fixed ray per pixel instead: the mirror direction with an 8-frame jitter, from a fixed march start, no prefilter, a ray that ends inside a solid object misses rather than reading dark (the buried-ray test needs the per-pixel offset; on a fixed start it bands), and a ray may hit the sky backdrop at the far plane, as before. At a grazing view the sampled lobe sends some rays back toward the camera; they leave the screen and take the environment's reflection, so a glossy silhouette shows a thin rim of the environment. `false` keeps the mirror reflection of what lies behind the edge there. |
| `settle` | bool | `true` | a frame drawn again at an unchanged scene time steps the frame index, so temporal AA, soft shadows and reflections settle over repeated draws. `false` gives every draw at one time that time's own index, so repeated draws repeat one jitter and one noise pattern, and screen-space reflections keep their roughness-scaled history where nothing moves instead of settling to 0.95 of it. |
| `scene_reflections` | bool | `true` | staging traces the scene's own reflection cube from the camera's eye, so a reflection screen-space reflections can't find reads the scene. `false` skips it, and such a reflection reads the probes and the sky. It has effect only while the renderer takes scene reflections (`Renderer::scene_reflections`, on by default). |
| `dof_spread` | bool | `true` | depth of field treats a pixel at or beyond the camera's far plane as the sky, infinitely far, and gathers at least two rings around any circle of confusion over 2 pixels, so a blurred near edge spreads over the background as the tracer's thin lens shows it. `false` reads such a pixel at the far plane's distance and gathers one ring up to a 6-pixel circle, as before; a plate or background behind a focus pulled close to the camera then blurs less, and small circles soften less. |
| `reversed_depth` | bool | `true` | the projection's precision mode, not the depth buffer's layout: the buffer is reversed (near 1, far and sky 0) either way. `true`: meshes and plates draw with reversed-Z depth, so surfaces a millimetre apart resolve by their true distance. `false` gives them the standard projection's depth, as before reversed-Z, so nearly coplanar layers resolve as they did then, including where they fought. It costs early depth rejection on those passes. Text, glass, impostors and particles keep reversed-Z. |
| `coat_reflections` | bool | `true` | a clear coat reflects by its own split-sum term over an attenuated base, and a coat of 0.5 or more writes its roughness to the reflection target, so its screen-space reflection follows the coat's mirror ray. `false` gives coats their earlier reflection: Schlick's reflectance at exponent 5 and the base's roughness in the reflection target, so the reflection takes the base's footprint and fade. |
| `taa` | bool | `true` | temporal anti-aliasing: the 8-frame subpixel jitter and the history resolve. `false` draws every frame unjittered and hands the lit frame straight to the finish, so edges alias and nothing settles over frames. |
| `ssr` | bool | `true` | screen-space reflections: the scene colour mips, the reflection trace, its prefilter and resolve. `false` skips them (the mips still run while glass is drawn), so every reflection reads the scene's reflection cube, the probes and the sky. |

### `[plates]`

The baked plates of the scene's static part. A bake traces the static objects and the static text into images once; a live renderer composites those images and draws only what is dynamic (`[[object]] dynamic`, `[text.*] dynamic`) over them. A renderer that reads no plates ignores the table.

| key | type | default | notes |
|---|---|---|---|
| `dir` | path | needed | the plate folder a bake writes. It need not exist yet: a missing folder is no error. |
| `proxies` | path | none | a `*.proxies.toml` file: the static objects' invisible stand-ins. |
| `casters` | `"meshes"`, `"proxies"` or `"none"` | `"meshes"` | what casts shadows on the dynamic objects in the static objects' place: each static object's own mesh, the proxies, or nothing. `"proxies"` needs `proxies`. |

### `[physics]`

Play mode's rigid world; renderers ignore it.

| key | type | default | notes |
|---|---|---|---|
| `gravity` | `[x, y, z]` | `[0, -9.81, 0]` | |
| `rate` | number, 1 to 1000 | `60` | the play clock's ticks per second and the rigid world's steps per second. |
| `substeps` | integer, 1 to 16 | `1` | solver steps per step. |
| `iterations` | integer, 1 to 64 | `4` | solver iterations per substep. |

### `[play]`

What the editor's Play runs; renderers ignore it ([play-modules.md](play-modules.md)).

| key | type | default | notes |
|---|---|---|---|
| `module` | path | none | a game's WebAssembly module, `*.wasm`, from the project's root; it need not exist until Play starts. |

### A material library

| key | type | default | notes |
|---|---|---|---|
| `format` | integer | 0 | `1`. |
| `[materials.<name>]` | tables | none | one material each. A name with `/` or `.` is quoted (`[materials."set/blue"]`). |
| `[authoring]` | table | none | the authoring tool's. |

`[materials.<name>]` is one physically based material. Every key has a default, and the values have no enforced range beyond the layer count; the *usual* ranges below say what makes sense.

| key | type | default | usual |
|---|---|---|---|
| `id` | id | none | |
| `family` | a family name | `"plain"` | |
| `base` | `[r, g, b]` | `[0.5, 0.5, 0.5]` | 0 to 1, linear. |
| `roughness` | number | `0.5` | 0 to 1. |
| `metalness` | number | `0` | 0 to 1. |
| `specular` | number | `0.04` | 0 to 1; reflectance at normal incidence. |
| `clearcoat` | number | `0` | 0 to 1. |
| `clearcoat_roughness` | number | `0.05` | 0 to 1. |
| `sheen` | number | `0` | 0 to 1. |
| `transmission` | number | `0` | 0 to 1. |
| `ior` | number | `1.5` | 1 and above. |
| `dispersion` | number | `0` | 0 and above. |
| `thickness` | number | `0` | 0 and above, scene units. |
| `subsurface` | number | `0` | 0 to 1. |
| `subsurface_tint` | `[r, g, b]` | `[1, 1, 1]` | 0 to 1. |
| `absorption` | number | `0` | 0 and above. |
| `thin_film` | number | `0` | 0 and above. |
| `thin_film_ior` | number | `1.5` | 1 and above. |
| `thin_film_amount` | number | `0` | 0 to 1. |
| `emission` | `[r, g, b]` | `[0, 0, 0]` | 0 and above, linear. |
| `fresnel_power` | number | `5` | the exponent of Schlick's `(1 − cos θ)` term on the base and the coat, live and traced; above 5 the reflection rises later toward grazing, so a glossy paint stays dark further off the normal. 0 or below is 5. |
| `normal` | table | `{ source = "flat", strength = 0 }` | `[materials.<name>.normal]`. |
| `maps` | table | `{ layer = -1, tile = 1, normal = 1, albedo = 1 }` | `[materials.<name>.maps]`. |
| `content` | a content kind | `"none"` | |
| `content_layer` | table | below | `[materials.<name>.content_layer]`. |
| `layers` | tables | none | `[[materials.<name>.layers]]`: at most four noise layers. |
| `ageing` | table | every key `0` | `[materials.<name>.ageing]`. |
| `wet` | table | none | `[materials.<name>.wet]`: a wet surface, below. |
| `fabric` | table | none | a cloth sheen layer (glTF's `KHR_materials_sheen`), below. |
| `anisotropy` | table | none | stretched highlights, as on brushed metal or hair (glTF's `KHR_materials_anisotropy`), below. |
| `scatter` | table | none | light wrapping into the surface, a cheap subsurface look for skin and wax, below. |
| `authoring` | any | none | |

The families: `plain`, `paper`, `plaster`, `stone`, `cloth`, `metal`, `occluder`, `petal`, `leaf`, `bark`, `cord`, `wood`, `lacquer`, `glass`, `liquid`, `emissive`, `chrome`. The content kinds: `none`, `ink`, `decal`, `screen`, `photo`, `scroll`, `print`, `field`.

`normal`, when present, needs both keys:

| key | type | notes |
|---|---|---|
| `source` | `"flat"`, `"bump"` or `"map"` | where the shading normal comes from. |
| `strength` | number | |

`maps`, when present, needs all four keys, each a number: `layer` (`-1` for none), `tile`, `normal` and `albedo`.

`content_layer` composites a content slot; each key has its default:

| key | type | default | notes |
|---|---|---|---|
| `slot` | integer | `-1` | the content slot; `-1` is none. |
| `blend` | `"over"`, `"multiply"` or `"emit"` | `"over"` | |
| `ink_roughness` | number | `-1` | `-1` keeps the surface's. |
| `emboss` | number | `0` | |
| `strength` | number | `1` | |

`wet` keys, each optional: `amount` (number, 0 to 1, how wet), `puddles` (number, 0 to 1, how much of the surface stands in puddles), `puddle_scale` (number > 0, the puddles' size in scene units) and `seed` (unsigned integer).

`fabric`, `anisotropy` and `scatter` are tables, written inline (`fabric = { color = [0.9, 0.8, 0.7], roughness = 0.6 }`) or as `[materials.<name>.fabric]`; every key in them is optional:

| table | key | type | notes |
|---|---|---|---|
| `fabric` | `color` | `[r, g, b]` | 0 to 1; the sheen's colour. |
| `fabric` | `roughness` | number | 0 to 1; the sheen's roughness. |
| `anisotropy` | `strength` | number | 0 to 1; how stretched the highlight is. |
| `anisotropy` | `rotation` | number | finite, radians; turns the stretch direction around the normal. |
| `scatter` | `color` | `[r, g, b]` | 0 to 1; the colour the light takes as it wraps. |
| `scatter` | `radius` | number | above 0, scene units; how far the light wraps. |
| `scatter` | `amount` | number | 0 to 1; how much of the light wraps. |

`ageing`, when present, needs all eight keys: `fade`, `yellow`, `ink`, `scratch`, `edge`, `dust`, `patina` (numbers, usually 0 to 1) and `seed` (unsigned integer).

`[[materials.<name>.layers]]` is one noise layer; a material takes at most four. A layer whose amplitude is 0 does nothing.

| key | type | default | notes |
|---|---|---|---|
| `kind` | a noise kind | needed | |
| `frequency` | number | needed | |
| `amplitude` | number | needed | |
| `seed` | unsigned integer | needed | |
| `params` | twelve numbers | twelve zeros | their meaning is the kind's. |

| kind | params, in order |
|---|---|
| `none` | an empty slot. |
| `fibre` | `across`, `along` (the fibre streaks' frequencies across and along the surface's y), `fine`, `mottle`, `ripple`, `swell` (bump frequencies), `fibre_gain`, `mottle_gain`, `ripple_gain`, `swell_gain` |
| `crinkle` | two frequencies, then two gains, of two bump octaves |
| `plank_wood` | `width` of a plank in metres (planks run along x and stack in z), `rings` per metre, `warp` of the rings in metres, two `figure` frequencies along and across the grain, `tone` spread between planks, `figure_depth`, `seam_floor` (the seam's darkest factor), `roughness` gain from the figure, `jitter` of the rings |
| `wall_mottle` | two broad and two fine albedo octaves, two swell bump octaves, `broad_gain`, `fine_gain`, two swell gains |
| `grime` | two frequencies, the `dirty` and `clean` tints, two roughness factors for dirty and clean |
| `leaf`, `bark` | none: fixed patterns with a height relief |
| `flow` | none: `frequency` scales the uv; a warped 2D noise that moves with time and shifts the thin film |
| `value`, `fbm` | none: `frequency` scales the position |
| `scratch` | `line` frequency across the scratches (stretched by 0.02 along them), two `smudge` values, `speck` |
| `coat_wobble` | three frequencies (fine, then two broad), two weights of the broad octaves relative to the fine one, whose weight is the layer's amplitude |

Unset params are 0, and a kind with no params draws nothing visible: `plank_wood` and `coat_wobble` need a positive first param to draw at all.

Every kind draws the same live, traced and baked by `bake_layers`: the same hash, octaves, scale and seed. Two exceptions: `coat_wobble` live reads a tiled noise of the same scales and strength, not the same pattern, so a small patch can shade a little differently live than traced or baked; and `flow` follows the scene's time live and traced while a bake holds it at time 0 and its baked maps carry no thin film.

### The proxies file

| key | type | default | notes |
|---|---|---|---|
| `format` | integer | 0 | `1`. |
| `[[proxy]]` | tables | none | one stand-in each. |
| `[authoring]` | table | none | the authoring tool's. |

A proxy is never drawn. It stands in for one static object: it casts that object's shadow with `casters = "proxies"`, and a ray that meets it picks that object. Its coordinates are world space.

`[[proxy]]`:

| key | type | default | notes |
|---|---|---|---|
| `id` | id | none | |
| `object` | reference | needed | a static object with a mesh, of the scene whose `[plates]` names this file (References). A dynamic object or a placement is refused (`bad-reference`). |
| `kind` | `"box"` or `"hull"` | needed | |
| `at` | `[x, y, z]` | `[0, 0, 0]` | box only: the box's centre. |
| `rotate` | `[x, y, z]` | `[0, 0, 0]` | box only: degrees about X, then Y, then Z. |
| `size` | `[x, y, z]`, each > 0 | box: needed | box only: the box is a unit cube centred on the origin, scaled by `size`, turned by `rotate` and moved to `at` (`T·R·S`). |
| `points` | array of `[x, y, z]`, at least four | hull: needed | hull only: the vertices. |
| `triangles` | array of `[a, b, c]`, at least four | hull: needed | hull only: the faces, each corner a place in `points` counting from 0, counter-clockwise seen from outside. |
| `authoring` | any | none | |

A box takes no `points` or `triangles`, and a hull no `at`, `rotate` or `size` (`bad-value`). The checker refuses a corner past the last point; keeping the winding counter-clockwise is the writer's part.

## Migration

`format` counts whole versions; a file never mixes two. Reading an older file migrates it in memory and warns once per file (`old-format`); diagnostics on it point into the file as written. The migration also rewrites a file on disk through the same code, keeping comments, spacing and order. Format 0 to 1:

1. Adds `format = 1` on the first line, followed by a blank line.
2. Rewrites each path from relative to its file to relative to the project root. A path that would leave the root is refused, as any such path is (`outside-root`). A lone scene with no `project.toml` has its own folder as its root, so a format 0 path with `..` past that folder can't migrate as it stands: the migration names each such path and suggests a `project.toml` at the deepest folder that holds every file the scene reads.
3. Renames `[[object]] id` to `pick`.
4. Moves each `[body.<object>]` into its object, in the same file, as `[object.body]`. A body whose object is in another file can't move (`migration`); move it by hand.
5. Writes an `id` into every entry that can be referenced. References stay names; they still resolve, and an editor writes ids as it touches them.

A migrated id is derived: `Id::derive` of `"<project-relative path>\n<kind>\n<name>"`, where the kind is the entry's table key (`mesh`, `object`, `light`, `emitter`, `mover`, `content`, `text`, `sound`, `rig`, `tiles`, `materials` or `proxy`) and the name is its `name` or table key (a proxy, which has no name, uses `"<object>\n<place>"`). If that id is taken, `"\n1"`, `"\n2"`, … is appended to the key until it is free. A written id goes after the entry's existing keys.

A newer `format` than the reader knows is refused, naming the version it knows (`newer-format`).

## Writing

- Every write goes through `pfx-scene`'s writer, on `toml_edit`: comments, blank lines, key order and number spelling survive, and a changed value keeps its place. Of an array, only the components that differ are replaced (`at = [ -0.9,   0.350, 0.0 ]  # by the wall` becomes `at = [ -0.9,   0.350, 1.5 ]  # by the wall`). A value that is already equal writes nothing.
- New keys go after the entry's existing ones; a new entry's keys go in the order the tables above list them, and a new table goes after the last of its kind. A file that can't be written back byte for byte (CRLF line ends) is refused when it is opened.
- An edit is a patch on the document: set, unset, push or remove at a target and a key path. A target is an entry by id or unique name, a section (`[sun]`, `[sky]`, `[haze]`, `[camera]`, `[finish]`, `[flare]`, `[motion_blur]`, `[trace]`, `[plates]`, `[physics]`) or a file's top level. A key path is the keys into it, where a number picks an entry of a list, counting from 0 (`["layers", "1", "frequency"]`); a missing table on the way is created. A list of tables lands as `[[…]]` tables, any other table as an inline one.
- An entry is edited in the file that holds it, a material in its library; a section where it is, or in the root scene file when no file holds it yet. A new entry goes into the root scene file (a material into the first library) unless a file of the scene is given.
- Every edit is checked before it is written: the edited texts are checked in memory as `check` checks them, and a refused edit writes nothing. An edit is also refused when a file changed on disk since it was read. Files are written by a temporary file and a rename.
- Editors group patches into undo steps. Each edit returns a patch group: a label and one patch per file it touched, with that file's exact text before and after, so undo and redo bring the bytes back exactly. A group can span many edits (a drag), keeping the first text before and the last text after per file; a group whose net text is unchanged is no step. Patches and groups are plain values with their inverse, so an editor with its own history can apply them itself.
- Ids are written only when an entry is made, or when a file is fixed or migrated; the first write of a file through the writer fills its missing ids.

The writer is `SceneEdit`, opened on a scene `.pitofx` (or a scene or prefab file of format 1 in a loose project):

```rust
let mut edit = SceneEdit::open("scenes/room.pitofx")?;
edit.set_at("crate", [-0.9, 0.35, 1.5])?;
edit.set(&Target::Light("7k2m9q4xzr".into()), &["intensity"], 9.5f32)?;
edit.add(Kind::Object, Id::from_bits(random), "ball", &[("mesh", "block".into())], None)?;
edit.undo()?;
```

- `set`, `unset` and `push` take a `Target` (`Scene`, `Object`, `Mesh`, `Node { mesh, node }`, `Material`, `Light`, `Emitter`, `Mover`, `Content`, `Text`, `Sound`, `Rig`, `Tiles`, `Particles`, `Sun`, `Sky`, `Haze`, `Camera`, `Finish`, `Flare`, `MotionBlur`, `Trace`, `Plates`, `Physics`) and a key path; `remove` takes a target. An entry placed by a prefab is edited through its placement's `[object.set]`. An object's character, trigger, animation and body are edited through its key paths (`["character", "jump_speed"]`, `["animation", "clip"]`), and a tile layer's cells through `rows` or `cells` (a painted row is its whole new string).
- `add(kind, id, name, fields, file)`, `add_material(name, id, material, file)` and `duplicate_object(name, new_name, id)` make entries with the id the caller gives; the id must be free in the project. `rename_object` follows the name references to the object (`parent`, a mover's `objects`, a sound's `object`, a rig's `target`); references by id need nothing. Typed helpers cover an object's keys (`set_at`, `set_rotate`, `set_scale`, `set_object_material`, `set_object_materials`, `set_shadow`, `set_two_sided`, `set_hidden`, `set_clip`, `set_parent`), a mesh (`add_mesh`, `set_node`) and a material (`set_material`).
- `begin(label)` … `end()` groups edits into one undo step and `cancel()` takes it back; `undo()`, `redo()` and `apply(&PatchGroup)` move between texts. A refused edit returns an `EditError` with the file, the line, the edited key, the diagnostic's code and its message. When a check refuses it, `diagnostics` holds what `Project::scene` returns for it, every error with its column and `related`; a refusal of the writer's own (a file changed on disk, a key that is not there) leaves it empty.
- `SceneEdit::open`, an edit and a dry run read the scene's includes without recursing, so a scene that includes itself, or nests its includes more than 64 deep, is refused with the `include-cycle` or `include-depth` finding and its chain, as `Project::scene` gives it, on any stack.
- `dry_run(|edit| …)` runs edits on the texts in memory and returns their merged group, labelled by its first edit, without writing a file or touching undo or `scene()`. Each edit sees the ones before it and the ids they made; the group is checked as a whole at the end, so a step may pass through a state that alone would be refused (an object naming a material the next edit adds). `apply(&group)` then writes it. Inside a dry run, `begin`, `undo`, `redo`, `apply`, `reload` and another dry run are refused.
- `PatchGroup::after()` maps each file to its text after the group: the overlay for `Project::scene_with` and `check_with`, so an editor runs its own checks (asset contents) on an edit before it writes. `PatchGroup::write()` writes a group without an editor, refused when a file no longer holds the text the group starts from; a failed write puts back the files already written.
- `Project::fix(file)` fills the missing ids of every file a scene or prefab reads (the file, its includes, its libraries, the prefabs it places and its proxies file), derived as the migration derives them, with every id of the project taken. It returns the patch group, checked as a whole, and writes nothing. A scene that does not load, a format 0 file (migrate it, which writes its ids) and a file that can't be written back byte for byte are refused. A scene that does not load is refused with the errors `Project::scene` gives in `EditError::diagnostics`: a cycle or a nesting too deep with its chain in the message and its includes or placements in `related`, as `check` reports them.

## Checking

`pfx-scene` checks a buffer or a whole project, and resolves a scene:

```rust
pub fn check(source: &str, file: &Path, project: &Project) -> Vec<Diagnostic>;

impl Project {
    pub fn open(root: &Path) -> Result<Project, Diagnostic>;
    pub fn check(&self) -> Vec<Diagnostic>;
    pub fn scene(&self, file: &Path) -> Result<Scene, Vec<Diagnostic>>;
    pub fn scene_with(&self, file: &Path, overlay: BTreeMap<PathBuf, String>) -> Result<Scene, Vec<Diagnostic>>;
    pub fn check_with(&self, overlay: BTreeMap<PathBuf, String>) -> Vec<Diagnostic>;
    pub fn fix(&self, file: &Path) -> Result<PatchGroup, EditError>;
    pub fn read(&self, file: &Path) -> Result<File, Vec<Diagnostic>>;
    pub fn migrate(&self, file: &Path) -> Result<String, Vec<Diagnostic>>;
    pub fn root_of(file: &Path) -> PathBuf;
}

pub struct EditError {
    pub file: PathBuf,
    pub key: String,
    pub line: Option<u32>,
    pub code: &'static str,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn migrate(text: &str, file: &Path, root: &Path, kind: FileKind) -> Result<String, Vec<Diagnostic>>;

pub const MAX_DEPTH: usize = 64;

pub struct Diagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub severity: Severity,
    pub code: &'static str,
    pub key: String,
    pub message: String,
    pub related: Vec<Location>,
}

pub struct Location {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}
```

- `Project::root_of` finds a file's project root. `Project::open` finds the files under the root by their names. It skips the root's `target` and `tmp` folders (build output and scratch), the paths `[project] ignore` lists, folders and files whose name starts with a dot, and folders that hold their own `project.toml` (another project). A skipped `target` or `tmp` is the root's alone: `levels/tmp` is read. A file a scene names inside a skipped folder is still read and checked with that scene; it is only not found on its own.
- `Project::check` checks every file `Project::open` found; each top scene (a scene no other scene includes, or one caught in an include cycle) with everything it reads; each prefab on its own; and each proxies file against the scenes whose `[plates]` name it, or against the project's objects when no scene does. It resolves every include, prefab, reference and id, and checks that each asset path exists with that case.
- `scene_with` and `check_with` resolve and check as `scene` and `check` do, with the overlay's texts in place of the files they name. Its paths are absolute or relative to the root; a file only the overlay holds counts as existing, so an unsaved new include resolves.
- `check` runs on an editor's unsaved buffer: it checks that one text in every context it takes part in, against the project for references, and returns the diagnostics located in that file, so a TOML editor marks errors as it types.
- `Project::read` returns one file as its typed data (`File::Scene`, `File::Materials`, …), migrated in memory when it is older. `Project::migrate` and `migrate` return a file's text migrated to format 1, or the diagnostics of the steps that can't run; a format 1 text comes back as it is.
- `Project::scene` returns the resolved scene, with its warnings: includes merged, prefabs placed with namespaced ids, every reference rewritten to the key of the entry it names, and paths relative to the root. A top-level entry's key is its name; a placed entry's key is `"<placement key>/<inner key>"`. Every object's `dynamic` is set, derived where the file leaves it out. `Scene::rigs` holds the rigs, each `target` rewritten to its object's key like any reference. `Scene::proxies` holds the proxies of `[plates] proxies`, each keyed by its 0-based place in its file, and `Scene::files` lists every file the scene read, the proxies file and the tile layers' prefabs among them. `Scene::tiles` holds the tile layers, keyed by name, each palette value rewritten to its prefab's root-relative path (a placement's id to that placement's prefab); a loader resolves each of those prefabs once (`Project::scene` on its path) and places it at every cell that names it. `Tiles::cells` lists a layer's filled cells as `([i, j], tile)`, from `rows` or `cells`, and `Tiles::position` gives a cell's position. `Scene::layers` holds the project's `[layers]`, empty without them, so a loader builds its physics from the scene alone.
- A layer name is checked against `project.toml` as the overlay or the disk holds it. When `[layers]` can't be read (the file doesn't parse, or the table has the wrong shape), the layer names of scenes and prefabs go unchecked and `Scene::layers` is empty; `Project::check` reports the project file's own error.
- `file` is relative to the project root. Lines and columns are 1-based, columns count characters, and `end_line` and `end_column` point just past the span.
- `related` holds the other places a diagnostic names: the first use of a duplicate id, each entry an ambiguous name matches, the other file of a table held twice, and for an include or prefab cycle or a chain nested too deep, the `include` or `prefab` key of each file on the chain, outermost first, ending with the diagnostic's own place.
- Resolving keeps the chain of files it is in on the heap, never on the call stack, so the stack it needs stays the same however deep includes and prefabs nest: the tests resolve includes and placements 64 deep, and refuse chains 1000 deep, on a 2 MB thread in a debug build.
- `key` is the dotted path to the value, with a list entry named by its id when it has one and by its 0-based place otherwise: `object.7k2m9q4xzr.at`, `light.0.intensity`.
- `severity` is `Error` or `Warning`. The warnings are `missing-id` and `old-format`; everything else is an error.
- `code` is a stable short name that editors may key help on:

| code | when |
|---|---|
| `syntax` | the text is not valid TOML. |
| `unreadable` | a file can't be read. |
| `not-utf8` | a file is not UTF-8. |
| `unknown-key` | a table holds a key it doesn't take; the message lists the keys it takes. |
| `missing-key` | a needed key is absent, such as an object's `mesh`, a light's `position` or a box proxy's `size`. |
| `bad-type` | a value has the wrong type or length, such as text for a number, two numbers for three, four numbers for a `scale` of a number or three, or a tunable's value of another type than its `type`. |
| `bad-value` | a value is outside its range or its list, or a combination the table refuses (`fov` with `focal`, a slide with a pivot, an object moved by two movers, a body or a character under an ordinary object or a moved placement, a character with a body, a 3d character with `wall_jump`, a follow rig with `pitch`, a sprite with `blend`, an event at a frame past its clip, a tile layer with `rows` and `cells`, a cell set twice, a box proxy with `points`, `casters = "proxies"` without `proxies`, a tunable's `default` outside its `min` and `max`, a layer listed on one side only). |
| `bad-id` | an `id` is not 10 lowercase Crockford base32 characters. |
| `missing-id` | an entry that can be referred to has no `id` (a warning). |
| `duplicate-id` | an id is used twice in the project. |
| `bad-name` | a name is empty, has a space at an end, or (a material's) holds a control character; a palette key is `.`, or holds a space. |
| `duplicate-name` | a name is used twice in its kind across a scene and its includes, or a material is in two libraries. |
| `bad-reference` | a reference matches no entry of its kind, or an entry that can't serve (a hit sound's object without a body, a proxy's dynamic object or placement, a palette or `prefab` id that is no placement), or a layer name names no layer of `[layers]`, a sprite's clip no clip of its `clips`, or a tile no key of its layer's palette. |
| `ambiguous-reference` | a reference names a name two entries carry. |
| `bad-path` | a path is empty, absolute, or holds a backslash. |
| `outside-root` | a path leaves the project root. |
| `missing-file` | a named file does not exist (a `[plates] dir` and the paths of `[project] ignore` may not exist yet), or the scene or prefab asked for is not found (at line 1, column 1 of that file). |
| `path-case` | a named file exists only with another case. |
| `include-cycle` | a scene or prefab includes itself; `related` lists each include of the cycle. |
| `prefab-cycle` | a prefab places itself; `related` lists each placement of the cycle. |
| `include-depth` | includes nest more than 64 deep; `related` lists each include of the chain. |
| `prefab-depth` | prefab placements nest more than 64 deep; `related` lists each placement of the chain. |
| `parent-cycle` | objects' parents form a cycle. |
| `held-twice` | `fallback` or a one-file table is held by two files of a scene. |
| `bad-override` | an `[object.set]` key names no entry or a key that entry doesn't take, or its value doesn't fit (a layer name the project doesn't name among them), or it puts a body or a character under an ordinary object or a moved placement, or it breaks a relation (Prefabs). |
| `placement-key` | a placement holds a key only an object with a mesh takes (`body`, `character`, `trigger` and `animation` among them). |
| `old-format` | a format 0 file, read through the migration (a warning). |
| `newer-format` | a file's `format` is newer than the reader knows. |
| `migration` | a migration step can't run, such as a `[body.<object>]` whose object is in another file. |

## JSON Schema

`docs/schema/` holds a JSON Schema (draft 2020-12) for each kind of file: `project.schema.json`, `scene.schema.json`, `prefab.schema.json`, `materials.schema.json`, `proxies.schema.json` and `finish.schema.json`. They are generated from the crate's types, so they carry every table and key with its type, each enumerated value and an id's form, and refuse unknown keys as the reader does; ranges, references and paths stay the checker's. Each tag of the crate carries the schemas of its version. With the crate's `schema` feature, `json_schema(kind)` gives one as text, and `cargo run -p pfx-scene --features schema --example scene-schema -- <folder>` writes all six.

## Scenes in a .pitofx file

A scene and every file it reads live in one `.pitofx` file, the form pfx gives a user (`pfx scene migrate <scene.toml>` imports a loose scene into one once) (the container is the `pfx-file` crate; [pfx-file.md](pfx-file.md) is the byte layout). The TOML files stay exactly as they are on disk, as chunks inside it:

- `SMET`, unnamed and stored, is a small TOML table: `format` (the scene format, 1), `entry` (the scene file the `.pitofx` opens to), and `recipe` and `scene` when a tool built the scene from a recipe (the SHA-256 of the recipe and of the scene chunks as built, so that tool sees an edit made elsewhere). The editor never changes `recipe` or `scene`.
- `SCNE`, one per TOML file of the scene (the root scene, its includes, its material libraries, the prefabs it places and their files), named by its path from the project root, exactly as the scene's own paths name it.
- `FILE`, one per other file the scene names (meshes, images, fonts, sounds, LUTs), named the same way. Images and other packed data are stored as they are, so a reader borrows them without a copy.
- Other chunks (`GLTF`, `GBIN`, `THMB`, a recipe and its facts) are kept as they are.

A `.pitofx` scene lays its files over the project: inside it, a path resolves to the `.pitofx`'s own `SCNE` or `FILE` chunk of that name first and then to the file on disk, still inside the project root. A library prefab or a shared mesh outside the scene stays a file on disk. `mesh.<name>.file = "@gltf"` names the glTF in the `.pitofx`'s own `GLTF` and `GBIN` chunks, with `node` naming its node; outside a `.pitofx` that holds one it is `missing-file`. The project root is found as for any file: the nearest folder above the `.pitofx` with a `project.toml`, or the `.pitofx`'s own folder.

- `Project::open_pfx(path)` opens one; `Project::scene`, `check`, `read` and `fix` then work on its files, and `Project::open` of a folder also checks every `.pitofx` in it. Diagnostics on a file inside it name it as `<the .pitofx>/<its name>` (`scenes/room.pitofx/scenes/room.scene.toml:12:5`). A `.pitofx` that is cut or corrupt, holds no `SMET`, names an entry it doesn't hold, a chunk name that is not a path inside the project, or a newer scene format is refused (`unreadable`).
- `Project::asset(path)` gives a file's bytes from the `.pitofx` or the disk, refusing a path that leaves the root; `Project::pfx()` gives the container's reader, for the `GLTF` and `GBIN` chunks; `Project::path_of(file)` and `shown(file)` give the path an editor shows for a file.
- `SceneEdit::open` takes the `.pitofx` (or `<the .pitofx>/<file>`) and edits its TOML in place: every edit, undo, redo and dry run works as on disk, and a write rewrites the `.pitofx` once, through a temporary file and a rename, keeping every chunk it doesn't change byte for byte and in its place (a new chunk goes at the end). `PatchGroup::write` does the same for a group whose files are inside a `.pitofx`.
- `pack(project, scene)` gives the `.pitofx` bytes of a scene of TOML files: its `SMET`, then each TOML file it reads (its entry first, the rest in path order), then each other file it names (in path order), compressed with the container's default. The same files give the same bytes. `unpack(bytes, folder)` writes the `SCNE` and `FILE` chunks back out as files under the folder, refusing to write through a link; unpacking and packing again gives the same bytes, and unpacking a packed scene gives its files exactly.

## The crate

`pfx-scene` is licensed with pfx (AGPL-3.0-only, or a commercial licence: [Licence](../README.md#licence)). It depends on `toml`, `toml_edit`, `serde` and `serde_json`, and on `pfx-file` (`crates/file`) for `.pitofx` scenes. The optional `schema` feature adds `schemars` for `json_schema`. Vectors are plain arrays (`[f32; 3]`), never a maths crate's types. Its semver is its own; the `format` number changes only with a migration.
