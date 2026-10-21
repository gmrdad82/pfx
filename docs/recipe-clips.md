# Recipe clips

A camera with a path, or an object turning under a still camera, turns into frames with `pfx clip`, drawn live or path-traced, and into a video when an encoder is there.

## Give a camera a path

`[camera.path]` belongs to the `[[camera]]` written last above it. An orbit turns about the vertical line through `target`:

```toml
[[camera]]
name = "orbit"
position = [0.0, 0.4, 1.0]
target = [0.0, 0.1, 0.0]
fov_y_deg = 34.0

[camera.path]
kind = "orbit"
target = [0.0, 0.1, 0.0]
turns = 1.0
rise_deg = 8.0
```

Keys move the camera from pose to pose over time `t` from 0 to 1:

```toml
[camera.path]
kind = "keys"
ease = "smooth"

[[camera.path.keys]]
t = 0.5
position = [0.4, 0.3, 0.5]
target = [0.0, 0.03, -0.02]

[[camera.path.keys]]
t = 1.0
position = [0.0, 0.6, 0.1]
target = [0.0, 0.0, -0.05]
```

Without a key at 0, the camera's own pose is the key at 0. `ease` is `linear` (the default) or `smooth`.

## Make the frames

```sh
pfx clip samples/workshop.pitofx --camera orbit --frames 120 --fps 30 --size 1280x800 --out orbit
pfx clip samples/workshop.pitofx --camera orbit --frames 120 --fps 30 --size 1280x800 --out traced --trace --spp 32
```

The first draws each frame on the live renderer; `--trace` path-traces them (64 samples a frame by default). Frames are written as `frame-0000.png`, `frame-0001.png`… and each finished frame is printed on stdout.

- **An orbit loops:** frame `i` of `N` is at `i / N`, so a whole number of turns repeats no frame. Keys end on their last key.
- **A run resumes:** a frame is written whole or not at all, and a second run into the same folder makes only what is missing (see [Stop and resume](#stop-and-resume)). A run with different settings into that folder is refused, so frames never mix.
- **Part of a clip:** `--range A..B` makes only frames A to B, both kept, numbered and seeded as in a whole run, into the same folder a whole run uses, so ranges made on several machines join by copying their frames together (`docs/jobs.md`). A long clip runs detached, pausable and resumable as a `pfx job` there.
- **Same input, same bytes:** each frame depends on its pose, the recipe and the flags only, never on the frames rendered before it in the same run.

## Denoise and the raw passes

```sh
pfx clip samples/workshop.pitofx --camera orbit --frames 24 --fps 24 --size 1280x800 --out raw --trace --spp 8 --denoise off
pfx clip samples/workshop.pitofx --camera orbit --frames 24 --fps 24 --size 1280x800 --out clean --trace --spp 8 --passes clean-passes
```

- **`--denoise on|off`** (with `--trace`, default `on`): `on` puts every traced frame through pfx's denoiser before the recipe's finish, as `pfx render --recipe` does for a `.png`; `off` writes the raw trace through the same finish. The same flag and wording as `pfx render --recipe` ([recipe-render.md](recipe-render.md)).
- **`--passes <dir>`** (with `--trace`) also writes, for every frame, the three layers the denoiser takes, as the tracer averaged them, before any denoise or finish: `frame-0007.color.exr` (the beauty, linear radiance), `frame-0007.albedo.exr` (the first surface's albedo) and `frame-0007.normal.exr` (its shading normal in world space, averaged over the samples and normalised, 0 where no sample hit), named after the frame's PNG. Each is a single-layer OpenEXR named for its pass (`color`, `albedo`, `normal`) with `R`, `G`, `B` and `A` channels of 32-bit floats, lossless, the values the denoiser reads bit for bit. Alpha is the tracer's own and is kept, the average over the pixel's samples of what each sample's first event wrote: 1 for a surface, 0 for a miss. Fog's first event counts as a hit, with fog's albedo and the reverse of the ray as its normal, and a shadow catcher writes its shadow into the albedo's alpha, so the albedo's alpha is not plain coverage there. The colour's alpha is 1 unless the tracer's transparent film is on, where it is the albedo's alpha. (`.exr` written by `pfx render --recipe` still sets alpha to 1 everywhere, as half floats.) The passes are written whole, before their frame's PNG, so a finished frame has its passes; they go to a folder of their own, never `--out`. They are the same with `--denoise on` and `off`. The folder is not part of the line in `clip.txt`, so a resume passes the same `--passes`, and ranges joined from several machines copy their passes folders together as they copy their frames. A frame's three passes take 48 bytes a pixel before compression, about 400 MB at 3840×2160, and lossless compression saves little on noisy low-sample floats: a 3840×2160 frame at 8 samples writes about 300 MB of passes, which adds about 0.8 s to a run of that one frame that takes 3.4 s without them on a Radeon RX 9060 XT.

What the denoiser does to a traced frame is in [denoise.md](denoise.md#a-traced-frame).

Both flags enter `clip.txt` when they leave the default (`denoise off`, `passes`), so a folder made with one never resumes with the other, and frames never mix.

## Stop and resume

A long clip survives a stop, a crash, a reboot or the GPU queue taking the card away: run the same line into the same `--out` and it carries on.

- **Whole files only.** Every frame, the `--video`, `clip.txt` and `frames.json` go to a hidden `.<name>.partial.<ext>` beside the file, are synced to disk and then renamed into place. A run sweeps any leftover `.partial` file before it starts.
- **`frames.json`** lists each finished frame with its number, size and sha256, rewritten whole after every frame. On resume each frame on disk is read against it: a frame that is missing, cut short or doesn't match its hash is made again. A frame the ledger doesn't know yet (a folder from an older pfx, or a stop between the rename and the ledger) is decoded whole first and only then recorded. Several `--range` runs into one folder share the ledger under `frames.lock`: each verifies, sweeps and redoes only its own frames, and a range checks the frame before its first missing one only when that frame is finished and recorded there.
- **The previous frame is made again first.** Before continuing at frame K, frame K−1 is rendered once more and compared byte for byte with the one on disk. When they match the run carries on; when they don't (another pfx build, another GPU driver, or state carried between frames), it stops with one line naming the frame, records the result under `checks` in `frames.json`, and renders nothing more, so old and new frames never mix. A traced check is paid once per build and GPU: `frames.json` keeps with each check the sha256 of the pfx binary that made it and the adapter, backend and driver it ran on, and once a check made by that same binary on that same GPU and driver has matched (and none by it has differed), a later run or `--range` into the folder skips rendering its frame K−1 again and says which frame proved it. A clip made one frame a run otherwise renders every frame twice. `--rerender-from <frame>` removes that frame and every later one, from the folder and from the ledger, and makes them again without the check.
- **Checkpoints inside a frame.** `--checkpoint <every>` (with `--trace`) saves the frame's accumulated samples to `.frame-NNNN.checkpoint` every so many minutes (`--checkpoint 10`) or samples (`--checkpoint 64spp`). The file holds the tracer's sums, its three output layers, the sample index and, for adaptive sampling, the rounds reached and the per-pixel noise estimates, so a resumed frame continues at the next sample and ends on the same bytes as an uninterrupted one. A checkpoint is kept only for the render that made it (the line, the recipe's sha256, the sha256 of each HDRI it keeps beside itself, and the pfx version); any other is removed. It is deleted once its frame is written. At 3840×2160 a checkpoint is about 1.1 GB.
- **What it says.** A resumed run prints `resuming: 37 of 120 frames done, starting at frame 37` (with `, from its checkpoint at 96 samples` when one is used), a line with the frames verified and redone, and the K−1 check's result.

`--json` prints the same on stderr as JSON lines, one object a line:

| `stage` | fields |
|---|---|
| `resume` | `done`, `of`, `from`, `checkpoint_samples`, `verified`, `adopted`, `redone` (`frame`, `why`), `check`, `rerender_from` |
| `check` | `frame`, `result` (`same`, `differs`, or `proven` with `by`, the frame whose check proved this build) |
| `path` | `frame`, `path`: the adapter, ray queries and BVH the frame traces on, the fields of `pfx render --recipe --progress json`'s `path` line ([recipe-files.md](recipe-files.md#pfx-render---recipe---progress-json)); printed before the first traced frame and again only when it changes. Plain output prints its `line` as `pfx clip: tracing on …` |
| `trace` | `frame`, `of`, `samples` (the mean so far under adaptive sampling), `spp`, `fraction` of the frame, `eta_s` for the run |
| `checkpoint` | `checkpoint` (its path), `frame`, `at_samples` |
| `frame` | `frame`, `of`, `done`, `out`, `eta_s`; with `--trace` also the frame's cost: `submissions` (GPU submissions), `longest_ms` (the longest one), `gpu_ms` (their GPU time), `bottom_builds` and `top_builds` (the BVH builds this frame made: a restaged trace builds every mesh and its top level, a kept one refits only the top level) |

`trace` lines come at most once a second, plus the first of every frame.

## Turn an object on a turntable

`--spin` turns an object, and every object parented to it, about the vertical line through its origin, while the camera and the lights stay still. Give the figure a hidden root to turn about, and a camera without a path:

```toml
[[object]]
name = "body"
kind = "box"
size = [0.01, 0.01, 0.01]
hidden = true

[[object]]
name = "figure"
kind = "mesh"
path = "figurine.glb"
parent = "body"

[[camera]]
name = "front"
position = [0.0, 0.12, 0.45]
target = [0.0, 0.08, 0.0]
fov_y_deg = 30.0
```

```sh
pfx clip figurine.pitofx --camera front --spin body --frames 96 --fps 24 --size 1080x1080 --out turn --video turn.mp4
pfx clip figurine.pitofx --camera front --spin body --turns -0.5 --frames 48 --fps 24 --size 1080x1080 --out half --trace --spp 64
```

Frame `i` of `N` turns `body` by `turns × 360° × i / N` (`--turns` is 1 by default, and may be fractional or negative to turn the other way), so a whole number of turns loops without repeating a frame. A camera with a path is refused with `--spin`, and so is `--timeline`. The recipe is read and built once: each frame only moves the turned objects.

## Film the timeline

```sh
pfx clip chase.pitofx --camera chase --timeline --size 1080x1920 --out chase
```

`--fps` takes a whole number, `num/den` such as `24000/1001`, or `23.976`, `29.97`, `47.952`, `59.94`, `119.88`, read as the exact NTSC ratios; any other decimal is refused naming the ratio to use. A ratio reaches pcod as the exact ratio (`--fps 24000/1001`), to `.mp4` or `.webm` alike.

A recipe with a `[timeline]` (the recipe guide's "Timeline") films with `--timeline` in place of `--frames` and `--fps`: frames 0 to the timeline's last at its `fps`, each the scene posed at exactly `n × den / num` seconds, so a resumed folder's frames match a full run's byte for byte, through `--camera`, which the timeline moves when it keys that camera. `--video` encodes at the timeline's `fps`.

## Play time at another speed

```toml
[timeline.map]
keys = [[0.0, 0.0], [2.0, 2.0], [4.0, 3.1], [6.0, 4.2]]
```

```sh
pfx clip chase.pitofx --camera chase --timeline --trace --size 1080x1920 --out ramp
pfx timeline chase.pitofx --times > ramp.txt
pfx clip chase.pitofx --camera chase --times ramp.txt --trace --size 1080x1920 --out ramp --range 0..71
```

A time map films each output frame at a source time of its own, so a speed ramp or slow motion renders new in-between poses instead of repeating source frames.

- **`--times <file>`** films the recipe's `[timeline]` at the times in the file: one time in seconds a line, output frame `i` at the `i`-th, or a CSV whose first line is a header and whose `time` column holds the seconds (`--column <name>` takes another). Every time lies within the timeline, 0 to its length; blank lines end the file and are refused inside it. The clip has as many frames as the file has times, at `--fps`, the timeline's `fps` when absent. The camera, rig and keys pose as `--timeline` poses them.
- **`[timeline.map]`** (the recipe guide's "Timeline") gives `--timeline` its times: output frame `n` is at output time `n × den / num` of the timeline's `fps`, and its source time is the map's curve there, from frame 0 to the last output key. `pfx timeline <x.pitofx> --times` prints those times, one a line, as the `--times` file that films the same frames.
- **Numbered, seeded and resumed as `--range`:** output frame `i` is `frame-<i>.png` whatever its source time, `--range A..B` makes lines A to B of the same file into the same folder, and the checkpoints, the ledger and the frame-before check work as they do for any clip, so parts rendered apart join with no gap or renumbering. A traced frame's seed is the recipe's seed, the same for every frame (as in every clip), so a time-mapped frame samples exactly as the frame of the same index in a `--range`, and a `--times` file of the timeline's own times (what `pfx timeline --times` prints for a timeline without a map) makes `--timeline`'s frames byte for byte. What the timeline animates from its `seed` (drift, shake) follows the source time.
- **The manifest** (`clip.txt`) records the map: `time map smooth: 0 -> 0, 2 -> 2, …` for `[timeline.map]`, or `times sha256 <hex>, <n> times` (and the CSV column) for a file, with the fps. Parts join only from the same keys or the same bytes, wherever the file sits; another map into that folder is refused, as any other line is.
- **Motion blur follows the slope.** The slope at frame `i` is the source seconds per output second, `(t[i+1] − t[i−1]) / 2` frames apart (one-sided at the first and last frame, 1 for a single time), rounded to a millionth, so a steady map has a slope of exactly 1. A traced frame's shutter in source time is `[motion_blur] shutter × slope / fps`, placed by `position` as always: 0.55x slow motion blurs 0.55x as far, a freeze (slope 0) is traced sharp, and reverse (a negative slope) keeps the shutter's length and runs it backwards from the frame, so `position = "start"` streaks ahead along the reversed motion, as it does forward. A slope of 1 at the timeline's `fps` is `--timeline`'s own shutter, bit for bit. The live renderer's blur is the motion since the previous output frame, which already scales with the map: a freeze has none and reverse streaks the other way.

## List what a traced frame holds

`--dump-scene <file.jsonl>` (with `--trace`) writes every instance each traced frame holds, so an object that is missing from a frame can be named: dropped in staging, placed wrong, hidden in the hardware top level, or held and never hit. It reads the trace after the frame is written and changes no pixel.

```sh
pfx clip chase.pitofx --camera chase --timeline --trace --size 540x960 --spp 16 --range 240..240 --out tmp/list --dump-scene tmp/list-scene.jsonl
jq -c '.list[] | select(.object == "lamp") | {name, traced, mesh, instanced, bounds, squash, one_sided, tlas_mask, tlas_in_place}' tmp/list-scene.jsonl
```

- **When it is written.** The file is emptied when the run starts and gets one JSON line, flushed, after each frame this run renders (the K−1 check included, see [Stop and resume](#stop-and-resume)). Frames already finished in `--out` are not rendered, so they are not listed: the run says how many it leaves out, and a fresh `--out` lists them all. A stopped run keeps the lines of its finished frames.
- **Each line:** `recipe` (its path), `camera`, `frame`, `instances` (the entries in `list`), `traced` (those with triangles in the trace), `meshes` (one `{ mesh, triangles, instances }` per trace mesh) and `list`, one entry per instance in staging order.
- **Every entry has:**

| key | meaning |
|---|---|
| `index` | the instance's place in the trace |
| `name` | the recipe piece, or `particle.N` for the frame's Nth sprite and `lettering.N` for the Nth text carrier |
| `object`, `material` | the recipe object and material, `null` for a sprite or a text carrier |
| `traced` | `true` when the instance has triangles in the trace; `false` means it has none (an empty piece), not that it was placed and then dropped |
| `mesh` | the trace's mesh id, `null` when not traced |
| `shared` | the staging fit group: pieces of one group are one mesh placed by a rigid or scaled fit |
| `triangles` | the trace mesh's triangle count |
| `instanced` | every instance on that trace mesh, other objects that fit it by scale included, so it is not a count of copies of one file |
| `moving`, `deforming`, `sprite` | whether the instance moves over the shutter, deforms, or is a particle sprite |

- **Present only sometimes:**

| keys | when | meaning |
|---|---|---|
| `two_sided` | pieces | what the recipe or the file asked for |
| `base`, `alpha`, `alpha_mode`, `alpha_cutoff`, `transmission` | a material was found for the piece | the material's values |
| `one_sided`, `shadow_only`, `casts_shadow`, `cutout`, `clipped`, `cutoff` | traced | what the tracer applies: `one_sided` drops a camera ray that hits the back of a face, `shadow_only` hides it from camera rays, `cutout` tests the alpha against `cutoff` (a card's own cutoff where it sets one), `clipped` cuts it at its clip planes |
| `bounds` | traced | `{ lo, hi }` in world metres, swept over the shutter for a mover |
| `determinant`, `squash` | traced | of the instance's transform: `squash` is the determinant's size over the product of the column lengths, 1 for a turned or scaled copy and near 0 for a skewed or flattened fit; a negative determinant is a mirrored copy |
| `tlas_mask`, `tlas_custom`, `tlas_in_place`, `tlas_opaque`, `hardware_mover` | the frame traced on ray queries | the instance's hardware top-level slot: its mask (255 drawn, 1 for a still sprite, 0 hidden from the top level as a mover, `null` when the slot is empty), its custom index, whether the slot holds exactly the transform and record the trace places the instance by, whether it took the opaque BLAS the hardware commits by itself (otherwise the shader's own hit test runs), and whether it sits in the movers' top level |

`PFX_TRACE_RT=0` traces the same frame on the software BVH ([trace.md](trace.md)), so a frame that draws there and not on ray queries points at the hardware path. `a_traced_clip_holds_every_copy_of_a_shared_two_sided_card_from_either_side_and_a_thin_pole_among_sprites_frame_after_frame` (`crates/still/tests/instances.rs`) holds these invariants on a generated street over four frames.

## Make the video

```sh
pfx clip samples/workshop.pitofx --camera orbit --frames 120 --fps 30 --size 1280x800 --out orbit --video orbit.mp4
```

With [pcod](https://github.com/gmrdad82/pito-codecs) on the `PATH`, `--video` encodes the frames into an `.mp4` (H.264) or a `.webm` (VP9). Opaque frames are written as RGB PNGs, which both take; a frame with transparent pixels keeps its alpha, which only `.webm` carries. Without pcod, `--video` says it needs pcod, the frames stay in the folder and the command exits 1.

pcod reads the frames folder at the clip's rate (`pcod <frames> <video> --fps <rate> --profile best`), so the video has every frame once.

`--memory-limit <GB>` (or `PFX_MEMORY_LIMIT`) plans the clip's memory before its scene is staged, live or `--trace`, and refuses one past the limit in one line; each frame's line ends with the peaks so far beside the plan ([memory.md](memory.md)). A traced clip keeps its built scene for the whole clip, so it plans more than a traced still of the same recipe.
