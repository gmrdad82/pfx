# Recipe documents in .pitofx files

A `.pitofx` is pfx's document, as a `.blend` is Blender's: one file with the recipe, its includes and every file it reads (LUTs, HDRs, PNG maps, mesh GLBs), the built glTF, the pfx scene built from it, the facts of every object, a map of the glTF nodes and an optional thumbnail.

It is a private container that only pfx opens, built for speed. A chunk table sits at the front, so a reader takes the thumbnail or the facts without touching the meshes, and an unchanged document opens without a rebuild. It is not a zip, which would make every open inflate and scan it, and not a GLB, which has no room for the recipe that made it. Other tools such as Blender exchange GLB files with pfx, never `.pitofx`. The layout is in [the formats reference](pfx-file.md).

## Import, read and build

A `.pitofx` is the one file a user receives and edits for a recipe; TOML is the syntax inside it, never a loose file beside it.

```
pfx scene migrate old/chase.toml [--bare] [--dry-run]
pfx new chase.pitofx --from street [--seed N]
pfx scene print chase.pitofx [<file>] [--json]
pfx scene write chase.pitofx [<file>] < new.toml
pfx scene save chase.pitofx [--thumb] [--json]
pfx build chase.pitofx
```

`migrate` imports a plain TOML recipe once: it loads and builds the recipe, records every file the load, the build and the scene read, and writes them into `chase.pitofx` beside it with the recipe, named by their paths from the nearest folder that holds them all; the `.toml` is never written again, and a second import is refused. `--bare` keeps only the recipe and the files it reads, and the first command that builds the document writes the build in. `new --from street` writes a starter recipe straight into a new document. `print` shows the recipe, or an included file by its name inside, on stdout (`--json` gives its table as JSON); `write` replaces it from stdin and keeps the change only when the recipe still checks. `save` rebuilds a stale document and, with `--thumb`, draws a 320 × 200 thumbnail through the recipe's first camera into it. Nothing writes the recipe or its files out to disk.

`pfx build chase.pitofx` writes the document's GLB without building while the recipe and its files are the ones it was built from, and says so on stderr; after a change it builds once and rewrites the document. `--variant`, `--all-variants` and `--set` build the document's recipe like any other recipe and leave the document as it is.

## Every command takes a .pitofx

`build`, `check`, `set`, `get`, `key`, `unkey`, `route`, `curve`, `add`, `remove`, `duplicate`, `scene list`, `timeline`, `shot --still`, `render --recipe`, `clip`, `shoot`, `export` and `edit` take the `.pitofx`, and every one but `edit` refuses a plain `.toml` with one line naming `pfx scene migrate`. pfx reads the recipe from the document and resolves its paths from the document's files in memory, with no temporary files, as if the files sat in the `.pitofx`'s folder: outputs land there.

A command that edits the recipe, such as `pfx route chase.pitofx set --route main --at 2 --point 4,0,30` or `pfx set chase.pitofx sky.intensity=1.8`, writes the edit and a fresh build back into the `.pitofx` in one atomic replace; one that fails leaves the `.pitofx` as it was. The window writes its edits back into the document when it closes, and its TOML panel shows the recipe with syntax highlighting.

## When pfx edits the scene

The document carries two sources of truth: the recipe, pfx's, and the scene, pfx's. pfx may edit the scene, never the recipe. pfx records a hash of the scene it built; when the scene inside a `.pitofx` no longer matches it, every command refuses to rebuild over the edits in one line naming `pfx scene fold`:

```
pfx scene fold chase.pitofx            # list the edits by scene path
pfx scene fold chase.pitofx --apply    # fold them into the recipe
pfx scene fold chase.pitofx --keep     # keep them as overrides
```

`--apply` writes each edit whose scene path has a recipe key (a light's intensity, colour, position, radius, range, direction, cone or shadow) into the recipe, in the file that holds it, with the same patch code as every recipe edit; an edit with no recipe key, such as a generated object's material, stays as an override, and the list says which. `--keep` keeps every edit as an override. Overrides apply on top of every later build's scene, so the edit survives the next rebuild. `--json` prints the edits, each with its scene file, path, old and new value and recipe key.
