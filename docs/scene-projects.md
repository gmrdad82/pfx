# Lay out a project

A project is a folder with a `project.toml`. Every scene file in it is TOML that starts with `format = 1`, and its kind comes from its name; in pfx a user's scenes ship packed with their prefabs and libraries in a `.pitofx` each ([Scenes in a .pitofx file](scene-format.md#scenes-in-a-pfx-file)), and the loose layout below is what is inside one, or the crate's own demo. This is the demo project in `crates/scene/examples/demo`:

```text
crates/scene/examples/demo
├── project.toml
├── assets
│   ├── ball.gltf
│   └── block.gltf
├── materials
│   └── room.materials.toml
├── prefabs
│   └── crate.prefab.toml
└── scenes
    └── room.scene.toml
```

| File | Holds |
|---|---|
| `project.toml` | the root's marker: the project's name, the scene an editor opens first, folders to ignore, the game's tunables and the collision layers |
| `*.scene.toml` | a scene entry: includes, material libraries, meshes, objects, lights, sounds, camera rigs, tile layers, sun, sky, camera and render settings |
| `*.prefab.toml` | a reusable piece: the entries a scene has, without the scene-wide sections |
| `*.materials.toml` | a material library |
| `*.proxies.toml` | a plate's invisible stand-ins |

The demo's `project.toml` is four lines:

```toml
format = 1

[project]
name = "demo"
scene = "scenes/room.scene.toml"
```

## Paths

A relative path resolves from the folder of the file that names it first, then from the project root, and uses `/` ([Scenes as data](scenes.md#loading) has the rule). A path written relative to the root, as the demo's are, keeps resolving through the second step. An absolute path, a backslash or a `..` that leaves the root is refused. A path keeps its case: one the checker finds only with another case is refused as `path-case`, so a project reads the same on Linux, Windows and macOS. Assets are standard files referenced by path, never embedded: glTF 2.0, PNG, WAV or Ogg Vorbis, TTF or OTF, Radiance HDR.

## Ids and names

Every entry something can refer to carries an `id`: 10 lowercase Crockford base32 characters, such as `id = "j66hkn5wx4"`. An id is written once, when the entry is made, and never changes, so a rename or a move breaks nothing. Names are labels for people: `name` on a list entry and the key of a keyed table (`[mesh.ball]`).

References go by id or by a unique name. In the demo's scene, the lamp names its mesh and its material by name:

```toml
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
```

A hand-written entry may leave `id` out: the checker warns (`missing-id`), and [Fill missing ids](scene-ids.md) writes them.

## Prefabs and includes

A prefab is placed by an object that names its file, with per-placement overrides in `[object.set]`:

```toml
[[object]]
id = "axxz3n1hcj"
name = "right crate"
prefab = "prefabs/crate.prefab.toml"
at = [1.2, 0.5, 0.3]
rotate = [0.0, 30.0, 0.0]

[object.set]
"box.body.density" = 0.5
```

A scene's `include` list merges other scene files into it. Includes and prefabs nest up to 64 deep, and a cycle is refused with its chain of files. [Resolve a scene](scene-resolve.md) shows what a resolved placement looks like.

Every key of every table, its type, default and range is in [the format](scene-format.md).
