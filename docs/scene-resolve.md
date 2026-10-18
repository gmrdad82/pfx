# Resolve a scene

`Project::scene` resolves a scene into a `Scene`: includes merged, prefabs placed as objects with their per-placement overrides, and every reference rewritten to the key of the entry it names. It is what an engine's loader reads. It returns the scene with its warnings, or the errors that stop it from loading.

## From the command line

The `scene-tree` example prints a resolved scene. On the demo project:

```sh
cargo run --release -p pfx-scene --example scene-tree -- crates/scene/examples/demo
```

```text
scenes/room.scene.toml resolved from 3 files
  floor            oak  at [0, -0.05, 0]  scale [8, 0.1, 8]  fixed body
                   id h3gw9t9qzy
  lamp             brass  at [0, 2.4, 0]  scale 0.3
                   id j66hkn5wx4
  left crate       places prefabs/crate.prefab.toml  at [-1, 0.5, 0]
                   id effsbdb6jg
  left crate/box   oak  at [0, 0, 0]  dynamic body  (prefabs/crate.prefab.toml)
                   id effsbdb6jg/6dek3mm9mh
  right crate      places prefabs/crate.prefab.toml  at [1.2, 0.5, 0.3]  turned [0, 30, 0]
                   id axxz3n1hcj
  right crate/box  oak  at [0, 0, 0]  dynamic body, density 0.5  (prefabs/crate.prefab.toml)
                   id axxz3n1hcj/6dek3mm9mh
  bulb             light at [0, 2.2, 0], intensity 40
```

A second argument names another scene file of the project.

## Keys and ids of placed entries

The crate prefab holds one object, `box`, with id `6dek3mm9mh`. Each placement makes its own copy, keyed and identified by the placement: `left crate/box` with id `effsbdb6jg/6dek3mm9mh`, and `right crate/box` with id `axxz3n1hcj/6dek3mm9mh`. Two placements of one prefab never collide. The right crate's `[object.set]` gave its box a density of 0.5; the left crate's box keeps the prefab's own.

A placed entry's position is its own, relative to its placement: the `at` above is the box's place in the prefab. The `parent` links say which placement holds it, and the loader composes the transforms.

## From code

```rust
use std::path::Path;

use pfx_scene::Project;

fn main() {
    let project = Project::open(Path::new("crates/scene/examples/demo")).unwrap();
    let scene = match project.scene(Path::new("scenes/room.scene.toml")) {
        Ok(scene) => scene,
        Err(errors) => {
            for error in errors {
                eprintln!("{error}");
            }
            return;
        }
    };
    for object in &scene.objects {
        println!("{} {:?} at {:?}", object.key, object.id, object.value.at);
    }
    for warning in &scene.warnings {
        println!("{warning}");
    }
}
```

Each list of the scene (`objects`, `meshes`, `materials`, `lights`, `sounds`, `rigs`, `tiles` and the rest) holds `Entry` values: the resolved `key`, the `id`, the `file` it came from, and the typed `value`. `Scene::object(key)`, `Scene::mesh(key)` and their like find one by key. The scene-wide sections (`sun`, `sky`, `haze`, `camera`, `finish`, `trace`, `plates`, `physics`) are `Option`s, `layers` holds the project's collision layers, and `files` lists every file the scene read.

To read one file as typed data without resolving it, `Project::read` returns a `File` (`File::Project`, `File::Scene`, `File::Prefab`, `File::Materials`, `File::Proxies` or `File::Finish`), plain serde structs, migrated in memory when the file is older.
