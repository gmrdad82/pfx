# Recipe documents in .pfx files

A `.pfx` is pfx's document, as a `.blend` is Blender's: one file with the recipe, its includes and every file it reads (LUTs, HDRs, PNG maps, mesh GLBs), the built glTF, the pfx scene built from it, the facts of every object, a map of the glTF nodes and an optional thumbnail.

It is a private container that only pfx opens, built for speed. A chunk table sits at the front, so a reader takes the thumbnail or the facts without touching the meshes, and an unchanged document opens without a rebuild. It is not a zip, which would make every open inflate and scan it, and not a GLB, which has no room for the recipe that made it. Other tools such as Blender exchange GLB files with pfx, never `.pfx`. The layout is in [the formats reference](pfx-file.md).

## Save, unpack and build

```
pfx scene save samples/chase.toml --pfx chase.pfx [--thumb] [--json]
pfx scene unpack chase.pfx chase/ [--json]
pfx build chase.pfx
```

`save` loads and builds the recipe, records every file the load, the build and the scene read, and writes them into the document with the recipe, named by their paths from the nearest folder that holds them all. `--thumb` draws a 320 × 200 thumbnail through the recipe's first camera. `unpack` writes the recipe and its files back under a folder at the same paths, so `pfx build` there gives the same GLB bytes.

`pfx build chase.pfx` writes the document's GLB without building while the recipe and its files are the ones it was built from, and says so on stderr; after a change it builds once and rewrites the document. `--variant`, `--all-variants` and `--set` build the document's recipe like any other recipe and leave the document as it is.

## Every command takes a .pfx

`check`, `route`, `timeline`, `shot`, `render`, `clip`, `shoot` and `gui` take a `.pfx` wherever they take a recipe. pfx reads the recipe from the document and resolves its paths from the document's files in memory, with no temporary files, as if the files sat in the `.pfx`'s folder: outputs land there, as they would beside the unpacked recipe.

A command that edits the recipe, such as `pfx route chase.pfx set --route main --at 2 --point 4,0,30`, writes the edit and a fresh build back into the `.pfx` in one atomic replace. The window writes its edits back into the document when it closes.

## When pfx edits the scene

The document carries two sources of truth: the recipe, pfx's, and the scene, pfx's. pfx may edit the scene, never the recipe. pfx records a hash of the scene it built; when the scene inside a `.pfx` no longer matches it, every command refuses to rebuild over the edits in one line naming `pfx scene fold`:

```
pfx scene fold chase.pfx            # list the edits by scene path
pfx scene fold chase.pfx --apply    # fold them into the recipe
pfx scene fold chase.pfx --keep     # keep them as overrides
```

`--apply` writes each edit whose scene path has a recipe key (a light's intensity, colour, position, radius, range, direction, cone or shadow) into the recipe, in the file that holds it, with the same patch code as every recipe edit; an edit with no recipe key, such as a generated object's material, stays as an override, and the list says which. `--keep` keeps every edit as an override. Overrides apply on top of every later build's scene, so the edit survives the next rebuild. `--json` prints the edits, each with its scene file, path, old and new value and recipe key.
