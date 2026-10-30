# pfx-scene

<p align="center"><img src="../../docs/images/scene-check.gif" width="800" alt="The scene-check example on the demo project: a path with the wrong case and a misspelt material, each reported at its line and column with its code, then both fixed and the scene resolved"></p>

One TOML format for 3D scenes, prefabs and material libraries, shared by a game engine and a 3D editor, so a scene saved in one opens in the other. This crate reads, checks, resolves, edits, migrates and packs it.

- [The pfx-scene crate](../../docs/scene-crate.md): what it is, how to take it, the demo and the API.
- [The scene format](../../docs/scene-format.md): every file kind, key, type, default and range.
- Guides: [lay out a project](../../docs/scene-projects.md), [check a project](../../docs/scene-check.md), [mark mistakes in an unsaved buffer](../../docs/scene-check-buffer.md), [resolve a scene](../../docs/scene-resolve.md), [edit a scene in place](../../docs/scene-edit.md), [fill missing ids](../../docs/scene-ids.md), [import a TOML file](../../docs/scene-migrate.md), [validate in a TOML editor](../../docs/scene-schema.md) and [measure the speed](../../docs/scene-speed.md).
- The JSON Schemas: [docs/schema/](../../docs/schema/).
- The `.pitofx` container a scene packs into: [pfx-file.md](../../docs/pfx-file.md).
