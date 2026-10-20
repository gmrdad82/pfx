# Import a TOML file

Every file names its format version on its first line. A file without `format` is format 0. Reading it migrates it in memory and warns once (`old-format`), with every finding pointing into the file as written, so an older project keeps loading. A newer format than the crate knows is refused, naming the version it knows (`newer-format`).

## Import

A scene is a `.pitofx` ([pfx-file.md](pfx-file.md#the-user-file)). `pfx scene migrate` imports a plain `.scene.toml` once into a `.pitofx` beside it, with every file it reads, and migrates its format 0 files to format 1 inside it; the `.toml` is left as it was and never written again:

```sh
pfx scene migrate scenes/room.scene.toml
```

```text
imported scenes/room.scene.toml into scenes/room.pitofx
```

- `pfx scene migrate <folder>` imports every `*.scene.toml` and recipe in the folder and below it (skipping `target/`, `tmp/` and hidden folders), each into its own `.pitofx`; one imported before is kept and said so.
- `pfx scene migrate <scene.pitofx>` migrates the format 0 files already inside a `.pitofx`, writing the `.pitofx` once.
- `--dry-run` says what it would import or migrate and writes nothing.
- A `.pitofx` that already exists beside the file is refused: the import happens once, and edits go to the `.pitofx`.
- A recipe (`recipe = 1`) is imported the same way, with its includes and files; `--bare` keeps only the recipe and the files it reads, and the build comes the first time a command needs it.

Every other command given a `.toml` refuses with one line naming the import: `pfx: room.scene.toml is a plain TOML file; import it once with `pfx scene migrate room.scene.toml`, which writes room.pitofx beside it`.

In this crate's own tree, the `scene-write` example's `migrate` rewrites a loose file as format 1 in place (`scene-write migrate scenes/room.scene.toml`). On a format 0 copy of the demo's scene, the diff starts:

```diff
@@ -1,5 +1,8 @@
-materials = ["../materials/room.materials.toml"]
+format = 1
+
+materials = ["materials/room.materials.toml"]
 
 [mesh.block]
-file = "../assets/block.gltf"
+file = "assets/block.gltf"
+id = "eqazbk5y03"
 
@@ -11,4 +14,5 @@
 scale = [8.0, 0.1, 8.0]
+id = "ym5syy6wws"
 
-[body.floor]
+[object.body]
 kind = "fixed"
```

From format 0 to 1, the migration:

1. adds `format = 1` on the first line;
2. rewrites each path from relative to its file to relative to the project root;
3. renames `[[object]] id` to `pick`;
4. moves each `[body.<object>]` into its object as `[object.body]`;
5. writes a derived `id` into every entry that can be referenced.

Comments, spacing and key order are kept, as for every edit.

## From code

`Project::migrate` returns the file's text migrated to format 1, or the diagnostics of the steps that can't run; you choose where to write it. A format 1 file comes back as it is.

```rust
use std::path::Path;

use pfx_scene::Project;

fn main() {
    let project = Project::open(Path::new("my-game")).unwrap();
    match project.migrate(Path::new("scenes/old.scene.toml")) {
        Ok(text) => std::fs::write("my-game/scenes/old.scene.toml", text).unwrap(),
        Err(findings) => {
            for finding in findings {
                eprintln!("{finding}");
            }
        }
    }
}
```

`pfx_scene::migrate(text, file, root, kind)` does the same on a text you hold. A path that would leave the project root can't migrate, and a body whose object is in another file has to be moved by hand; each is reported at its key. A lone scene with no `project.toml` has its own folder as its root, so give it a `project.toml` at the folder that holds every file it reads before migrating it.
