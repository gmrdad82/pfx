# Build GLBs from recipes

`pfx build` turns a recipe into the files a game or an app loads. It is CPU only: no GPU, no window, no clock and no randomness beyond the recipe's `seed`, so the same recipe gives the same bytes on any machine and a changed GLB in a diff is a changed scene.

## What a build writes

The recipe's `[output]` table names the files, relative to the recipe:

```toml
[output]
glb = "scene/desk.glb"
sidecar = "scene/desk.json"
sidecar_dialect = "facts"
bake = "assets/bake.toml"
scene = "scene/desk.scene.toml"
```

- `glb`: the meshes, their materials by name, the node tree. Without it the build writes `<name>.glb`. A part repeated across the scene is written once: a placed part that is an affine copy of an earlier one (the same triangles, UVs and material slots, every vertex within 1e-5 of its size plus 1e-6 of its distance from the origin once carried by one transform, every normal within 1e-4) becomes a node with that transform on the earlier part's glTF mesh, and parts whose meshes are equal (a scatter's, a library object's) share one mesh too; images are embedded once by their bytes. A file material whose name an earlier object already uses for a different material is renamed `<object>.<name>` after the first object that brings it, and every later object bringing the same material takes that name, so the copies of one model keep one material and share their mesh. A loaded scene keeps one geometry for each shared glTF mesh, and the tracer one bottom level.
- `sidecar` with `sidecar_dialect`: the cameras, sun, sky, look and movers as JSON. `facts` is pfx's own dialect; an [app's build binary](recipe-format.md#generators) can register its own.
- `bake`: a `pfx bake` recipe for the scene, with the same sky, sun and files.
- `scene`: the engine's runtime scene file, which the window and pfx open.

The recipe lives in a `.pitofx` ([.pitofx files](pfx-file.md#the-user-file)), and the build goes into it too: its glTF and its scene, reused while the recipe and its files are unchanged. `pfx build <x.pitofx>` keeps that build current and writes the GLB beside the document; with `--set`, `--variant`, `--all-variants`, `--stage` or `--only` it writes every `[output]` file beside it (or into `--out`) and leaves the document as it is. `--out <dir>` writes the files into one folder instead:

```
$ pfx build samples/workshop.pitofx --out out
samples/workshop.pitofx
out/workshop.glb
```

Each path written is printed on stdout: the `.pitofx` first when it was rebuilt (a sample is shipped without its build, so the first build writes it in), then the GLB.

## Variants

A variant is a named change to the recipe, built into its own files:

```toml
[variant.bare]
drop = ["slat*", "wall_*", "vase"]
set = { "sun.irradiance" = 0.0 }
remove = ["light.lamp"]
```

```sh
pfx build desk.pitofx --variant bare     # writes desk-bare.glb and desk-bare.json
pfx build desk.pitofx --all-variants     # the recipe, then every variant in name order
```

`drop` removes objects by glob; `set` and `remove` take dotted paths, with an entry of an array of tables addressed by its name (`camera.main.fov_y_deg`, `light.lamp`). A variant's name goes before the last extension of each file name, unless `[variant.<name>.output]` names its files.

A recipe's `[[stage]]`s build the scene up one step at a time:

```sh
pfx build street.pitofx --stage 1        # the first stage's objects only
pfx build street.pitofx --stage city     # stages 1 to city, the rest dropped
pfx build street.pitofx --only road,hero # only these objects (globs, or <object>.<piece>)
```

The generators `pfx/road`, `pfx/buildings` and `pfx/furniture` take `reveal = 0..1` to build only the part of the street from the route's start up to that share of its length.

## One-off overrides

`--set key=value` replaces a value the recipe already has (or adds a mesh object's morph weight, `object.<name>.morph.<target>`), for that run only; the file is never written. `build`, `shot`, `render` and `clip` take it, as often as needed:

```
$ pfx build desk.pitofx --set object.ball.radius=0.08 --out big
big/desk.glb
big/desk.json
```

The value takes the kind of the one it replaces. A key the recipe does not carry is refused in one line, exit 1; a missing `=` is a line pfx cannot parse, exit 2.

## Edit the recipe from the command line

`add`, `remove`, `duplicate` and `list` change and read the recipe's entries inside its `.pitofx`: objects, cameras, lights, routes, stages, particles, follows, uses, movers and markers. They write the recipe the way the window does, keeping its comments, then check it again and write the recipe entry back into the `.pitofx` in place (the build inside is redone by the next `pfx build` or save); a write that brings in a problem is refused in one line, exit 1, and the `.pitofx` is left as it was.

```
$ pfx add desk.pitofx object lamp2 kind=sphere radius=0.1 at='[0,2,0]'
desk.toml: added object[lamp2]  kind sphere
$ pfx duplicate desk.pitofx object lamp2 lamp3 at='[1,2,0]'
desk.toml: duplicated object[lamp3]  kind sphere
$ pfx remove desk.pitofx object desk
pfx remove: desk.pitofx: object[desk]: object[lamp].parent refers to it; change or remove object[lamp].parent first
$ pfx scene list desk.pitofx marker --json
{"entries":[{"name":"launch","t":1.25,"table":"marker"}],"recipe":"desk.toml"}
```

A value that is not TOML is a string (`kind=sphere`), a dotted key writes a dotted key (`params.radius=0.1`), and a name the table already has is refused. `remove` names the entry that still refers to the one asked for: a child's `parent`, a follow, a camera rig, a light or emitter riding it, a stage whose glob only it matches, a follow or generator on a route, or the scene's camera. A marker (`[[marker]] name, t`) names a time on the timeline: `pfx timeline` lists them, the window's `[` and `]` jump between them, and a tape's `seek <marker|seconds>` sets the clock.

## Problems

`pfx check` and `pfx build` report a problem at its line, naming the key, and exit 1:

```
$ pfx build samples/workshop.pitofx --variant nonesuch --out out
pfx build: samples/workshop.pitofx: variant.nonesuch: no such variant, the recipe has no variants
```

## Compare two builds

`pfx diff` compares two GLBs by structure, node by node:

```
$ pfx diff out/desk.glb big/desk.glb
~ ball
    bounds [0.1000 -0.0000 -0.0000]..[0.2000 0.1000 0.1000] -> [0.0700 -0.0300 -0.0300]..[0.2300 0.1300 0.1300]
    area 0.031388 -> 0.080353
```

It exits 0 with `no differences beyond tolerance` when they match and 1 when they differ. `--json` prints the same report as JSON ([file formats](recipe-files.md)). `--render --recipe <x.pitofx>` also draws both GLBs alike through the recipe's camera and compares the pictures by PSNR, failing under `--floor` (35 dB by default):

```sh
pfx diff out/desk.glb big/desk.glb --render --recipe desk.pitofx --camera main
```
