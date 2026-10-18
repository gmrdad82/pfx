# The object library

An object library is a folder of finished objects, each one `object.pitofx` holding a standard glTF 2.0 model, its sidecar (a small TOML table of its source, licence and facts) and an optional thumbnail, which any recipe places by name. A model exported from Blender (File > Export > glTF 2.0) works as well as one pfx writes: its hierarchy, materials and textures, morph targets and animations come in.

```
library/
  index.json
  car/sports/object.pitofx
```

An `object.pitofx` is a pfx-file container (`docs/pfx-file.md`) of four chunks: the model's glTF JSON (`GLTF`) and binary buffer (`GBIN`), which join back into the exact GLB it was made from, the sidecar's TOML (`FACT`) and the PNG thumbnail (`THMB`, stored). `pfx library bake` and `pfx library add` write it; `pfx scene print` does not read it, and `pfx export` is how a model leaves as a standalone `.glb`. The model inside stays standard glTF: pfx's own data lives in the sidecar only.

## Place an object

```toml
[[object]]
name = "hero"
kind = "library"
object = "car/sports"
```

The recipe finds the library through its top-level `library = "<path>"`, or else the nearest `library/` holding an `index.json` from the recipe's folder up. Every node with a mesh is a piece `<object>.<node>`, the root `<object>.body`, so a `[[follow]]` spins `hero.wheel_rl` about its axle as it does a generated car's. The recipe reference's "Library" section has every key.

## List, index and check

```
pfx library list [--kind <kind>] [--library <dir>] [--json]
pfx library index [--library <dir>] [--json]
pfx library check [--library <dir>] [--json]
```

- `list` prints each object from `index.json`: kind and name, triangles, size in metres and tags; `--json` gives `{ library, objects: [...] }` with the index's entries.
- `index` rewrites `index.json` from the folders, sorted by kind and name, each entry naming its `object.pitofx` with the sha256 of its model (the joined GLB), its sidecar and its thumbnail; the same folders give the same bytes. `--json` gives `{ index, objects }`.
- `check` validates every object's sidecar (its tables, its kind and name against its folder, and its licence: pfx's own library objects carry the line `MIT, Catalin Ilinca; original, no third-party models or brand marks`, any other object an SPDX licence expression in `object.licence` (ids from pfx's own table of the SPDX License List: the Creative Commons, permissive, copyleft and font licences, or `LicenseRef-<name>`; joined by `OR` or `AND`, no parentheses) with a `credit` line), every model (it parses, its first root is named after the object, it carries none of pfx's own extras), and that `index.json` matches the folders and their sha256s. When `gltf_validator` (the Khronos glTF-Validator) is on the PATH it also validates each model, written for it as a GLB under the current folder's `tmp/pfx/library-check/` and removed after, and reports any error; without it, it says so on stderr. An object whose `licence` is empty is unlicensed: `check` prints `warning: <path>: unlicensed: …` and still exits 0 for it. It prints each problem as `<path>: <text>` and exits 1 when there is one; `--json` gives `{ library, objects, validator, problems: [{ path, text }], warnings: [{ path, text }] }`.
- `add` writes the licence only when it is given: `--licence <SPDX id>` with `--credit "<author, source URL>"` (a licence needs its credit line), and `--author` and `--source` as separate fields; without `--licence` the sidecar's `licence` is empty. `pfx import` takes the same four options and writes them into the document's facts. `pfx_build::library::credits` lists the credits of a library and refuses, naming the object, when one is unlicensed.

Without `--library`, each command uses the nearest `library/` with an `index.json` from the current folder up; `index` also takes a `library/` folder without one yet.
