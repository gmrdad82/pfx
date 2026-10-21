# Frame stats

The engine's inside view of a running target: one JSON line per presented frame, for a tool that runs a game and reads how its frames went (a target report can read it beside MangoHud's outside view). It is an out of pfx, and this file is its spec. The schema version changes only with this file.

```
PFX_FRAME_STATS=<path> <game>
```

When the variable is set to a non-empty path, `pfx-live`'s `Renderer` opens the file for append (creating it) when it is built, and writes one line per presented frame. When it is unset, nothing is opened, nothing is written and a frame pays one branch: no allocation, no clock read.

- A file that cannot be opened turns the stats off and prints one line to stderr naming the variable.
- One renderer per process is the supported case. Every renderer opens the variable's file when it is built, so two that both draw interleave their lines, each with its own ticks and its own end line. A stream that never counts a frame writes nothing, not even its end line, so a second renderer that only builds (a loader compiling its pipelines) or a handle replaced by `set_frame_stats` before any frame leaves the file to the one that draws. A renderer on another thread that should stay out of the file gets `set_frame_stats(FrameStats::off())`.
- The file is JSON Lines: UTF-8, one object per line, `\n` ended, flushed whenever the writer catches up, so a reader may tail it.

## What counts as a frame

A frame is one submission that puts an image on the output:

| Source | `kind` |
|---|---|
| `render`, `submit` and their `with_flat` forms; `render_flat`, `submit_flat` and their `look` forms; a frame on demand that is `Need::Full` | `full` |
| a frames-on-demand `Need::Partial` | `partial` |
| a frames-on-demand `Need::Reproject`, and the bridge frame | `reproject` |
| `present_last`: the last image presented again, nothing redrawn | `idle` |

`Need::None` submits nothing, so it has no line (a product that wants an idle line per vsync calls `present_last`). `wake` is not a frame and has none.

### Presented frames

A product that presents frames the renderer does not draw (a splash, a frame drawn only from its UI) opens a presented frame around each present, so every frame on the screen has exactly one line:

```rust
let stats = renderer.frame_stats();
stats.present_begin(Source::Ui);
// the frame's work: the game's update, renderer submissions, the UI, the acquire
stats.presented([width, height]);
```

- `present_begin(source)` opens the frame; `presented(size)` closes it and writes its line, right after the present call; `present_abandon()` drops it when nothing was presented (the surface was lost, nothing changed), and no line is written; a second `present_begin` before `presented` drops the open frame the same way, and `presented` with no frame open writes nothing.
- Renderer frames submitted while it is open write no line of their own: they fold into the presented frame's line. Its `kind` is the strongest of them (`full` over `partial` over `reproject` over `idle`), or `full` when the renderer drew nothing, since the product drew the whole output; its `submit_ms`, `prepare_ms`, `encode_ms`, `poll_ms` and `queue_ms` are theirs, summed, so they keep the meaning the key table gives them (all `0` and `prepare_ms` `null` when the renderer drew nothing); its `gpu_ms` is the sum of their GPU times, written against the presented frame's `tick` once every one of them has resolved ([below](#gpu_ms-and-its-tick)).
- `frame_ms` is the presented frame's own time on the CPU, from `present_begin` to `presented`: everything the product did for it (the game's update, which `sim_ms` also counts, the renderer, the UI, the acquire and the present). `acquire_ms` is the part of it spent waiting for the surface's next image (pfx-game reports its `get_current_texture` as `Span::Acquire`); `wait_ms` is the renderer's own waits for the GPU inside it, as in the key table; `hud_ms` is the dev HUD's draw inside it; `size` is the output's size, the window's.
- `source` says what drew it: `splash`, `ui` when the renderer drew nothing in it, and `game` when the renderer drew in a frame opened as `ui`. A line outside a presented frame (a game with its own loop, `pfx bench`) is `game`.
- A handle's presented frame belongs to the thread that opened it: a renderer drawing on another thread with the same handle would fold its frames into whichever frame is open, and its `present_begin` would drop that frame. A renderer on another thread gets its own stats, or none.

pfx-game does this in a game's window and in `Headless` with the GPU, so a game splits its boot by `source`: the splash's frames, then the UI's (an ident, a title), then the game's. The window's ground presents while the game is built and warmed (two before the renderer, and its stats, exist, and one more when the window's size changes during the warm-up) are not frames the game or the splash drew, and have no line. The game's warm-up passes run inside frames: under the splash each pass runs in a splash frame from the second on, so its time is in that frame's `frame_ms` and a renderer frame the game submits while warming folds into that line; without the splash the passes run on redraws that present nothing, so they write no line, and a renderer frame submitted in one is dropped with it.

## The line

```json
{"schema_version":2,"tick":1,"t_ms":207.486,"sim_ms":0.031,"submit_ms":1.425,"prepare_ms":0.412,"encode_ms":0.946,"poll_ms":0.009,"queue_ms":0.058,"wait_ms":5.240,"gpu_ms":4.825,"gpu_tick":1,"present_interval_ms":7.400,"kind":"full","size":[2560,1440],"device":"desktop","hud_ms":null,"frame_ms":null,"acquire_ms":null,"source":"game"}
```

Every key is always present, in this order. Times are milliseconds as JSON numbers with three decimals, never negative.

| Key | Type | Meaning |
|---|---|---|
| `schema_version` | integer | `2` (version 1 had no `prepare_ms`, `encode_ms`, `poll_ms`, `queue_ms` or `wait_ms`; every other key means the same) |
| `tick` | integer | the frame's index, from 0, counting the frames of this file (not the renderer's submission count, which also counts `wake`) |
| `t_ms` | number | monotonic time from the start of the first frame to the start of this one; the first line is `0.000` |
| `sim_ms` | number or null | the time the game's simulation took since the previous frame's submit: the sum of the `sim_begin`/`sim_end` spans completed in that interval, which `pfx-play`'s `PlaySession::advance` and `step` report when given the stats (`set_frame_stats`). `null` when no span was reported |
| `submit_ms` | number | CPU time of the frame from the renderer's first work on it to the end of `queue.submit`: encoding and submitting, not waiting for the GPU; in a presented frame, the sum over the renderer frames folded into it, `0` when there were none. It never holds the simulation, so `sim_ms + submit_ms` is the frame's CPU work |
| `prepare_ms` | number or null | the part of `submit_ms` spent preparing the scene before any command is encoded: a flat frame's draw, text and buffer uploads (`FlatPass::prepare`). `null` for frames whose path has no separate prepare step (the 3D frame, a flat look, frames on demand), where that work is in `encode_ms` |
| `encode_ms` | number | the rest of `submit_ms`: recording the frame's passes and finishing the command buffer (`submit_ms` minus `prepare_ms`, `poll_ms` and `queue_ms`) |
| `poll_ms` | number | the part of `submit_ms` in the non-blocking `device.poll` before the submit and in reading the GPU times it resolved. A large value is the driver or the OS holding that call |
| `queue_ms` | number | the part of `submit_ms` in the `queue.submit` call itself: validation, residency and the driver's submit. A large value with small `prepare_ms` and `encode_ms` is a stall in the driver or the OS, not the frame's own work |
| `wait_ms` | number or null | time the CPU blocked waiting for the GPU after this frame's submit and before the next frame began: a blocking `render`, `render_flat` and their forms, or `wait_frames` that had to wait. `null` when nothing waited, which is the usual case with frames in flight (`Renderer::set_frames_in_flight`) |
| `gpu_ms` | number or null | GPU time, see below |
| `gpu_tick` | integer or null | the `tick` that `gpu_ms` measured; `null` exactly when `gpu_ms` is |
| `present_interval_ms` | number or null | time from the previous line's end to this one's: a line outside a presented frame ends at its submit, a presented frame's line at its `presented`. The renderer does not own the swapchain, so between two renderer lines this is submit to submit, which a product that presents once per frame holds to its present to present; between presented frames it is present to present; in a stream that mixes them it runs from whichever end the previous line had. `null` on the first line |
| `kind` | string | `full`, `partial`, `reproject` or `idle`, as in the table above; in a presented frame, the strongest of its renderer frames, or `full` when the renderer drew nothing in it (a splash or UI frame: read `source` to tell them from the renderer's own full frames) |
| `size` | `[w, h]` | the renderer's drawn size in pixels; in a presented frame, the output's (the window's) |
| `device` | string or null | the device the game runs on: `steam-deck-lcd`, `steam-deck-oled` or `desktop` (`docs/screens.md`). Detected once when the stats open a file (the variable, or `FrameStats::to_file`), with `PFX_DEVICE` as an override; `null` for stats made with `to_writer` and no `set_device` |
| `hud_ms` | number or null | the dev HUD's own draw cost over this frame, from the start of its draw to the end of its `queue.submit` (see [Dev HUD](#dev-hud)); `null` when the HUD did not draw over the frame: hidden, or built without the `dev-hud` feature |
| `frame_ms` | number or null | a presented frame's whole time on the CPU, from `present_begin` to `presented`, sim, UI, acquire and present included ([Presented frames](#presented-frames)); `null` on a line outside a presented frame |
| `acquire_ms` | number or null | the part of `frame_ms` spent waiting for the surface's next image; `null` outside a presented frame, and when nothing was acquired (`Headless`, an offscreen target) |
| `source` | string | what drew the frame: `splash`, `ui` or `game` ([Presented frames](#presented-frames)) |

`frame_ms`, `acquire_ms` and `source` were added within schema version 2, after `hud_ms`, and every other key keeps its meaning: a reader that finds no `source` (a file written before them) reads `game` and `null` for the other two, and a reader that ignores them reads every line as before.

### `gpu_ms` and its tick

GPU time comes from the renderer's timestamp queries and resolves a frame or two after the frame was submitted, so a line cannot hold its own frame's time at the moment it is written. A line is written when the next frame is submitted (or when the file is closed), and carries the newest GPU time that had resolved by then, against the tick it measured:

- `gpu_tick` is that tick. It is the line's own tick when the frame had already resolved (a product that waits on its frames, such as `render`), and an earlier one otherwise, at most two frames earlier.
- `gpu_ms` is the sum of the frame's timed GPU passes. It is one number for the whole frame, with no gaps between passes, so it is below the frame's wall time whenever the GPU idles.
- Each GPU time is written once, on the first line after it resolved. A presented frame that folded in several renderer frames has one GPU time, their sum, written on the first line after the last of them resolved. Only renderer frames that timed a pass count: one the GPU profiler could not time (no pass timed, every read slot busy) is never waited for. If a newer frame's time resolves before an older frame's sum is whole, the older frame's partial sum is written on the next line, unless the newer frame's sum is whole by then, which is written instead; a late part of an older frame is dropped. A line with no new time has `gpu_ms` and `gpu_tick` both `null`; if two resolve between two lines, only the newer is written. A reader joins GPU time to frames by `gpu_tick`, never by the line it sits on.
- A timing that resolves later than the renderer's two-frame lag is not reported at all. Never assume one GPU time per line.
- Several renderers may share one handle (a game's own `Renderer` given `renderer.frame_stats()`): `set_frame_stats` gives each renderer its own submitter on the handle (`FrameStats::joined`), so their frame numbers, each counted from 0, never mix, and a renderer rebuilt on a resize starts as a new submitter.
- The first frame of a run usually shows `null`: its timing resolves after its line is written and is superseded by the next.
- On a device with no timestamp queries every line has `null`.

## Drops

The renderer hands each line to a writer thread through a bounded channel of 256 lines, and never blocks on it. If the channel is full (a stalled disk), the line is dropped and counted. A dropped line leaves a gap in `tick`; later lines are unaffected.

## The end line

On a clean exit (the renderer, and every handle to its stats, dropped, or `FrameStats::finish` called) of a stream that counted at least one frame, the last line is written:

```json
{"schema_version":2,"end":true,"frames":6,"dropped":0}
```

`frames` is the number of frames the run presented, dropped ones included; `dropped` is how many of them have no line. So the file holds `frames - dropped` frame lines before the end line. A file without an end line came from a process that did not exit cleanly (a kill, a crash, `process::exit`): its lines are valid up to the last complete one. A stream that counted no frame writes no end line, and nothing else.

## An example

The first three frames of a measured run of `crates/game/examples/flat` at 2560×1440 (one `render_flat` per frame, the renderer waiting for each frame: `FLAT_IN_FLIGHT=0`):

```json
{"schema_version":2,"tick":0,"t_ms":0.000,"sim_ms":0.001,"submit_ms":201.512,"prepare_ms":198.993,"encode_ms":2.312,"poll_ms":0.035,"queue_ms":0.172,"wait_ms":5.379,"gpu_ms":4.880,"gpu_tick":0,"present_interval_ms":null,"kind":"full","size":[2560,1440],"device":"desktop","hud_ms":null,"frame_ms":null,"acquire_ms":null,"source":"game"}
{"schema_version":2,"tick":1,"t_ms":207.486,"sim_ms":0.031,"submit_ms":1.425,"prepare_ms":0.412,"encode_ms":0.946,"poll_ms":0.009,"queue_ms":0.058,"wait_ms":5.240,"gpu_ms":4.825,"gpu_tick":1,"present_interval_ms":7.400,"kind":"full","size":[2560,1440],"device":"desktop","hud_ms":null,"frame_ms":null,"acquire_ms":null,"source":"game"}
{"schema_version":2,"tick":2,"t_ms":214.727,"sim_ms":0.001,"submit_ms":1.629,"prepare_ms":0.412,"encode_ms":1.128,"poll_ms":0.012,"queue_ms":0.079,"wait_ms":5.015,"gpu_ms":4.855,"gpu_tick":2,"present_interval_ms":7.445,"kind":"full","size":[2560,1440],"device":"desktop","hud_ms":null,"frame_ms":null,"acquire_ms":null,"source":"game"}
```

(The first frame's 199 ms of `prepare_ms` is pipeline creation; a target report skips frames until the interval settles.) Every frame of that run blocks about 5 ms in `wait_ms`. The same game with two frames in flight (`FLAT_IN_FLIGHT=2`, the default) waits for nothing, and its GPU time arrives a frame later, joined by `gpu_tick`:

```json
{"schema_version":2,"tick":999,"t_ms":3656.506,"sim_ms":0.000,"submit_ms":1.355,"prepare_ms":0.419,"encode_ms":0.883,"poll_ms":0.001,"queue_ms":0.051,"wait_ms":null,"gpu_ms":2.846,"gpu_tick":998,"present_interval_ms":3.640,"kind":"full","size":[2560,1440],"device":"desktop","hud_ms":null,"frame_ms":null,"acquire_ms":null,"source":"game"}
{"schema_version":2,"tick":1000,"t_ms":3659.224,"sim_ms":0.001,"submit_ms":1.327,"prepare_ms":0.391,"encode_ms":0.900,"poll_ms":0.001,"queue_ms":0.036,"wait_ms":null,"gpu_ms":2.847,"gpu_tick":999,"present_interval_ms":2.691,"kind":"full","size":[2560,1440],"device":"desktop","hud_ms":null,"frame_ms":null,"acquire_ms":null,"source":"game"}
```

A clean exit then closes the file with its end line, as above.

## For a game with its own loop

A game that does not use `pfx-play` reports its simulation time through a handle, and the renderer does the rest:

```rust
let stats = renderer.frame_stats();
loop {
    stats.sim_begin();
    game.update();
    stats.sim_end();
    renderer.submit(&scene, &text, &effects, finish, &view)?;
}
```

`frame_stats()` is a cheap clone of the renderer's handle, off when the variable is unset (every call then returns at once). `Renderer::set_frame_stats` replaces it, and `FrameStats::to_file(path)` makes one without the variable, for a test. `PlaySession::set_frame_stats` hands a session the same handle so its `advance` and `step` are the `sim_ms`.

## Dev HUD

The same numbers on screen, for a developer playing a dev build. It is an egui panel in pfx's theme (`pfx-theme`, the colours and fonts `pfx gui` uses), drawn over the presented frame, top right, hidden at start and toggled by F3.

It shows the frame rate and the p50, p99 and largest `present_interval_ms` over the graph's four seconds (`Pace`), the newest frame's `sim_ms`, `submit_ms` and `kind`, the newest resolved `gpu_ms`, its own draw cost (`hud_ms` of the last frame it drew over), a frame-time graph of the last four seconds against the 16.67 ms line, and how many frames in the graph went over it. The graph's bars are `present_interval_ms`, each as wide as its frame lasted: violet within the budget, orange over it, red over twice it.

It reads the stats, never a clock of its own: the renderer keeps the last 512 frames' values in memory for it (`FrameStats::keep_recent`, `recent`), exactly the values the file's lines get: a frame's `gpu_ms` is filled in when a line writes it, the same sum against the same tick, so a frame whose time the file never writes has none in the HUD either, and the newest time appears a frame after it resolved. When `PFX_FRAME_STATS` is unset, the HUD gives the renderer a measuring handle (`FrameStats::measure()`) that keeps those values and writes no file, so a dev build pays the clock reads the stats file would.

**The feature.** `pfx-live`'s `dev-hud` feature, off by default, adds `pfx_live::hud` and with it egui, egui-wgpu, `pfx-theme` and `pfx-input`. Without it, nothing of egui links into `pfx-live` (`cargo tree -p pfx-live -e features` shows none, a test checks it). A product turns it on only in its dev builds, never in a release or player build, for example with a feature of its own that enables `pfx-live/dev-hud`. `pfx-play` has the same feature and forwards it; `pfx gui` turns it on.

**For a game with its own loop,** the HUD is one value and two calls, the input event and the draw after the frame:

```rust
let mut hud = pfx_live::hud::Hud::new();
loop {
    for event in input_events() {
        if hud.input(&event) {
            continue;
        }
        game.feed(event);
    }
    renderer.submit(&scene, &text, &effects, finish, &view)?;
    hud.draw(&mut renderer, &view);
}
```

`input` takes F3's press (once per press, however long it is held) and release and returns `true` for them; every other event returns `false`. `draw` attaches to the renderer's stats every frame, so the history is there the first time F3 shows it; while hidden it draws nothing. It lays out on the CPU, renders egui into a small sRGB canvas the size of the panel, and composites it over the output in one submission: linear into a float or sRGB output (the renderer's `Rgba16Float` included), encoded into a plain 8-bit one. The panel's scale follows the output's height (one point per pixel up to 1080 lines, two at 2160). `Hud::rect` is where it drew, in output pixels. `draw` takes the output's format and size from the renderer; `draw_on(renderer, output, format, size)` draws into any other output, such as a window's surface at the window's size after an upscale (`PlaySession::draw_hud_on`, which `pfx-game` uses).

**With `pfx-play`,** `PlaySession::feed` hands F3 to the session's HUD instead of the game, and `PlaySession::draw_hud_on(&mut renderer, &output, format, size)` after the frame draws it and points the session's `sim_ms` at the stats the HUD reads. Without the feature `draw_hud_on` is an empty call, so a loop needs no `cfg`. pfx-game's `Painter::draw` calls it after each frame, in a game's window and in `pfx gui`'s play view alike, so F3 in a playing scene shows the HUD. `pfx bench` is headless and never shows it, so it leaves the HUD unwired.

## Stats in the editor

`pfx gui` and a game's `<game> edit` draw the same panel over their viewport, top right: F3 (outside a text field), the `stats` chip under the viewport (`stats  F3` beside Play in `<game> edit`), or `[display] stats = true` in the user's `settings.pitofx` turns it on, and the switch is kept there. It shows:

| Row | What |
|---|---|
| `fps`, `p50`, `p99`, `max` | the window's presents over its last 240 frames: present-call to present-call intervals, counted only between frames drawn back to back, so an idle editor's gaps never count |
| `shown p50`, `p99`, `max` | the same for the times the compositor showed the frames, where it reports them (`wp_presentation` on Wayland, [window.md](window.md#vrr-in-effect)) |
| `cpu` | the p50 of the window thread's time per frame, the panel itself included |
| `gpu` and the passes under it | the p50 of the frame's GPU time (the viewport's passes and the UI's paint), then the six slowest passes by their mean, from the renderer's timestamp queries; none on a device without them |
| `scale` | the viewport's render scale, 100 % at full size ([window.md](window.md)) |
| `target`, `vrr`, `present` | the frame-rate setting (60 fps, 120 fps, auto or uncapped), VRR on, off, or on but not in effect, and the present mode the surface runs |
| `hud` | the panel's own layout time on the CPU |

The window host (`pfx-editor-shell`) records each present into `pfx_gpu::stats::FrameStats` only while the app asks for stats (`App::stats`), and hands the app a snapshot at most four times a second (`App::frames`), so the numbers are steady enough to read and cost nothing while the panel is off. The panel never asks for a frame of its own: an idle window stays idle, and the numbers hold until the next frame.

Its cost, from `crates/live/examples/hud_cost.rs` (600 frames after 60 warm ones, at 3840x2160 and two pixels a point, release build under `pgpu run --class bench`, on an AMD Radeon RX 9060 XT with RADV, the machine at a load average of 21 to 25), two runs:

| | p50 | p99 |
|---|---|---|
| CPU, an egui frame with the panel, its numbers changing four times a second as the editor shows them | 0.12 to 0.15 ms | 0.27 to 0.41 ms |
| CPU, the panel's share over an empty egui frame | 0.12 to 0.14 ms | 0.24 to 0.38 ms |
| CPU, every number changing every frame | 0.14 to 0.20 ms | 0.27 to 0.53 ms |
| GPU, the panel's paint into the 4K frame (524 triangles) | 0.019 to 0.026 ms | 0.027 ms |

The p50 is under 0.2 ms on the CPU and the GPU; the CPU's p99 on this loaded machine is over it, almost all of it egui laying out the panel's twenty rows of text.

## Bench

`pfx bench` plays a scene headless through `pfx-play` with the stats on, so a tool that runs a target gets the same load every time and reads the engine's frame stats beside MangoHud's.

```
pfx bench <scene.pitofx> --seconds N [--camera <object>] [--replay <recording.json>] [--size WxH] [--stats <path>] [--seed S]
```

| Flag | Meaning |
|---|---|
| `<scene.pitofx>` | the scene to play (a `.toml` is refused, naming `pfx scene migrate`), played by `SceneGame` (movers, bodies, sound) plus the camera rig below. Required |
| `--seconds N` | game seconds to run, above 0. The run is `ceil(N × 60)` frames, each advancing the session by exactly 1/60 s, whatever the wall clock does: a slow GPU takes longer, never plays more or fewer ticks. Required |
| `--camera <object>` | stand the camera at the scene's object of that name, looking along its local −Z with its +Y up; the scene's `[camera]` supplies the projection, near, far and shift. Without it the scene's own `[camera]` (or the default camera) is used. The scene format has one `[camera]`, so a named camera is an object |
| `--replay <recording.json>` | drive the camera from a recorded input stream: a `pfx_input::Recording` as JSON, fed one step per sim tick, live events ignored; `pfx gui` saves one with F9 while a scene plays, to `tmp/pfx/recordings/<scene>-<time>.json`. Without it no input arrives and the camera stays fixed |
| `--size WxH` | the offscreen target, 1 to 16384 each. Default 1920×1080 (a scene carries no size of its own) |
| `--stats <path>` | the frame stats file, created or truncated. Default `tmp/pfx/bench/<scene>-<unix seconds>.jsonl` under the caller's git tree, `<scene>` being the file name without `.pitofx` or `.scene.toml` |
| `--seed S` | a whole number, default 0: the session's seed, and the frame seed (its low 32 bits) |

It opens no window: the frames present to an offscreen `Rgba16Float` target, and gamescope provides the real present when it wraps the command. It keeps pgpu's manners through `pfx_gpu::pace::Pace` (60 fps cap unless `PFX_UNCAPPED=1`, a `pgpu turn` between frames), so run it under `pgpu run --class clip --as pfx` when pgpu is installed. The frame stats file is the one in this document, written for this run's renderer and session (a `PFX_FRAME_STATS` in the environment is replaced).

The camera rig is the one game of the bench. A recording holds these actions, all axes bound to keys:

| Action | Keys (negative, positive) | Moves |
|---|---|---|
| `walk` | `S`, `W` | along the view, 2 units/s |
| `strafe` | `A`, `D` | along the view's right, 2 units/s |
| `lift` | `Q`, `E` | along the camera's up, 2 units/s |
| `turn` | `Left`, `Right` | yaw, 60°/s |
| `tilt` | `Down`, `Up` | pitch, 60°/s, held within ±86° |

A recording is made against these actions (`pfx_bench::actions()` is the list). With no key held a tick leaves the camera exactly where it was.

**Deterministic:** the same scene, seed and recording give the same ticks, the same inputs per tick, the same frame kinds and sizes (`full`, at the target's size, ticks 0 to frames − 1) and the same final camera. Only the times vary.

### The summary

When the run ends, the last thing on stdout is one JSON line (a measured run of the neutral room at 320×180) (keys sorted, no promised order):

```json
{"bench":true,"camera":{"at":[0.0,1.4,4.2],"look_at":[0.0,0.7,0.0]},"dropped":0,"frames":120,"gpu_ms":{"max":0.756,"p50":0.217,"p95":0.223,"p99":0.754},"inputs":0,"over_16_67":0,"scene":"bench.pitofx","schema_version":1,"seed":5,"size":[320,180],"sim_ms":{"max":1.197,"p50":0.438,"p95":0.69,"p99":0.965},"stats":"tmp/pfx/bench/bench-1791307762.jsonl","submit_ms":{"max":6.919,"p50":2.989,"p95":3.895,"p99":6.391},"ticks":120}
```

| Key | Meaning |
|---|---|
| `schema_version` | `1`, changing only with this file |
| `bench` | `true` |
| `scene`, `seed`, `size` | as given or defaulted; `size` is `[w, h]` |
| `frames` | the frames the run presented (the stats file's end line), dropped ones included |
| `dropped` | how many of them have no line |
| `ticks` | sim ticks the world ran |
| `inputs` | recorded events fed over those ticks; 0 without `--replay` |
| `camera` | the camera at the end: `at` and `look_at`, four decimals |
| `sim_ms`, `submit_ms`, `gpu_ms` | `p50`, `p95`, `p99` and `max` of that column, nearest rank, milliseconds with three decimals; `null` when no line has the column. Tick 0 is left out (it holds pipeline creation), `null` `sim_ms` and `gpu_ms` values are skipped, and `gpu_ms` is the set of resolved GPU times, joined to frames by `gpu_tick` in the file, not by line |
| `over_16_67` | the lines (tick 0 left out) whose `sim_ms + submit_ms` or whose `gpu_ms` is above 16.67 ms |
| `stats` | the stats file's path, as written |

Every refusal comes before the GPU opens and before the stats file is created, with nothing on stdout. Arguments it cannot parse (an unknown flag, a missing scene or `--seconds`, a bad `--seconds`, `--size` or `--seed`) get clap's answer on stderr with exit 2, as every `pfx` command does (`docs/game.md`, "Refusals"). The rest print one line, `pfx: …`, and exit 1: no scene at the path, a scene that does not load, `--camera` naming no object, a `--replay` that is missing or not a recording.

## Tests

- `crates/live/src/stats.rs` (unit): lines parse and keep their order, the end line, sim time landing on the next frame, GPU time carried once against its tick, a full channel dropping and counting lines (`frames - dropped` lines before the end line), nothing written or counted when unset, the variable's value opening a file and appending.
- `crates/live/src/stats.rs` (unit, continued): `hud_ms` landing on the frame the HUD drew over and `null` otherwise, last in the line; a measuring handle keeping recent frames with their GPU and HUD times and writing nothing.
- `crates/live/src/stats.rs` (unit, GPU times): a presented frame's GPU time as the sum of its renderer frames once all resolved, two renderers on one handle with the same frame numbers each joined to their own tick, a renderer frame that timed nothing never waited for, a frame whose last part never resolves written as a partial sum when a newer one supersedes it, a late part dropped, and `recent` agreeing with the file on every tick.
- `crates/live/src/stats.rs` (unit, presented frames): one line per presented frame with its `source`, renderer frames folding into it with the strongest kind, their poll and queue time and their GPU time on its tick, a wait and the HUD inside it landing on it, an abandoned frame writing nothing and its GPU time ignored, a line outside one being `game`, the key's place last in the line; a measuring handle keeping the source. Also: `presented` with nothing open, a second `present_begin` dropping the first frame and its GPU time, a splash frame staying `splash` when the renderer drew in it, and an off handle presenting nothing.
- `crates/game/src/gpu_tests.rs` (GPU, ignored): `every_drawn_frame_writes_one_stats_line_with_its_source` runs a scene game on demand and a flat game headless and checks one line per drawn frame, `game` where the renderer drew and `ui` where it did not.
- `crates/live/src/hud.rs` (unit, `--features dev-hud`): F3 toggling once per press, the readout taking the newest numbers and four seconds of frames, the panel's layout in pfx's theme and its place top right.
- `crates/live/tests/dev_hud_off.rs`: `cargo tree -p pfx-live -e features` has no egui without the feature, and has it with.
- `crates/play/src/tests.rs`: a session's `advance` reports `sim_ms`.
- `crates/bench/src/tests.rs` (headless): the command's flags and their refusals, a missing scene, recording or camera refused before the GPU, a fixed camera without a recording, a camera standing at a named object, a recording driving the camera and driving it identically twice, an idle recording leaving it alone, the nearest-rank spreads and the summary over stats lines.
- `crates/bench/tests/bench.rs` (GPU, ignored): `two_benches_of_the_same_scene_give_the_same_ticks_inputs_and_frame_kinds` benches the neutral room for 2 s twice (120 frames each) and compares the tick, kind and size columns, the sim ticks, the inputs and the final camera, and reads the summary line; `a_replayed_recording_moves_the_benched_camera` replays a recording that holds `D` and checks the camera moved, with the same ticks and frame columns as the still run.
- `crates/live/tests/frame_stats.rs` (GPU, ignored): `frame_stats_carry_a_positive_gpu_time_within_two_frames` submits six frames of the neutral room and checks the lines, the end line, and that every `gpu_ms` is positive and at most two ticks old.
- `crates/live/tests/dev_hud.rs` (GPU, ignored, `--features dev-hud`): `the_hud_draws_top_right_over_a_neutral_frame_and_f3_toggles_it` draws the HUD over the neutral room, writes the shot to `tmp/dev-hud/hud-over-room.png`, checks that nothing outside its rect changed, that the panel, the 16.67 ms line and its text are in pfx's colours, that F3 hides it again, and that `hud_ms` is in the lines exactly while it drew; `the_hud_writes_linear_colour_into_a_float_frame_and_measures_without_a_file` checks the panel's colour in an `Rgba16Float` output and the measuring handle.

```
cargo test -p pfx-live --test frame_stats --no-run
pgpu run --class interactive --as pfx -- cargo test -p pfx-live --test frame_stats -- --ignored --test-threads=1
cargo test -p pfx-live --features dev-hud --test dev_hud --no-run
pgpu run --class interactive --as pfx -- cargo test -p pfx-live --features dev-hud --test dev_hud -- --ignored --test-threads=1
cargo test -p pfx-bench --test bench --no-run
pgpu run --class clip --as pfx -- cargo test -p pfx-bench --test bench -- --ignored --test-threads=1
```
