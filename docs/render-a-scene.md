# Render a scene

`pfx shot` renders a scene file offline on the live renderer, the one games and the editor show. It writes numbered frames, a `manifest.json` and, when asked, a WAV that pito-codecs' `pcod frames` turns into a video. The clock is fixed: frame n is drawn at t = n / fps, and the scene's animations, movers, physics and play run on that clock, never on the wall clock. So two runs of the same scene, flags and build give the same bytes on the same GPU and driver. It opens no window. Frames go to an offscreen target on a headless GPU device, and a folder path, which would mean the editor, is refused.

## Run it

```
pfx shot <scene.toml> (--seconds S | --frames N) [--size WxH] [--fps N] [--from F] [--camera <object|node>]
         [--draft | --final] [--hour H] [--set dotted.key=value]... [--out <dir>] [--format png|exr]
         [--audio <out.wav>] [--sound <script>] [--shutter S] [--seed S] [--json]
```

| Flag | Default | What it does |
|---|---|---|
| `--seconds S` or `--frames N` | one is required | the shot's length; seconds become `round(S × fps)` frames |
| `--size WxH` | 1920x1080 | any even size from 2 to 16384 a side, within the GPU's texture limit; 2160x3840 is a vertical 4K frame |
| `--fps N` | 60 | 1 to 240 |
| `--from F` | 0 | the first frame to write; the frames before it are simulated, not written |
| `--camera <name>` | the scene's camera | follows an object, or an animated glTF node, every frame (below) |
| `--draft` or `--final` | draft | quality, below |
| `--hour H` | the scene's | the sun's hour, 0 to 24, set as `sun.hour` |
| `--set key=value` | none | any scene key, through the editor's edit path (below); repeatable |
| `--out <dir>` | `tmp/pfx/shot/<scene>-<draft or final>` under the caller's git tree | where the frames and the manifest go |
| `--format png` or `exr` | png | 8-bit sRGB PNG, or half-float linear EXR |
| `--audio <out.wav>` | `<out>/audio.wav` when a sound script is known | writes the shot's sound as a 48 kHz stereo float WAV |
| `--sound <script>` | the scene's `sound` key | the offline mix script (`*.sound.toml`) |
| `--shutter S` | off | poses the previous frame at t − S / fps for motion blur (below) |
| `--seed S` | 0 | the play session's seed |
| `--json` | off | one JSON line per frame and the summary as JSON |

Input pfx can't parse gets clap's short answer with exit 2. A refusal after the parse is one line, `pfx: …`, with exit 1: a missing scene file, a `--set` the scene's checks refuse, a camera name the scene doesn't hold, a size over the GPU's limit, or an `--out` folder holding files pfx shot did not write. A folder pfx shot wrote before (it has its `manifest.json`) is cleared of its frames and manifest and written again.

## The clock

The world ticks at a rate that puts every frame on a tick: fps × k, with k the smallest whole number that keeps the rate at or above the scene's `[physics] rate` (60 by default). With `--shutter`, k is also chosen so that S × k is a whole number, which puts t − S / fps on a tick too. At 60 fps with the default physics rate the world ticks at 60 Hz, at 24 fps at 72 Hz, and at 60 fps with `--shutter 0.5` at 120 Hz. Before frame n is posed, the clock is moved to n / fps plus one microsecond, so a frame that lands exactly on a tick includes it. Poses are taken at the latest tick, so they are exactly the clip's pose at t. The rate is in the manifest as `tick_rate`.

Animations play through the play session as in a game ([Animation](animation.md#playing-it-pfx-play)). That covers glTF node animation, skins and morph weights started by `[object.animation]`, sprite clips, movers, physics and camera rigs. A scene whose motion is baked into a glTF file (one node animation, keys at n / fps, `LINEAR`) therefore plays exactly what was baked.

## The camera

`--camera <name>` places the camera every frame. The name is looked up first as an object, then as a node of an object's rigged mesh. `chase` also finds `camera.chase`. An object's camera sits at the object's origin, looking down its −Z with +Y up. A node's camera sits at the object's matrix times the node's animated pose, the same place the node's parts are drawn. When the node carries a glTF perspective camera, its `yfov` becomes the vertical field of view and its `znear` the near plane. An `aspectRatio` that differs from the frame's by more than 1% leaves a warning in the manifest and keeps the vertical field of view. Depth of field follows the scene camera's settings. When `[camera] focus_keys = [[t, metres], ...]` is set, the focus distance is interpolated linearly from those keys at each frame's t, holding the first and last (`DepthOfField::distance_at`, the helper `Staged::frame` uses for its lens).

## Draft and final

- **Draft** (the default) uses the scene's settings, with the sun's shadow map at half its resolution (never below 512).
- **Final** keeps the scene's full shadow resolution.

Frame N of a shot started with `--from N` is the same, byte for byte, as frame N of a run from 0. The live renderer meters exposure automatically: it starts at 1.0 and adapts towards the frame's mean luminance over about half a second. That state depends on every frame before, so under auto exposure, the renderer's default, both draft and final render frames 0 to N − 1 first without writing or reading them back. A warm frame costs its GPU time and the simulation, about a draft frame without its readback and PNG: 4.8 ms each for the room at 2160x3840 (below), so `--from 300` adds about 1.5 s. After each frame the shot waits for the GPU, exposure meter readback included, so the next frame's exposure never depends on when a readback lands. When exposure is fixed, final renders only the up to 8 frames before `--from` to settle TAA's history, and draft none; frames then match a full run once TAA has settled, not bit for bit. The manifest records `warm_frames`. With `--from 0` there is nothing before the first frame, and TAA settles over the first frames as it does live.

The live renderer's TAA jitter follows the scene clock (its sample index is round(t × 60)). Rendering one frame time several times therefore adds no samples, and final does not supersample. A larger internal size would change every pixel-sized effect and cost four times the GPU time. The manifest records `shadow_resolution` and `warm_frames`.

## Motion blur's previous pose

By default each frame's previous pose (`Instance::previous_model`, the skin palettes, the morph weights and the camera's previous view-projection) is the last frame's, as live. TAA stays intact, and the blur pass scales that whole-frame motion by `[motion_blur] shutter`. `--shutter S` instead presents the scene at t − S / fps just before t, so the previous pose is the true pose at the shutter's opening: a fast-spinning wheel or a curved path blurs along its real arc, not along the chord between two whole frames. It trades away TAA stability. The renderer's history check sees a previous view-projection that is not the last rendered frame's, so in a shot whose camera moves TAA restarts every frame, and the shot shows the jittered single samples.

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

The out folder holds `000000.png`, `000001.png`, … (or `.exr`), numbered from 0 whatever `--from` is, in the order `pcod frames` reads them, and `manifest.json`:

| Key | |
|---|---|
| `shot` | the manifest's schema, 1 |
| `frames`, `from`, `fps`, `size`, `seconds` | what was written; `seconds` is frames / fps |
| `format` | `png` or `exr` |
| `wav`, `audio` | the WAV's absolute path or null; its source (`script` or `silence`), script, rate, channels and a note |
| `scene`, `scene_sha256`, `rev` | the scene file, its SHA-256, and the Git HEAD of its folder (`+dirty` when it has changes), or null |
| `pfx` | the engine's version |
| `quality`, `shadow_resolution`, `warm_frames`, `tick_rate`, `shutter` | how it rendered |
| `camera`, `hour`, `seed`, `sets` | what was asked; `camera` is the object or `object/node` followed |
| `gpu_ms` | `p50` and `p99` of the GPU time of the written frames, or null on a GPU without timestamps |
| `wall_ms` | `total`, and `per_frame`: the total over the frames, readback and file writing included |
| `out`, `pcod`, `adapter`, `warnings` | the folder, the exact `pcod frames <dir> [--audio <wav>] --fps <fps> <dir>.mp4` command, the GPU, and any warnings |

PNG frames are the finished frame, tone-mapped and sRGB-encoded, as 8-bit RGB. EXR frames hold the same finished frame decoded to linear light (Rec. 709 primaries, D65), as half floats with alpha. They are not the scene-linear radiance before tone mapping. With `--json`, each frame prints `{"frame", "of", "number", "time", "gpu_ms", "camera": {"at", "look_at"}, "wall_ms"}` as it is read back, then the manifest prints as the last line. Without it, pfx prints the folder and the `pcod` command.

Frames are read back one at a time and written by up to 8 threads while the GPU draws the next frame. The pacer takes turns with other GPU jobs as pgpu asks ([Pacing](pacing.md)) but caps no frame rate, so run long shots under `pgpu run --class render`.

## What it costs

`crates/load/tests/scenes/room.scene.toml` at 2160x3840, 60 fps, 120 frames, PNG, release build, on an AMD Radeon RX 9060 XT under Linux, under `pgpu run --class bench` beside two other jobs:

| | GPU ms per frame, p50 / p99 | wall ms per frame | shadow map |
|---|---|---|---|
| `--draft` | 4.76 / 4.82 | 34.4 | 2048 |
| `--final` | 4.79 / 4.83 | 32.1 | 4096 |

The wall time is the whole run divided by its frames: device start, staging, the 33 MB readback of each half-float frame and its PNG encoding (about 540 KB a frame) included. It is about seven times the GPU time, so a shot's wall time is set by readback and writing, not by the renderer. In the room, a few blocks, the shadow map's size barely moves the GPU time. The draft's half-size map saves more in a scene with many casters.

## Not covered

- Supersampling and TAA warm-up before frame 0.
- The scene-linear EXR before tone mapping.
- Interpolating poses between ticks: the clock puts every frame on a tick instead.
