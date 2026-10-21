# Recipes

A recipe is TOML that describes a scene to build, kept inside a `.pitofx` document: objects from generic kinds (boxes, lathes, sweeps, extrusions, text, cards, imported glTF) and from generators, materials, cameras, lights, the sun and the sky, routes, keys and a timeline. It starts with `recipe = 1`. pfx builds a recipe headless into a GLB, its sidecar and a scene, the same bytes on Linux, Windows and macOS; draws it live with `pfx shot <file.pitofx> --still`; and path-traces it with `pfx render --recipe <file.pitofx>`.

The crates behind it are libraries an app or a game links on its own: `pfx-recipe` (parse, validate, patch), `pfx-mesh` (geometry), `pfx-generators` (a kit for an app's own generators), `pfx-build` (recipe to GLB, sidecar and bake recipe), `pfx-glb` (the deterministic GLB writer and reader), `pfx-motion` (routes, noise, the timeline), `pfx-recipe-scene` (recipe to engine scene and the headless still), `pfx-runtime` (a GLB and its sidecar into live instances) and `pfx-still` (path-traced stills).

[Your first recipe](recipe-first-scene.md) walks through one; [the recipe format](recipe-format.md) is the whole schema; [recipe file formats](recipe-files.md) lists what a build writes; [asset sources](asset-sources.md) fetches CC0 and account assets and builds a scene's credits. `samples/` holds pfx's own sample recipes and `library/` its object library.

## Commands

```
pfx build <file.pitofx> [--variant <name> | --all-variants] [--set key=value]... [--stage N|name] [--only <glob>[,<glob>]] [--out <dir>]
                                                          recipe to GLB, sidecar and bake recipe; --stage builds stages 1 to N, --only those objects, pieces, collections or scatters
pfx check <file.pitofx>                                       validate and list problems
pfx diff <a.glb> <b.glb> [--json] [--render --recipe <file.pitofx> [--camera <name>] [--size WxH] [--floor <dB>]]
                                                          structure, side by side, or both drawn alike and compared by PSNR
pfx shot <file.pitofx> --still --camera <name> --size WxH --out <png> [--variant <name>] [--set key=value]... [--stage N|name] [--only <glob>,...] [--time S] [--format png|png16] [--dither on|off] [--background sky|transparent] [--memory-limit GB]
                                                          one still on the live renderer, headless; --set overrides a value the recipe has
pfx render --recipe <file.pitofx> --camera <name> [--spp N] [--threshold T] [--min-samples N] [--draft] [--seed S] [--size WxH] [--equirect|--ortho] [--crop x,y,w,h] [--variant V] [--set key=value]... [--stage N|name] [--only <glob>,...] [--time S] [--denoise off|on] [--format png|png16] [--dither on|off] [--background sky|transparent] [--master <file.png>] [--progress plain|json] [--checkpoint <minutes|Nspp>] [--memory-limit GB] --out <file.png|file.exr|file.hdr>
                                                          path-traced on the engine's tracer
pfx clip <file.pitofx> --camera <name> (--frames N --fps F [--spin <object> [--turns N]] | --timeline) [--range A..B] --size WxH --out <dir> [--trace [--spp N] [--threshold T] [--checkpoint <minutes|Nspp>] [--denoise off|on] [--passes <dir>] [--dump-scene <file.jsonl>]] [--json] [--rerender-from <frame>] [--variant V] [--set key=value]... [--video <file.mp4|file.webm>] [--format png|png16] [--dither on|off] [--memory-limit GB]
                                                          a camera moving along the recipe's path, as frames and a video
pfx new <out.pitofx> --from street [--seed N] [--json]
                                                          writes a starter chase shot: a 9:16 timeline, a route, a low-rise street, a drifting hero car, traffic, a low chase camera, road spray behind each rear wheel, a golden-hour sun about 5 degrees up dead ahead down the route, a top camera over the route, five stages (street, car, city, camera, look) and lilac.cube, all inside the new .pitofx; it is built on first use
pfx timeline <file.pitofx> [--json]                           builds the recipe and reports its frames, keyed nodes, camera node, each followed object's distance and top speed, and the markers with their frames
pfx shoot <file.pitofx> --out <dir> [--draft|--final] [--range a..b] [--camera <name>] [--memory-limit GB] [--json]
                                                          builds the recipe and renders its timeline with `pfx shot` on the live renderer: frames, the scene's sound as a WAV and shoot.json; resumes where it stopped
pfx library list|index|check [--kind <kind>] [--library <dir>] [--json]
                                                          the library's objects, its index rewritten from the folders, or every object checked against the index
pfx library bake <file.pitofx> <object> --as <kind>/<name> [--thumb] [--tags a,b] [--library <dir>] [--json]
                                                          builds the recipe and writes the object to library/<kind>/<name>/object.pitofx: its model, its sidecar (source, params and facts) and with --thumb a 512x512 still on a neutral ground; then the index
pfx library add <file.glb|file.gltf|file.obj> --as <kind>/<name> [--licence <SPDX id>] [--credit <line>] [--author <name>] [--source <url>] [--thumb] [--tags a,b] [--library <dir>] [--json]
                                                          brings a glTF 2.0 model (.glb, a split .gltf, or an .obj with its .mtl) into the library as one standalone GLB inside its object.pitofx, with its sidecar (file name, sha256, size, wheels and light anchors found by node name), checked by gltf_validator when installed
pfx route <file.pitofx> list|set|add|remove [--route <name>] [--at <i>] [--point x,y,z] [--json]
                                                          list the routes' points and lengths, or move, add or remove a point in the file
pfx set <file.pitofx> <key>=<value>... [--through] [--json | --brief]    writes values into the recipe file by dotted key, as --set takes them, keeping its comments; refused and left untouched when the result does not check
pfx get <file.pitofx> <key>... [--json]                       prints the recipe's values by dotted key, as TOML or JSON
pfx curve <file.pitofx> <object> <param> list|set|add|remove [--at <i>] [--point a,b] [--json]
                                                          list one curve param of a generator object, or move, add or remove a point in the file
pfx add <file.pitofx> <table> <name> [key=value]... [--json | --brief]
                                                          append an entry to object, camera, light, route, stage, particles, follow, use, mover, marker, scatter or collection, in the file
pfx remove <file.pitofx> <table> <name> [--json | --brief]
                                                          remove an entry; refused, naming the referrer, while another entry refers to it
pfx duplicate <file.pitofx> <table> <name> <new-name> [key=value]... [--json | --brief]
                                                          copy an entry under a new name, with keys changed
pfx key <file.pitofx> <path> <t> <value> [--interp linear|smooth|step] [--json | --brief]
                                                          set or insert a key of a property over the timeline, as object.hero.at 2 0,0,12, in the file
pfx unkey <file.pitofx> <path> [<t>] [--json | --brief]                 remove the property's key at t, or every key of it, in the file
pfx align <file.pitofx> <object>... --axis x|y|z [--edge min|center|max] [--to <object>] [--json | --brief]
                                                          moves the objects (names or globs) so one side of their bounds lines up: the lowest min, the highest max or the mean centre of the selection, or the `--to` object's; writes each new `at` into the file
pfx align <file.pitofx> <object>... --distribute --axis x|y|z [--json | --brief]
                                                          spaces the objects' centres evenly between the two outermost on the axis
pfx align <file.pitofx> <object>... --ground [--on <object>] [--json | --brief]
                                                          drops each object straight down until its bounds rest on the highest face below its footprint (of every other object, or the `--on` object's), or on y = 0 when none is below
pfx align <file.pitofx> <object>... --snap <object> --axis x|y|z [--edge min|center|max] [--json | --brief]
                                                          sets each object against the other's bounds: `max` (the default) puts its min side on the other's max side, as a cup on a table with `--axis y`, `min` its max on the other's min, `center` centre on centre
pfx scene list <file.pitofx> [<table>] [--json]               the recipe's entries and their names (an object's kind and parent)
pfx scene print <file.pitofx> [<file>] [--json]            prints a TOML file the .pitofx holds (its recipe by default, an include by its name inside) to stdout, or its table as JSON
pfx scene print <file.pitofx> --all                          prints every entry in a stable order, TOML whole and packed files by size and CRC-32: what git diff shows (docs/git.md)
pfx scene write <file.pitofx> [<file>] < new.toml          replaces that file with TOML from stdin; kept only when the recipe still checks, written in place
pfx scene save <file.pitofx> [--thumb] [--json]             rebuilds the .pitofx from its recipe when it is stale and writes it, with a thumbnail drawn into it if asked
pfx scene migrate <file.toml|folder> [--dry-run] [--bare]
                                                          imports a plain TOML recipe or scene (or every one in a folder) once into <name>.pitofx beside it; the .toml is never written; --bare keeps only the recipe and the files it reads, built on first use
pfx scene fold <file.pitofx> [--apply | --keep] [--json]    lists the scene edits made in a .pitofx, folds them into the recipe or keeps them as overrides
pfx scene compact <file.pitofx> [--json]                    rewrites a .pitofx without the dead bytes its in-place edits left behind
pfx import <file.glb|file.gltf|file.obj> (--as <kind>/<name> | --out <x.pitofx>) [--licence <SPDX id>] [--credit <line>] [--author <name>] [--source <url>] [--tags <a,b>] [--thumb] [--json]
                                                          a glTF 2.0 model (.glb, a split .gltf, or an .obj with its .mtl) into a .pitofx: its glTF packed into one GLB, a one-object recipe and detected facts (size, wheels, light anchors)
pfx export <file.pitofx> --glb <out.glb> [--only <object>] [--json]
                                                          a standalone GLB with its node hierarchy and no pfx extras
pfx asset search [words...] [--source <name>] [--kind hdri|texture|model|pack] [--limit N] [--allow <kind>]... [--json]
                                                          searches the asset sources: Poly Haven, ambientCG, the Kenney and Quaternius catalogs and configured accounts
pfx asset get <source:id> [--out <dir>] [--res <r>] [--format <f>] [--from <file.zip>] [--allow nc|nd|editorial|custom]... [--json]
                                                          downloads an asset, checks it and its licence, unpacks it and records its licence facts in asset.pitofx
pfx asset list|sources [--in <dir>] [--json]             the fetched assets with their licences, or the sources
pfx asset settings [--stdin] [--json]                    pfx's settings.pitofx as TOML, or TOML on stdin written to it
pfx credits <scene.pitofx> [--json]                         one credit line per asset a scene uses, ready for a credits card; unlicensed assets are refused by name
pfx git setup [<folder>] [--json]                           makes git diff show a .pitofx as its TOML: writes .gitattributes and the repo's own git config, once
pfx gui [<recipe|scene|file.pitofx|project folder|folder>] [--play <module.wasm>] [--camera <name>] [--stage N|name] [--only <glob>,...] [--clay] [--wireframe [--blend <0..1>] [--xray]]
                                                          the window on a recipe, a scene, a .pitofx, a game's project or the start screen; Play runs the scene with a game's module (docs/editor.md)
pfx gui [<file>] --shot <png> [--size WxH] [--input <script>] [--fake] [--fixture <name>] [--start] [--bare [--strip]] [--clean]
                                                          one frame of the window, offscreen (docs/editor-shots.md)
pfx gui [<file>] --clip <dir> --tape <file> [--fps 30|60|120] [--poster first|last|S | --frames] [--scale F] [--framed [--window WxH]] [--size WxH] [--start] [--bare [--strip]] [--clean] [--json]
                                                          the window playing a tape, filmed offscreen into an H.264 master, a poster and a manifest
```

`add`, `remove`, `duplicate` and `align` write the recipe through the same edits the editor makes, keeping its comments, check the recipe again after the write, and put it back byte for byte when it no longer checks.

A recipe lives in a `.pitofx` document, pfx's one user file: the recipe, every file it includes or reads, its build and its scene in one compressed container (docs/pfx-file.md). Every command reads the recipe and its files from the document in memory, never unpacking them to disk, and `set`, `route` and every other command that edits the recipe write the edit alone back into the `.pitofx` in place, in milliseconds whatever its size (docs/pfx-file.md, Editing in place), leaving its build for the next `pfx build` or save; a command that fails leaves the `.pitofx` as it was. To read what is inside, `pfx scene print` shows the recipe or any TOML file the document holds, and the editor opens it in its TOML panel with syntax highlighting.

A recipe or scene written as TOML is imported once: `pfx scene migrate street.toml` writes `street.pitofx` beside it (a format 0 scene is migrated to format 1 inside the document), and the `.toml` is never written again. Every other command given a `.toml` refuses with one line naming that command: `pfx build: street.toml is a plain TOML file; import it once with `pfx scene migrate street.toml`, which writes street.pitofx beside it`.

`pfx <command> --help` prints a command's whole help. A line pfx cannot parse gets clap's short error on stderr and exits 2; a recipe, camera or file it refuses after that is one line on stderr, `pfx <command>: …`, and exits 1. A file whose header claims more memory than the machine (or a `ulimit -v` / `prlimit --as` limit) can give is refused the same way before pfx fills it, never by an abort: `pfx scene print: big.pitofx: chunk RCPE needs 3221225472 bytes: memory allocation failed because the memory allocator returned an error`. The buffers a file or a recipe sizes are reserved that way: `.pitofx` chunks, PNG and JPEG textures, `.hdr` skies (which must also hold at least the shortest scanlines their size needs) and the traced and denoised frames read back from the GPU.

pfx tells a recipe from its other inputs by the command line: `pfx render --recipe` is a recipe's traced still while `pfx render --app|--asset|--icons|--scene` stay pfx's offline renders; `pfx shot` takes a recipe's `.pitofx` when `--still` is given, and draws a `.pitofx`'s scene otherwise; `pfx new <out> --from <template>` writes a starter recipe while `pfx new <game>` scaffolds a game; `pfx scene list|save|print|write|fold` work on recipe documents (`print` and `write` on scene documents too) while `pfx scene migrate|fix` import and fix scenes.
