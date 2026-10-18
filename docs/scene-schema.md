# Validate in a TOML editor

Each tag of pfx carries a JSON Schema (draft 2020-12) for every kind of file in `docs/schema/`, generated from the crate's types:

| File | Schema |
|---|---|
| `project.toml` | `project.schema.json` |
| `*.scene.toml`, `scene.toml` | `scene.schema.json` |
| `*.prefab.toml` | `prefab.schema.json` |
| `*.materials.toml` | `materials.schema.json` |
| `*.proxies.toml` | `proxies.schema.json` |
| a finish file | `finish.schema.json` |

A schema knows every table and key with its type, each enumerated value (a light's `kind`, a finish's `style`) and an id's form, and refuses a key the reader doesn't take. Ranges, references, paths and ids that clash are the checker's: run `Project::check` ([check a project](scene-check.md)) or [check the unsaved buffer](scene-check-buffer.md) for those.

## In an editor that reads Taplo's settings

Taplo, and the editors built on it (Even Better TOML in VS Code, Zed, Helix and others), take a `.taplo.toml` at the project root:

```toml
[[rule]]
include = ["**/*.scene.toml", "**/scene.toml"]
schema.path = "https://raw.githubusercontent.com/gmrdad82/pfx/<tag>/docs/schema/scene.schema.json"

[[rule]]
include = ["**/*.prefab.toml"]
schema.path = "https://raw.githubusercontent.com/gmrdad82/pfx/<tag>/docs/schema/prefab.schema.json"

[[rule]]
include = ["**/*.materials.toml"]
schema.path = "https://raw.githubusercontent.com/gmrdad82/pfx/<tag>/docs/schema/materials.schema.json"

[[rule]]
include = ["project.toml"]
schema.path = "https://raw.githubusercontent.com/gmrdad82/pfx/<tag>/docs/schema/project.schema.json"
```

Put the pfx tag your project pins in place of `<tag>`, so the editor and the reader agree. One file can also name its schema on its first line:

```toml
#:schema https://raw.githubusercontent.com/gmrdad82/pfx/<tag>/docs/schema/scene.schema.json
format = 1
```

## From the crate

With the `schema` feature, `json_schema(kind)` returns a kind's schema as JSON text, and the `scene-schema` example writes all six into a folder:

```toml
[dependencies]
pfx-scene = { git = "https://github.com/gmrdad82/pfx", tag = "<tag>", features = ["schema"] }
```

```rust,ignore
use pfx_scene::{FileKind, json_schema};

std::fs::write("scene.schema.json", json_schema(FileKind::Scene)).unwrap();
```

```sh
cargo run -p pfx-scene --features schema --example scene-schema -- schemas
```

The feature adds `schemars` and `serde_json`; without it the crate depends on neither.
