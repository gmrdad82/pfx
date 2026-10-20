# Your first recipe

A recipe is TOML kept inside a `.pitofx` document, pfx's one user file for recipes and scenes ([.pitofx files](pfx-file.md#the-user-file)). This one is a desk with a box and a brass ball, a camera, a sun and a sky.

## Write the recipe

The recipe's text:

```toml
recipe = 1
name = "desk"

[output]
glb = "desk.glb"
sidecar = "desk.json"
sidecar_dialect = "facts"

[[object]]
name = "top"
kind = "plane"
size = [1.2, 0.8]
material = "oak"

[[object]]
name = "box"
kind = "box"
at = [-0.15, 0.05, 0.0]
size = [0.2, 0.1, 0.14]
bevel = { width = 0.01, segments = 4 }
material = "blue"

[[object]]
name = "ball"
kind = "sphere"
at = [0.15, 0.05, 0.05]
radius = 0.05
material = "brass"

[[camera]]
name = "main"
position = [0.0, 0.35, 0.7]
target = [0.0, 0.03, 0.0]
fov_y_deg = 35.0

[sun]
model = "authored"
toward = [-0.45, 0.82, 0.35]
color = [1.0, 0.92, 0.82]
irradiance = 2.2

[sky]
kind = "analytic"
intensity = 0.9

[materials.oak]
family = "wood"
base = [0.42, 0.28, 0.16]
roughness = 0.55

[materials.blue]
family = "lacquer"
base = [0.08, 0.2, 0.5]
roughness = 0.3

[materials.brass]
family = "metal"
base = [0.85, 0.66, 0.35]
roughness = 0.3
metalness = 1.0
```

Bring it into a document once: save the text as a scratch `desk.toml` and import it, which writes `desk.pitofx` beside it; from then on `desk.pitofx` is the recipe, and the scratch file can go.

```sh
pfx scene migrate desk.toml
```

```
imported desk.toml into desk.pitofx
```

Every command below reads and writes `desk.pitofx` in memory, never a loose TOML file. `pfx scene print desk.pitofx` prints the recipe, `pfx set desk.pitofx object.ball.radius=0.06` changes one value and checks the result, and `pfx scene write desk.pitofx < new.toml` replaces the whole text when it still checks.

Units are metres, Y is up, angles are degrees and colours are linear RGB from 0 to 1. `recipe = 1` is the schema version; [the recipe](recipe-format.md) has every key.

## Check it

```sh
pfx check desk.pitofx
```

A valid recipe prints nothing and exits 0. A problem is printed at its line, and the command exits 1. With a negative radius on the ball:

```
$ pfx check desk.pitofx
desk.toml:27: object[ball].radius: expected a number above 0
```

## Build it

```sh
pfx build desk.pitofx
```

```
desk.glb
```

`pfx build` keeps the document's build current (the import built it already, so its glTF is reused; after an edit it is rebuilt into the `.pitofx` and the `.pitofx` is printed first) and writes the GLB the `[output]` table names beside it; `--out <dir>`, `--set`, `--variant` and the other picks write the GLB, the JSON sidecar and the scene into that folder instead. It needs no GPU and no window, and takes a few milliseconds. The build is deterministic: the same recipe gives the same bytes on any machine.

The sidecar holds what the GLB cannot: the cameras, the sun and the sky, in the `facts` dialect ([file formats](recipe-files.md)).

## Draw it live

```sh
pfx shot --still desk.pitofx --camera main --size 800x500 --out desk.png
```

`pfx shot --still` draws one still through the `main` camera with the engine's live renderer, headless, and prints `desk.png`.

## Path-trace it

```sh
pfx render --recipe desk.pitofx --camera main --size 800x500 --out desk-traced.png
```

```
pfx render --recipe: 800x500, 37.9 samples a pixel on average, 64 at most of 64, threshold 0.01, seed 0: 36 submissions, the longest 10.29 ms, 177 ms on the GPU
desk-traced.png
```

The path written goes to stdout and the summary to stderr. [Path-trace a still](recipe-render.md) covers drafts, crops and the other formats.

## Open it in the window

```sh
pfx gui desk.pitofx
```

Click the ball, drag the gizmo's arrow, and the recipe changes under you, ● after its name until Ctrl+S writes it into `desk.pitofx`; its TOML panel shows the text with syntax highlighting. [Arrange a scene in the window](editor-window.md) says how.
