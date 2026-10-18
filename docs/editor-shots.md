# Shoot and film the window headless

The window can be drawn offscreen, with no compositor and nothing on your desktop: one frame with `--shot`, or a whole clip with `--clip`. A program or an AI agent sees what the window shows this way, and every window clip in these pages was made by it.

## One frame

```sh
pfx gui samples/workshop.pitofx --shot workshop.png
pfx gui --start --shot start.png --size 1280x800
```

`--size` sets the window (1586x992 by default). The frame shows the settled window: the scene built, the stage drawn, every panel laid out, and the viewport drawn until it settles as the window does after a change (16 frames, each stepping the temporal AA jitter and the screen-space reflections' sample pattern), so its edges and glossy paint come out as smooth as the window's. `--bare` draws the viewport alone instead, filling the whole frame (see below).

`--wireframe` draws the viewport as a wireframe, `--blend <0..1>` stops part way between shaded and wireframe, and `--xray` shows the hidden edges brighter. `--clay` draws it in clay: every surface a matte grey under a soft key light and sky, no textures, emission or effects, the selection outlined in the accent colour. In the window `z` cycles shaded, clay and wireframe. They work for a recipe, with `--shot` and with `--clip`:

```sh
pfx gui samples/chase.pitofx --shot clay.png --clay
pfx gui samples/chase.pitofx --shot wire.png --wireframe
pfx gui samples/chase.pitofx --shot half.png --wireframe --blend 0.5
```

A scene, a `.pitofx` scene or a project folder shoots the same way, in the window's scene panels (`docs/editor.md`, "Scenes in the window"):

```sh
pfx gui crates/editor/tests/scenes/room.scene.toml --shot room.png
```

**Play in a shot.** `--play <module.wasm>` chooses the module, and a script presses F5 and lets frames pass; the viewport then shows the game's frame:

```sh
cargo run -p pfx-game --features modules --example marker -- tmp/play/marker.wasm
printf 'key F5\nframe\nframe\nframe\nframe\n' > tmp/play.txt
pfx gui crates/editor/tests/scenes/room.scene.toml --play tmp/play/marker.wasm --input tmp/play.txt --shot play.png
```

## Play a script first

`--input <script>` plays a script of one step a line before the frame. Coordinates are points from the window's top left, and a line starting with `#` is a comment.

```
# pick the vase and look at its properties
click 995 480
frame
key g
```

| Step | Does |
| --- | --- |
| `key <name>` | one key by egui's name (`g`, `Escape`, `Enter`, `ArrowDown`); `ctrl+`, `shift+`, `alt+` hold a modifier: `key ctrl+z` |
| `text <words>` | types the rest of the line |
| `move x y` | moves the pointer |
| `click x y` | presses and releases |
| `press x y` / `release x y` | holds the button down until the release, for a drag a frame can stop inside |
| `drag x y x y [middle]` | a whole drag in one line |
| `scroll x y lines` | the wheel, positive to come nearer |
| `frame` | draws one pass |
| `wireframe on` / `wireframe off` | switches the viewport to wireframe or back to shaded, in one frame |
| `wireframe <amount> [<seconds>]` | fades the viewport to an amount between shaded (0) and wireframe (1) over the seconds, one even step a frame at the clip's rate; without seconds it jumps; in a tape the fade takes its time like a `wait` |
| `wireframe xray on` / `wireframe xray off` | shows the hidden edges brighter, or dimmed again |
| `clay on` / `clay off` | switches the viewport to clay (wireframe off) or back to shaded, in one frame |
| `shade shaded\|clay\|wireframe [<seconds>]` | fades the viewport to that mode over the seconds, one even step a frame; into wireframe the clay stays under the fading edges, out of it the clay is there as they fade; without seconds it jumps |
| `seek <marker\|seconds>` | sets the timeline's clock to a marker's time or to seconds, paused, in one frame; a marker the scene lacks stops the tape, naming its line |

A tape also directs the view. These steps work in `--tape` (not in `--input`). Each timed one takes its frames like a `wait`, one even step a frame at the clip's rate, eased in and out (`reveal` is linear):

| Step | Does |
| --- | --- |
| `stage <N\|name>` | rebuilds the recipe as its `[[stage]]` N (stages 1 to N) or the stage of that name, and waits for the build |
| `view top\|front\|side\|back\|orbit` | snaps to the window's orthographic view, as keys 7, 1, 3 and 9 do (`top` looks down, `front` from +Z, `side` from +X, `back` from -Z), framing the selection or the whole scene; `orbit` returns to perspective, as key 5 does |
| `camera <name>` | looks through a recipe camera; a camera with a `[camera.rig]` looks through its baked track at the clock's time (the chase cam) |
| `select <name>[,<name>]` | selects objects, pieces (`road.crosswalks`, `hero.wheel_rl`) or a light by name, `*` and `?` globs allowed; the wireframe draws them in the accent colour, and a piece alone outlines only that piece |
| `orbit <degrees> <seconds>` | turns the view about its target, positive as a drag to the right turns it |
| `dolly <metres> <seconds>` | moves the view toward its target, negative away |
| `frame <name>[,<name>] [<seconds>]` | fits the view to objects or pieces, keeping its direction; without seconds it jumps; `Home` frames the selection |
| `reveal <object> <fraction> <seconds>` | animates a generator's `reveal` param (`pfx/road`, `pfx/buildings`, `pfx/furniture`) from its value to the fraction, rebuilding every frame |
| `region <x0> <y0> <x1> <y1>` / `region off` | while the view looks through a camera, shows the shaded render inside that box and the wireframe outside it; the corners are fractions of the viewport from its top left |
| `strip off` / `strip on` | hides or shows the timeline strip under the viewport |

`orbit`, `dolly`, `view` and `frame` leave a camera you are looking through and start from where it looked. A name the window can't find stops the clip with the tape's line: `pfx gui --clip: line 4: no camera chase`.

In wireframe the viewport also draws, in the accent colour: every camera's frustum (its lens at the timeline's aspect, 2.5 m deep) except the one you look through, the chase camera's baked path as a line, each spot light's cone (its apex and its inner and outer rings at its range) and each point light as a small cross (accent when picked in B3), each particle emitter as 36 seeded dots along its direction and cone at its parent's pose at the clock's time, and the camera's frame as a rectangle when the view looks through a camera. With `--bare`, a camera's frame fills the whole viewport at the camera's aspect, with no margin.

```sh
pfx gui samples/workshop.pitofx --input pick.txt --shot picked.png
```

## Film a tape

A tape is a script with time: `wait <seconds>` holds still, the pointer glides to each point, a drag takes its time, text types at 15 characters a second.

```
wait 1
click 673 547
wait 0.6
press 720 556
move 690 551
move 660 546
release 630 541
wait 0.8
key ctrl+z
wait 1.5
```

```sh
pfx gui samples/workshop.pitofx --clip clips --tape move.tape --poster last
```

It writes three files into `clips/`, each named `<tape>-<W>x<H>-<fps>fps`: an H.264 `.master.mp4`, a `.poster.png` and a `.json` manifest (`docs/recipe-files.md`). `--fps` is 30, 60 or 120. The frames are drawn as PNGs into a staging folder, `clips/<stem>.partial/`, encoded by pito-codecs' `pcod frames` on the CPU at CRF 14, and removed; the manifest names pcod and its version. It needs `pcod` on the `PATH` and an even `--size`, and refuses in one line without them.

- **It plays on a copy:** the recipe's folder is copied into the clip folder and played there, so the tape's edits never reach your files, and the copy is removed at the end.
- **It is deterministic:** between frames it waits for every build, pick and render the window started, so the same tape gives the same frames byte for byte.

### Frames, scale and a framed window

- `--frames` keeps the PNG frames instead of encoding a master: `clips/<tape>-<W>x<H>-<fps>fps/frame-00000.png` on, and the `.json` manifest beside the folder, with each frame's tape step and line. It needs no `pcod`, takes no `--poster`, and the same tape gives the same PNG bytes.
- `--bare` draws the viewport alone: no panel, header, hint bar or timeline bar, so the viewport fills the whole frame at any size, a portrait `--size 1080x1920` or `--size 2160x3840 --scale 2` included. The pointer still draws, the viewport's own marks (the grid, the axes, a camera's frame) stay, and the tape's coordinates are the viewport's points. The keys still work: `key Space` plays a timeline, `key f` looks through its chase camera. It needs a recipe or `--start`, and doesn't go with `--framed`. `--shot` takes it too.
- `--bare --strip` keeps the timeline bar under the viewport when the scene has a `[timeline]`, so a portrait clip shows the editor's strip: play, step, loop, chase cam and the scrubber. The viewport still starts at the frame's top left, so the tape's coordinates stay the viewport's points, with the bar below them.
- `--scale <factor>` lays the window out at that many pixels a point, from 1 to 4: `--size 2160x3840 --scale 2` reads like the window at 1080x1920, text, panels and the pointer twice as large. The tape's coordinates stay in points.
- `--framed` lays the window out at `--window` (1586x992 points by default) and centres it in the `--size` frame on the editor's ground, so a 9:16 film shows the real landscape window. The window is drawn at `--scale`, or at the largest scale that fits the frame when that one doesn't: with `--size 2160x3840 --scale 2` it is drawn 2160x1351, at 1.36 pixels a point, sharp, never scaled down after drawing.
- `--json` writes one JSON line a frame on stderr, `{"stage":"frame","frame":n,"done":n+1,"total":N}`, and one line on stdout at the end, `{"stage":"done","kind":…,"frames":N,"seconds":…,"manifest":…,"files":[…]}`, with the folder (or the master and poster) and the manifest. A refusal stays one line on stderr, `pfx gui --clip: …`, exit 1.

```sh
pfx gui samples/workshop.pitofx --clip film --tape tweak.tape --frames --size 2160x3840 --scale 2 --framed --fps 60 --json
pfx gui samples/chase.pitofx --clip film --tape chase.tape --bare --size 2160x3840 --scale 2 --fps 60
```

To make a GIF of it, [pcod](https://github.com/gmrdad82/pito-codecs) converts the master:

```sh
pcod clips/move-1586x992-30fps.master.mp4 move.gif --size 1280 --fps 15
```
