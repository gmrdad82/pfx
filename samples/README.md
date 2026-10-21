# Samples

pfx's own scenes, built only from generic kinds and their own materials, to show what pfx and the engine can do. Each is a `.pitofx` document holding its recipe and the files it reads, apart from a music style it links; it is built the first time a command needs it. Open one in the window, play with it, then render it big. An app's scenes live in the app's own repo, and its own generators register through `pfx_build::Registry` (`docs/recipe-format.md`, generators).

- `workshop.pitofx`: a workbench before a plaster wall, under a lamp room whose window throws striped light: a teal case with two pockets, a brass lip and a dial and lid that move, a glazed vase, a glass jar, a cable, a spool, a mirror ball, a cupped note and the word `pfx` on the case. Cameras `hero`, `top`, `detail` and `orbit`, which carries an orbit path for `pfx clip`.
- `hero.pitofx`: the README's picture: a still life on a blueprint-blue cove under a low warm sun and a cool fill, with a flag above the set, out of view, shading the upper wall: a lacquer case with two pockets, a brass lip, an amber knob and a glowing `recipe = 1`, a swept cable, a gold ring, a chrome ball, an amber capsule, a lathed porcelain vase on an oak plinth, a lathed wine glass and a cupped note. Camera `hero`. Its `[trace]` keeps the chrome a mirror and the glass crisp with no fireflies under the small sun: `filter_glossy = 0.25`, with `clamp_indirect = 4.0` as the backstop.
- `figurine.pitofx`: a figurine from an imported GLB, `figurine.glb`, held inside the document: a head with morph targets (`smile`, `jaw open`, `brow raise`) and a hat child node on a capsule body. Variants `grin` (morph weights) and `dusk` (a low sun, the rim light removed); cameras `hero` and `orbit`, which carries an orbit path for `pfx clip`.
- `gallery.pitofx`: the 2.5D case: three cards with slot placeholders in a wooden frame behind a glass pane, a caption, and an overlay title. Cameras `flat` (orthographic) and `angle`.
- `courtyard.pitofx`: the window's frame-rate reference, a scene of moderate size: a paved courtyard under an afternoon sun and an analytic sky, ringed by sixteen plaster pillars and four walls, with benches, glazed planters, brass lamps with glass globes, a slate fountain with a brass rim and a mirror orb, 95 objects in all. A teal cart drives a loop route for 10 s; camera `chase` follows it and `hero` looks over the yard. `docs/window.md`, "Measured pacing", times `pfx gui` on it at 2560x1440 and 3840x2160.
- `chase.pitofx` and `mustang/`: `chase.pitofx` composes its music in the style kept in `mustang/mustang.pitofx`, which it links with `[compose.link]` instead of holding it, so copy the `mustang` folder with it (`docs/compose.md`, "Bring your own style").

Run everything from the repo's root. To read a sample's recipe, print it; to change it, use the editor's TOML panel or the verbs, which write straight into the `.pitofx`:

```
pfx scene print samples/workshop.pitofx
pfx set samples/workshop.pitofx seed=7
```

## Open one in the window

```
pfx gui samples/workshop.pitofx
pfx gui samples/gallery.pitofx
```

## What to try

- **Orbit:** drag in the viewport to turn the view, hold Shift or drag with the middle button to pan, scroll to dolly. In the cameras panel, `↑` and `↓` jump between the cameras, and `0` goes back to the chosen one.
- **The hour scrubber:** the slider in the sky panel. The sun and the lamp room's ambient follow the hour.
- **Picking:** click a part, the glass jar or the word on the case. The outliner selects it and the properties panel shows where it sits.
- **Moving a part:** with a part picked, `g` moves it, `r` rotates, `s` scales (hold Ctrl to snap). The new place is written straight into the `.pitofx`, and Ctrl+Z puts it back.
- **Setting a camera:** orbit to a view you like and press `s` in the cameras panel; the camera is rewritten in the `.pitofx`.

## A hi-res still

The live look at 1920x1080, a few seconds:

```
pfx shot samples/workshop.pitofx --still --camera hero --size 1920x1080 --out tmp/workshop-hero.png
```

The traced still; the GPU time grows with the size and the samples, so it runs under the optional GPU queue `pgpu` when that is on the PATH, and directly otherwise:

```
pfx render --recipe samples/workshop.pitofx --camera hero --size 3840x2160 --spp 1024 --out tmp/workshop-hero-4k.png
pfx render --recipe samples/gallery.pitofx --camera flat --size 2400x1600 --spp 256 --out tmp/gallery-flat.png
```

## A clip

```
pfx clip samples/workshop.pitofx --camera orbit --frames 240 --fps 30 --size 1920x1080 --out tmp/workshop-orbit --video tmp/workshop-orbit.mp4
```

`--trace --spp 128` gives the traced look.
