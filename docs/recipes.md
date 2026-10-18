# Recipes

A recipe is one TOML file that describes a scene to build: objects from generic kinds (boxes, lathes, sweeps, extrusions, text, cards, imported glTF) and from generators, materials, cameras, lights, the sun and the sky, routes, keys and a timeline. It starts with `recipe = 1`. pfx builds a recipe headless into a GLB, its sidecar and a `scene.toml`, the same bytes on Linux, Windows and macOS; draws it live with `pfx shot <recipe> --still`; and path-traces it with `pfx render --recipe <recipe>`.

The crates behind it are libraries an app or a game links on its own: `pfx-recipe` (parse, validate, patch), `pfx-mesh` (geometry), `pfx-generators` (a kit for an app's own generators), `pfx-build` (recipe to GLB, sidecar and bake recipe), `pfx-glb` (the deterministic GLB writer and reader), `pfx-motion` (routes, noise, the timeline), `pfx-recipe-scene` (recipe to engine scene and the headless still), `pfx-runtime` (a GLB and its sidecar into live instances) and `pfx-still` (path-traced stills).

[Your first recipe](recipe-first-scene.md) walks through one; [the recipe format](recipe-format.md) is the whole schema; [recipe file formats](recipe-files.md) lists what a build writes. `samples/` holds pfx's own sample recipes and `library/` its object library.

## Commands

```
pfx build <recipe> [--variant <name> | --all-variants] [--set key=value]... [--stage N|name] [--only <glob>[,<glob>]] [--out <dir>]
                                                          recipe to GLB, sidecar and bake recipe; --stage builds stages 1 to N, --only those objects or pieces
pfx check <recipe>                                       validate and list problems
pfx diff <a.glb> <b.glb> [--json] [--render --recipe <recipe> [--camera <name>] [--size WxH] [--floor <dB>]]
                                                          structure, side by side, or both drawn alike and compared by PSNR
pfx shot <recipe> --still --camera <name> --size WxH --out <png> [--variant <name>] [--set key=value]... [--stage N|name] [--only <glob>,...] [--time S]
                                                          one still on the live renderer, headless; --set overrides a value the recipe has
pfx render --recipe <recipe> --camera <name> [--spp N] [--threshold T] [--min-samples N] [--draft] [--seed S] [--size WxH] [--equirect|--ortho] [--crop x,y,w,h] [--variant V] [--set key=value]... [--stage N|name] [--only <glob>,...] [--time S] [--denoise off|on] [--master <file.png>] [--progress plain|json] --out <file.png|file.exr|file.hdr>
                                                          path-traced on the engine's tracer
pfx clip <recipe> --camera <name> (--frames N --fps F [--spin <object> [--turns N]] | --timeline) --size WxH --out <dir> [--trace [--spp N]] [--variant V] [--set key=value]... [--video <file.mp4|file.webm>]
                                                          a camera moving along the recipe's path, as frames and a video
pfx new <out> --from street [--seed N] [--json]
                                                          writes a starter chase shot: a 9:16 timeline, a route, a low-rise street, a drifting hero car, traffic, a low chase camera, road spray behind each rear wheel, a golden-hour sun about 5 degrees up dead ahead down the route, a top camera over the route, five stages (street, car, city, camera, look) and lilac.cube beside the recipe
pfx timeline <recipe> [--json]                           builds the recipe and reports its frames, keyed nodes, camera node, each followed object's distance and top speed, and the markers with their frames
pfx shoot <recipe> --out <dir> [--draft|--final] [--range a..b] [--camera <name>] [--json]
                                                          builds the recipe and renders its timeline with `pfx shot` on the live renderer: frames, the scene's sound as a WAV and shoot.json; resumes where it stopped
pfx library list|index|check [--kind <kind>] [--library <dir>] [--json]
                                                          the library's objects, its index rewritten from the folders, or every object checked against the index
pfx library bake <recipe> <object> --as <kind>/<name> [--thumb] [--tags a,b] [--library <dir>] [--json]
                                                          builds the recipe and writes the object to library/<kind>/<name>: model.glb, object.toml (its source, params and facts) and with --thumb a 512x512 still on a neutral ground; then the index
pfx library add <file.glb> --as <kind>/<name> [--thumb] [--tags a,b] [--library <dir>] [--json]
                                                          brings a glTF 2.0 binary file into the library: copies it, writes its object.toml (file name, sha256, size, wheels and light anchors found by node name), checked by gltf_validator when installed
pfx route <recipe> list|set|add|remove [--route <name>] [--at <i>] [--point x,y,z] [--json]
                                                          list the routes' points and lengths, or move, add or remove a point in the file
pfx set <recipe> <key>=<value>... [--json]               writes values into the recipe file by dotted key, as --set takes them, keeping its comments; refused and left untouched when the result does not check
pfx get <recipe> <key>... [--json]                       prints the recipe's values by dotted key, as TOML or JSON
pfx curve <recipe> <object> <param> list|set|add|remove [--at <i>] [--point a,b] [--json]
                                                          list one curve param of a generator object, or move, add or remove a point in the file
pfx add <recipe> <table> <name> [key=value]... [--json]
                                                          append an entry to object, camera, light, route, stage, particles, follow, use, mover or marker, in the file
pfx remove <recipe> <table> <name> [--json]
                                                          remove an entry; refused, naming the referrer, while another entry refers to it
pfx duplicate <recipe> <table> <name> <new-name> [key=value]... [--json]
                                                          copy an entry under a new name, with keys changed
pfx key <recipe> <path> <t> <value> [--interp linear|smooth|step] [--json]
                                                          set or insert a key of a property over the timeline, as object.hero.at 2 0,0,12, in the file
pfx unkey <recipe> <path> [<t>] [--json]                 remove the property's key at t, or every key of it, in the file
pfx scene list <recipe> [<table>] [--json]               the recipe's entries and their names (an object's kind and parent)
pfx scene save <recipe> --pfx <out.pfx> [--thumb] [--json]
                                                          packs the recipe, its includes and every file it reads, its build and its scene into one .pfx
pfx scene unpack <file.pfx> <dir> [--json]               writes a .pfx's recipe and files into a folder, so building it there gives the same GLB
pfx scene fold <file.pfx> [--apply | --keep] [--json]    lists the scene edits made in a .pfx, folds them into the recipe or keeps them as overrides
pfx import <file.glb> (--as <kind>/<name> | --out <x.pfx>) [--tags <a,b>] [--thumb] [--json]
                                                          a glTF 2.0 binary (a Blender export too) into a .pfx: its glTF as-is, a one-object recipe and detected facts (size, wheels, light anchors)
pfx export <file.pfx|recipe> --glb <out.glb> [--only <object>] [--json]
                                                          a standalone GLB with its node hierarchy and no pfx extras
```

`add`, `remove` and `duplicate` write the recipe file through the same edits the editor makes, keeping its comments, check the recipe again after the write, and put the file back byte for byte when it no longer checks.

Every command that takes a recipe also takes a recipe `.pfx` document: it reads the recipe and its files from the document in memory, and `route` (and every other command that edits the recipe) writes the edit and a fresh build back into the `.pfx`.

`pfx <command> --help` prints a command's whole help. A line pfx cannot parse gets clap's short error on stderr and exits 2; a recipe, camera or file it refuses after that is one line on stderr, `pfx <command>: …`, and exits 1.

pfx tells a recipe from its other inputs by the command line: `pfx render --recipe` is a recipe's traced still while `pfx render --app|--asset|--icons|--scene` stay pfx's offline renders; `pfx shot` takes a recipe when `--still` is given or the file is a recipe (a TOML file whose top level has `recipe = N`), and a `scene.toml` or a scene `.pfx` otherwise; `pfx new <out> --from <template>` writes a starter recipe while `pfx new <game>` scaffolds a game; `pfx scene list|save|unpack|fold` work on recipes while `pfx scene migrate|fix` work on scene files.
