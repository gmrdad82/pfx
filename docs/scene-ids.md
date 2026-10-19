# Fill missing ids

A hand-written entry may leave `id` out. The checker warns about it as `missing-id`, the scene still loads, and references to it go by name. `Project::fix` fills the missing ids of every file a scene or prefab reads: the file, its includes, its libraries, the prefabs it places and its proxies file. On the command line, `pfx scene fix <scene.pitofx>` writes them inside a scene's `.pitofx`; from code, the path names the scene entry in the project (`scenes/room.scene.toml`).

```rust
use std::path::Path;

use pfx_scene::Project;

fn main() {
    let project = Project::open(Path::new("my-game")).unwrap();
    let group = project.fix(Path::new("scenes/room.scene.toml")).unwrap();
    if group.is_empty() {
        println!("every entry has its id");
        return;
    }
    for file in group.files() {
        println!("fills ids in {}", file.display());
    }
    group.write().unwrap();
}
```

`fix` writes nothing itself: it returns the patch group, checked as a whole, and `write()` writes it, refused when a file no longer holds the text the group starts from. A scene that does not load is refused with its errors in `EditError::diagnostics`, and a format 0 file is refused too: [migrate it](scene-migrate.md), which writes its ids. An editor that opens a scene through `SceneEdit` fills them on its first write of each file.

## How a filled id is made

A filled id is derived, not random: `Id::derive` of the file's project-relative path, the entry's kind and its name, the first 50 bits of their SHA-256. Running `fix` twice on the same project gives the same ids, and an id already taken anywhere in the project is skipped. The crate reads no clock and no system randomness.

## Ids for new entries

When your tool adds an entry, give it an id: random for an entry a person adds, or derived from a stable key your tool owns, so a build gives the same bytes for the same input.

```rust
use pfx_scene::Id;

let random_bits: u64 = 0x1234_5678_9abc;
let made = Id::from_bits(random_bits);
let derived = Id::derive(b"levels/one.scene.toml\nobject\ndoor");
```

`Id::from_bits` keeps the low 50 bits. An id is 10 lowercase Crockford base32 characters; once written it never changes, so renaming or moving an entry keeps its references.
