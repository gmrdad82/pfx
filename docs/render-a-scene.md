# Render a scene

`pfx shot` renders a scene's `.pitofx` offline on the live renderer (a recipe's `.pitofx` draws the scene its build holds; a `.toml` is refused with one line naming `pfx scene migrate`, [.pitofx files](pfx-file.md#the-user-file)), the one games and the editor show. It writes numbered frames, a `manifest.json` and, when asked, a WAV that pito-codecs' `pcod frames` turns into a video. The clock is fixed: frame n is drawn at t = n / fps, and the scene's animations, movers, physics and play run on that clock, never on the wall clock. So two runs of the same scene, flags and build give the same bytes on the same GPU and driver. It opens no window. Frames go to an offscreen target on a headless GPU device, and a folder path, which would mean the editor, is refused.

## Run it

```
pfx shot <scene.pitofx> (--seconds S | --frames N) [--size WxH] [--fps N] [--from F] [--camera <object|node>]
         [--draft | --final] [--hour H] [--set dotted.key=value]... [--out <dir>] [--format png|png16|exr] [--dither on|off]
         [--audio <out.wav>] [--sound <script>] [--shutter S] [--exposure auto|fixed] [--seed S] [--json]
```

| Flag | Default | What it does |
|---|---|---|
| `--seconds S` or `--frames N` | one is required | the shot's length; seconds become `round(S × fps)` frames |
| `--size WxH` | 1920x1080 | any even size from 2 to 16384 a side, within the GPU's texture limit; 2160x3840 is a vertical 4K frame |
| `--fps N` | 60 | 1 to 240 |
| `--from F` | 0 | the first frame to write; the frames before it are simulated, not written, and the files still number from `000000` (file k is frame F + k) |
| `--camera <name>` | the scene's camera | follows an object, or an animated glTF node, every frame (below) |
| `--draft` or `--final` | draft | quality, below |
| `--hour H` | the scene's | the sun's hour, 0 to 24, set as `sun.hour` |
| `--set key=value` | none | any scene key, through the editor's edit path (below); repeatable |
| `--out <dir>` | `tmp/pfx/shot/<scene>-<draft or final>` under the caller's git tree | where the frames and the manifest go |
| `--format png`, `png16` or `exr` | png | 8-bit sRGB PNG, 16-bit sRGB PNG, or half-float linear EXR |
| `--dither on` or `off` | on | dithers the 8-bit PNG frames against banding (below); png16 and EXR frames are never dithered |
| `--audio <out.wav>` | `<out>/audio.wav` when a sound script is known | writes the shot's sound as a 48 kHz stereo float WAV |
| `--sound <script>` | the scene's `sound` key | the offline mix script (`*.sound.toml`) |
| `--shutter S` | off | poses the previous frame at t − S / fps for motion blur (below) |
| `--exposure auto` or `fixed` | auto | auto meters each frame; fixed holds the exposure at 1.0, as `pfx shot --still` and `pfx render --recipe` draw a recipe |
| `--seed S` | 0 | the play session's seed |
| `--json` | off | one JSON line per frame and the summary as JSON |

Input pfx can't parse gets clap's short answer with exit 2. A refusal after the parse is one line, `pfx: …`, with exit 1: a missing scene file, a `--set` the scene's checks refuse, a camera name the scene doesn't hold, a size over the GPU's limit, an `--out` folder holding files pfx shot did not write, or a degenerate camera at a drawn frame (a `look_at` equal to its `at`, an `up` within 1e-4 of the view direction or of zero length, a field of view outside 0 to 180 degrees, a near plane at or below 0, a far plane not beyond the near one): `pfx: <scene>: at frame N the scene's camera looks from [..] at [..], no direction to look along; pfx shot renders no degenerate view`. A folder pfx shot wrote before (it has its `manifest.json`, or holds only numbered `NNNNNN.png` or `.exr` frames and `audio.wav`, as an interrupted run leaves it) is cleared of every numbered frame and its manifest and written again, so no frame of an earlier, longer run survives.

## Progress and stderr

When stderr is a terminal, `pfx shot` keeps one line on it updated in place with `\r`: `pfx shot [#####-------------------] frame 30/120  35.2 fps  ETA 3s`. The rate counts the frames since the first one written, so device start and staging don't drag it down; the line is redrawn at most ten times a second and on the last frame, and cleared before the summary prints or an error does. It is left out when stderr is piped or redirected, when `NO_COLOR` is set to anything non-empty, when `TERM` is `dumb`, and with `--json`, which keeps stdout to its JSON lines and stderr quiet. `pfx film` and `pfx bench` show the same line (`pfx film`, `pfx bench`) under the same rules.

Mesa's radv driver prints `WARNING: radv is not a conformant Vulkan implementation, testing use only.` at device creation on hardware it has no conformance submission for. The `pfx` binary sets `MESA_VK_IGNORE_CONFORMANCE_WARNING=1`, the switch Mesa's `vk_warn_non_conformant_implementation` reads, for its own process at start, before any GPU instance exists, and only when that variable isn't already set; set it to `0` to see the warning again. Programs pfx starts inherit it. Nothing else on stderr is filtered. The library crates a game links never set it: a game decides for itself.

## The clock

The world ticks at a rate that puts every frame on a tick: fps × k, with k the smallest whole number that keeps the rate at or above the scene's `[physics] rate` (60 by default). With `--shutter`, k is also chosen so that S × k is a whole number, which puts t − S / fps on a tick too. At 60 fps with the default physics rate the world ticks at 60 Hz, at 24 fps at 72 Hz, and at 60 fps with `--shutter 0.5` at 120 Hz. Before frame n is posed, the clock is moved to n / fps plus one microsecond, so a frame that lands exactly on a tick includes it. Poses are taken at the latest tick, so they are exactly the clip's pose at t. The rate is in the manifest as `tick_rate`.

Animations play through the play session as in a game ([Animation](animation.md#playing-it-pfx-play)). That covers glTF node animation, skins and morph weights started by `[object.animation]`, sprite clips, movers, physics and camera rigs. A scene whose motion is baked into a glTF file (one node animation, keys at n / fps, `LINEAR`) therefore plays exactly what was baked.

## The camera

`--camera <name>` places the camera every frame. The name is looked up first as an object, then as a node of an object's rigged mesh. `chase` also finds `camera.chase`. `object/node`, the form the manifest records in `camera`, looks the node up only in that object's rig and its glTF camera only in that object's mesh, so `--camera street/camera.chase` (or `street/chase`) renders the shot the manifest describes again. An object's camera sits at the object's origin, looking down its −Z with +Y up. A node's camera sits at the object's matrix times the node's animated pose, the same place the node's parts are drawn. When the node carries a glTF perspective camera, its `yfov` becomes the vertical field of view and its `znear` the near plane. An `aspectRatio` that differs from the frame's by more than 1% leaves a warning in the manifest and keeps the vertical field of view. Depth of field follows the scene camera's settings. Every `keys` table of the scene ([scenes.md](scenes.md#keys-over-time)) is sampled at each frame's t, the frame number over `--fps`: keyed `fov`, `fstop` and `focus` (`focus_keys`) move the camera's lens, and keyed lights, sun hour, haze, finish and sky change the frame as `Staged::frame` does in the window, so the shot and the window agree.

## Draft and final

- **Draft** (the default) uses the scene's settings, with the sun's shadow map at half its resolution (never below 512).
- **Final** keeps the scene's full shadow resolution.

Frame N of a shot started with `--from N` is the same, byte for byte, as frame N of a run from 0. The live renderer meters exposure automatically: it starts at 1.0 and adapts towards the frame's mean luminance over about half a second. That state depends on every frame before, so under auto exposure, the renderer's default, both draft and final render frames 0 to N − 1 first without writing or reading them back. A warm frame costs its GPU time and the simulation, about a draft frame without its readback and PNG: 4.8 ms each for the room at 2160x3840 (below), so `--from 300` adds about 1.5 s. After each frame the shot waits for the GPU, exposure meter readback included, so the next frame's exposure never depends on when a readback lands. Auto exposure takes back a change of the whole frame's brightness: a sky at a third of its `intensity` moves the sky of a test scene from 135.9 to 56.2 in the first frame, and after two seconds only from 157.2 to 138.4, while the floor brightens from 159.4 to 184.8. `--exposure fixed` holds it at 1.0, so the frame moves with the sky (135.9 to 56.2 after two seconds too) and matches the recipe's own views; the manifest records `exposure`. When exposure is fixed, final renders only the up to 8 frames before `--from` to settle TAA's history, and draft none; frames then match a full run once TAA has settled, not bit for bit. The manifest records `warm_frames`. With `--from 0` there is nothing before the first frame, and TAA settles over the first frames as it does live.

The live renderer's TAA jitter follows the scene clock (its sample index is round(t × 60)). Rendering one frame time several times therefore adds no samples, and final does not supersample. A larger internal size would change every pixel-sized effect and cost four times the GPU time. The manifest records `shadow_resolution` and `warm_frames`.

## Motion blur's previous pose

By default each frame's previous pose (`Instance::previous_model`, the skin palettes, the morph weights and the camera's previous view-projection) is the last frame's, as live. TAA stays intact, and the blur pass scales that whole-frame motion by `[motion_blur] shutter`. `--shutter S` instead presents the scene at t − S / fps just before t, so the previous pose is the true pose at the shutter's opening: a fast-spinning wheel or a curved path blurs along its real arc, not along the chord between two whole frames. A `[motion_blur] shutter` above 1 is a stylised multiplier on top of it: the pass scales the true velocity by that shutter, so `--shutter 1` with a key of 2 streaks two frames' worth along the real arc; a key of 1 or below leaves the velocity as it is. It trades away TAA stability. The renderer's history check sees a previous view-projection that is not the last rendered frame's, so in a shot whose camera moves TAA restarts every frame, and the shot shows the jittered single samples.

## Sound

When `--sound` names a script, or the scene names one with `sound = "x.sound.toml"`, the WAV holds that script's offline mix for the shot's length, `round(frames × 48000 / fps)` stereo frames. A pfx built without the `shot-sound` feature, which links pfx-sound's offline mix, writes silence of the same length and says so in the manifest's `audio.note`. `--audio` alone, with no script, writes silence too.

## Settable keys

Each `--set dotted.key=value` goes through the same scene-edit path as the editor ([Scenes](scenes.md)): the edit is applied to the scene's files in memory, checked as a save would be, and never written to disk. The value is read as TOML (`0.4`, `true`, `[1, 2, 3]`, `"text"`); anything that doesn't parse as TOML is a string. The first segments pick the entry:

- `object.<name>.…`, `light.<name>.…`, `emitter`, `mover`, `rig`, `tiles`, `particles.<name>.…`: a named entry of that list.
- `mesh.<name>.…`, `content`, `text`, `sound.<name>.…`, `materials.<name>.…`: a named entry of that table.
- `sun.…`, `sky`, `haze`, `camera`, `finish`, `trace`, `plates`, `physics`, `flare`, `motion_blur`: that section, in whichever file of the scene holds it.
- Anything else, numbered list entries (`particles.0.rate`) included: that key path in the root scene file.

Every key the scene format knows is settable, including keys added after this page was written. For example, `--set haze.shafts=0.5 --set finish.lut_amount=0.8 --set light.headlight.cone=[12,28]`.

## Out

The out folder holds `000000.png`, `000001.png`, … (or `.exr`), numbered from 0 whatever `--from` is, in the order `pcod frames` reads them: file k is the scene's frame `from + k`, so `--from 300 --seconds 5` at 60 fps writes `000000.png` as frame 300 and `000299.png` as frame 599. It also holds `manifest.json`:

| Key | |
|---|---|
| `shot` | the manifest's schema, 1 |
| `frames`, `from`, `fps`, `size`, `seconds` | what was written; `seconds` is frames / fps |
| `first_number` | the scene frame `000000` holds, equal to `from`; file k is frame `first_number + k` |
| `format` | `png`, `png16` or `exr` |
| `dither` | true when the PNG frames were dithered |
| `wav`, `audio` | the WAV's absolute path or null; its source (`script` or `silence`), script, rate, channels and a note |
| `scene`, `scene_sha256`, `rev` | the scene file, its SHA-256, and the Git HEAD of its folder (`+dirty` when it has changes), or null |
| `pfx` | the engine's version |
| `quality`, `shadow_resolution`, `warm_frames`, `tick_rate`, `shutter`, `exposure` | how it rendered |
| `camera`, `hour`, `seed`, `sets` | what was asked; `camera` is the object or `object/node` followed |
| `gpu_ms` | `p50` and `p99` of the GPU time of the written frames, or null on a GPU without timestamps |
| `wall_ms` | `total`, and `per_frame`: the total over the frames, readback and file writing included |
| `out`, `pcod`, `adapter`, `warnings` | the folder, the exact `pcod frames <dir> [--audio <wav>] --fps <fps> <dir>.mp4` command, the GPU, and any warnings |

PNG frames are the finished frame, tone-mapped and sRGB-encoded, as 8-bit RGB, dithered where they are rounded to 8 bits: blue noise of about one level in the encoded space, laid at an offset from `--seed`, the same offset every frame, so a static region is byte for byte the same from frame to frame and a smooth sky shows fine grain instead of bands ([the output dither](recipe-render.md#the-output-dither) has the noise, the measurements and why the pattern does not move). `--dither off` rounds plainly. `--format png16` writes the same finished frame as 16-bit RGB (big-endian, sRGB tagged), undithered, for a master; pcod reads such a folder as it reads 8-bit frames, and a 10-bit encode takes it as [16-bit frames](recipe-render.md#16-bit-frames) says. EXR frames hold the same finished frame decoded to linear light (Rec. 709 primaries, D65), as half floats with alpha. They are not the scene-linear radiance before tone mapping. With `--json`, each frame prints `{"frame", "of", "number", "time", "gpu_ms", "camera": {"at", "look_at"}, "wall_ms"}` as it is read back, then the manifest prints as the last line. Without it, pfx prints the folder and the `pcod` command.

Frames are read back one at a time and written by up to 8 threads while the GPU draws the next frame. The pacer takes turns with other GPU jobs as pgpu asks ([Pacing](pacing.md)) but caps no frame rate, so run long shots under `pgpu run --class render`.

## What it costs

The neutral room (`crates/load/tests/scenes/room.scene.toml`, imported once into `room.pitofx`) at 2160x3840, 60 fps, 120 frames, PNG, release build, on an AMD Radeon RX 9060 XT under Linux, under `pgpu run --class bench` beside two other jobs:

| | GPU ms per frame, p50 / p99 | wall ms per frame | shadow map |
|---|---|---|---|
| `--draft` | 4.76 / 4.82 | 34.4 | 2048 |
| `--final` | 4.79 / 4.83 | 32.1 | 4096 |

The wall time is the whole run divided by its frames: device start, staging, the 33 MB readback of each half-float frame and its PNG encoding (about 540 KB a frame) included. It is about seven times the GPU time, so a shot's wall time is set by readback and writing, not by the renderer. In the room, a few blocks, the shadow map's size barely moves the GPU time. The draft's half-size map saves more in a scene with many casters.

## Not covered

- Supersampling and TAA warm-up before frame 0.
- The scene-linear EXR before tone mapping.
- Interpolating poses between ticks: the clock puts every frame on a tick instead.
