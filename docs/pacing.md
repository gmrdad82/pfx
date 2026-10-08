# Pacing an app's bench

A bench or a render loop that draws on the GPU keeps pgpu's manners: it caps itself at 60 frames per second, hands the queue a turn at least every ~50 ms of GPU work and at most every 0.4 s of wall time, reports honest numbers in each turn, and never counts a freeze as work. `pfx_gpu::pace` has all of it as public API, so a product's bench, in its own repository, links the engine crate and writes none of it. The crate writes no job records: a job record is the caller's own.

```rust
use pfx_gpu::pace::{Frames, Pace};

let mut pace = Pace::new();
let mut frame_ms = Vec::new();
for index in 0..frames {
    let passes = pace.frame(|| renderer.render(&scene, index));
    frame_ms.push(passes_total_ms(&passes));
}
frame_ms.sort_by(f64::total_cmp);
let report = Frames::of(&frame_ms);
```

## What `Pace` does each frame

`Pace::new()` reads the environment; `Pace::with_clock(clock, cap_fps)` takes a `FrameClock` and the cap, for tests.

- **`frame(work)`** times `work` on the clock less frozen time and passes the result's GPU time on. The result is a `FrameGpu`: `()` (no timing), `Vec<PassTiming>` or `[PassTiming]` (the frame's pass timestamps, summed by `pace::gpu_ms`).
- **`after(frame_ms)`** is for a loop that times its own frames. **`after_gpu(frame_ms, gpu_ms)`** takes the GPU time too.
- **Work counted.** A frame's GPU time, when it has one. Otherwise its CPU time, never more than the unfrozen time since the previous frame's turn and sleep, so a freeze, a turn's wait or the cap's sleep never counts. The first CPU-timed frame adds no work, since it holds first-use pipeline builds.
- **Turns.** A turn goes out once the work since the last one reaches `TURN_AFTER_MS` (50), or once `PFX_TURN_WALL_MS` of wall time (default `pace::TURN_WALL_MS`, 400) has passed since the last one, whatever the work. It carries `--work-ms` (the sum) and `--longest-ms` (the longest single frame since the last turn) from `TurnReport::args()`, with three decimals. `WallClock` runs `pgpu turn` only when `GPU_QUEUE_TURN` is set, and waits for it; a turn's wait is never counted in the next frame.
- **The cap.** After the turn, the loop sleeps to the next 1/60 s boundary. A late frame does not owe a burst: the boundary restarts from now. `PFX_UNCAPPED=1` (`pace::UNCAPPED_VAR`) lifts the cap for a deliberate peak measurement; the turns stay.
- **Frozen time.** pgpu writes a job's cumulative frozen milliseconds to the file named by `GPU_QUEUE_FROZEN` (`pace::FROZEN_VAR`). `WallClock::frozen_ms` reads it at both ends of every interval and subtracts the difference. A missing variable or file reads as no freeze (`pace::frozen_file_ms`, `pace::Frozen`).
- **`Frames::of(&sorted_ms)`** gives p50, p95, p99, max and `fps` (1000 ÷ p50) of sorted frame times.

A frame is one submission, so keep each under ~300 ms and a live frame near a few milliseconds; a heavy pass is split with the `Pacer`.

## Records and progress: the caller's hook

`Pace` knows nothing of any job system. Give it a `Sink` to report progress at each turn and its end:

```rust
use pfx_gpu::pace::{Pace, Sink, Tally, FrameClock};

struct Mine;

impl Sink for Mine {
    fn progress(&mut self, done: u64, total: u64, fps: f64) {}
    fn end(&mut self, ok: bool) {}
}

let mut pace = Pace::new();
let now = pace.clock.now_ms();
let pace = pace.with_tally(Tally::with_sink(Box::new(Mine), planned_frames, now));
```

`progress` runs after each turn with the frames done, the frames planned and the rate since the start; `end(true)` runs when the `Tally` drops, `end(false)` when it drops during a panic. A bench that writes job records implements `Sink` over its own job; the engine's examples do it in `crates/live/examples/manners`, which also reads `GPU_QUEUE_PID` to decide whether to record at all; the record itself goes through `pfx-report`'s job hook, a no-op unless a build patches in a reporter.

## Testing a loop

`FrameClock` has `now_ms`, `sleep_ms`, `turn(TurnReport)` and `frozen_ms`; a fake of four methods drives `Pace` with no wall clock, no sleeping and no pgpu, which is how the library's own tests check the 60 fps boundaries, the 50 ms and 0.4 s turns, the first-frame rule and the freezes (73-minute and 127-second freezes inside 12 ms frames, a 3.2 s turn wait, 127 s of wall time around 4.5 ms of GPU time).

## Running

Run a bench as `pgpu run --class clip --as <product> -- <command>`; `--class interactive` is for a single short GPU test. Build it first, then run the binary under `pgpu run`.
