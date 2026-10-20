# Long render jobs

`pfx job` runs a long `pfx clip`, `film`, `render` or `bake` detached under a name. The job keeps its state in a `job.json`, carries on from its last finished frame after a pause, a stop, a crash or a reboot, and ends when its frames are done. There is no daemon and no socket: each job has one supervisor process while it runs, and `job.json` plus signals is the whole interface.

Nothing starts a job on its own. Start it when the machine is free, and resume it the same way.

## Start a job

```sh
pfx job start night -- clip film.pitofx --camera main --timeline --size 3840x2160 --trace --spp 256 --out tmp/night
```

Everything after `--` is the pfx command the job runs, with its own flags. The job runs it in the folder you started it from, so relative paths mean what they mean on that line. `pfx job start` prints the supervisor's pid and the job's folder and returns at once; `--json` prints `{"name", "pid", "dir", "state"}`. `--checkpoint <every>` (minutes, or samples like `64spp`) is handed to a traced clip or `render --recipe` as its own `--checkpoint`, so a pause or a crash inside a long frame loses at most that much: the run is ended right after a checkpoint is written when a pause is waiting. A clip job also runs with `--json`, so the job sees each frame's samples, its checkpoints and its resume report.

A clip's frames go through the GPU queue (pgpu) when it is on the `PATH`, as `pfx clip --trace` does on its own; without it the clip runs directly. The supervisor itself holds no GPU and only waits on its run.

## Part of a clip

`--frames A..B` makes only frames A to B of a clip, both kept:

```sh
pfx job start first-half -- clip film.pitofx --camera main --timeline --size 3840x2160 --trace --out tmp/film --frames 0..59
```

The same works without a job, as `pfx clip … --range 0..59`. A range numbers, names and seeds every frame exactly as a whole run of the line does: frame 40 is `frame-0040.png` and gets the same pose, the same seed (the recipe's), the same samples per pixel and the same adaptive threshold whichever range made it. The clip line check (`clip.txt`) leaves the range out, so a range and a whole run share one folder, and a second run with the same line skips every finished frame.

To split a clip across two machines, run the same line with two ranges, one on each, then copy the `frame-*.png` files of one folder into the other. A run of the whole line in the joined folder finds every frame finished and goes straight to `--video` if you asked for one (`--range` and `--video` don't go together). Copy the frames only, not `clip.txt`: it names the recipe by its absolute path, which differs between machines.

Frames made on two different GPUs, drivers or APIs (Vulkan on Linux, Metal on macOS) match in everything above but not bit for bit: shader compilers fuse and reorder float operations differently, so a path-traced pixel lands a little apart. What a frame from another device looks like next to this machine's is measured in [Numbers](#numbers); a joined clip's frames each come whole from one machine, so a seam can only show as that small per-frame noise difference, never as a jump in pose or timing.

## Watch and steer it

| Command | What it does |
|---|---|
| `pfx job list` | Every job with its state, frames and ETA. |
| `pfx job status <name> [--watch]` | The state, frames done of total, the frame in flight and its samples when the run reports them, the median frame time and the ETA; `--watch` prints a line on every change until the job stops running. |
| `pfx job pause <name>` | Ends the run once the frame in flight is written; the job is `paused`. |
| `pfx job stop <name> [--now]` | The same, `stopped`; `--now` ends the run at once and the frame in flight is made again on resume. |
| `pfx job resume <name>` | Carries on a paused, stopped or interrupted job from its last finished frame. |
| `pfx job resume --all` | Carries on every job a crash or a reboot interrupted. Jobs paused or stopped by hand wait for their name. |
| `pfx job log <name> [--lines N]` | The run's stderr and the supervisor's notes, kept to two files of 4 MiB. |
| `pfx job clean` | Removes the folders of done and failed jobs. The frames stay where the run made them. |

Each takes the job's name or its supervisor's pid, and `--json`. A pause or a stop never lands mid-write: a frame is written to a `.partial` file and renamed when it is whole, and the run is ended only after a finished frame is announced.

The supervisor also takes signals: `SIGUSR1` pauses and `SIGUSR2` stops at the next frame, and `SIGTERM`, `SIGINT` or `SIGHUP` stop it now.

## States

| State | Means |
|---|---|
| `running` | The supervisor runs the command. |
| `waiting` | Between tries after a failed run. |
| `paused` | Paused by hand; `resume` carries on. |
| `stopped` | Stopped by hand, or its supervisor is gone (a crash or a reboot): `status` says which. |
| `done` | Every frame is made; the job wrote its report and exited. |
| `failed` | The same frame failed 3 times, or the supervisor could not run. |

A job whose `job.json` says `running` but whose supervisor is dead, or whose pid now belongs to another process, reads as `stopped`, never `running`. The supervisor's pid is checked against its start time (on Linux `/proc/<pid>/stat` and the boot id, on macOS `ps`), and the job's lock file is held by its supervisor while it lives. When a resume finds a run left over from a supervisor that died, it ends that run's process group first.

A run that fails is tried again 10 s later from its last finished frame; the third failure of the same frame fails the job with the run's last error. A run that exits cleanly with frames left counts as a failure.

## job.json

The job's folder is `<state>/jobs/<name>/`, where `<state>` is `$PFX_STATE` if it is set, else `$XDG_STATE_HOME/pfx` or `~/.local/state/pfx` on Linux, `~/Library/Application Support/pfx` on macOS and `%LOCALAPPDATA%\pfx` on Windows. It holds `job.json`, the `log` and `log.1`, a `lock` and, for a moment, an `ask` file a command left for the supervisor. `job.json` is written whole to a temporary file, synced and renamed, after every frame and every change of state.

| Field | Holds |
|---|---|
| `format` | 1 |
| `name`, `argv`, `cwd` | The job's name, the pfx command and the folder it runs in. |
| `range` | `[A, B]` from `--frames`, or null. |
| `state`, `asked`, `reason` | The state above, what was last asked (`run`, `pause`, `stop`) and why it is in its state. |
| `supervisor`, `child` | `{"pid", "start"}` of the supervisor and of the run in flight; `start` is the process's start time as the system tells it. |
| `frames` | `{"done", "total"}`: frames finished of the range or the clip. |
| `frame`, `samples` | The frame in flight and `{"done", "of"}` samples when the run reports them. |
| `resume` | What the clip's last resume found ([recipe-clips.md](recipe-clips.md#stop-and-resume)): `{"verified", "redone", "check"}`, the count of frames whose sha256 matched the folder's `frames.json`, the frames made again, and `{"frame", "same"}` for the frame re-rendered before carrying on (`same` null until it is compared, false when it differs and the run stopped), or null. |
| `frame_seconds`, `median_frame_seconds`, `eta_seconds` | The last 64 frame times, their median and the median times the frames left. |
| `path` | The sentence the clip's last `path` line gave: the adapter, whether ray queries run and the BVH, or null before it traces. `pfx job status` shows it as `traced on`. |
| `fails`, `last_error` | Failed tries by frame, and the last run's error. |
| `runs` | How many runs the job has started. |
| `created`, `updated`, `finished` | Local times, `YYYY-MM-DDTHH:MM:SS`. |
| `report` | When done: frames, median frame time and runs. |

`pfx job status --json` prints the same object with `state` as it is seen (a dead supervisor's job as `stopped`), `stale` true when that differs from the file, and `dir`.

## Numbers

<p align="center"><img src="images/resume.gif" width="800" alt="A terminal: pfx new writes the street starter; pfx clip traces frames 0 to 3 of its timeline at 480x270 and 64 samples into whole/; the same frames start as a job named cut, and once frame 2 is in flight both the supervisor's and the run's process groups are killed with SIGKILL; pfx job list shows cut stopped, 2 of 4 frames, its supervisor gone, and cut/ holds frames 0 and 1; pfx job resume carries on and the list shows cut done, 4 of 4 frames; cmp finds every frame the same bytes as the uninterrupted run."></p>

Measured on Linux with an AMD Radeon RX 9060 XT (RDNA 4, Vulkan through RADV), on the table test scene with an orbiting camera.

- **A kill mid-frame.** A traced clip of 8 frames at 480x300 and 512 samples (about 1.5 s a frame) ran as a job; both the supervisor's and the run's process groups were killed with `SIGKILL` halfway through frame 2, after frames 0 and 1 were written. `pfx job resume --all` found the job `stopped`, carried on from frame 2 and made frames 2 to 7. All 8 frames equal an uninterrupted run's byte for byte, and `--range 3..5` makes the same bytes as the whole run's frames 3 to 5 (`tests/jobs.rs`).
- **Another driver.** The same frames on Mesa's lavapipe, a CPU Vulkan driver with its own shader compiler and float path, beside RADV:

| Frames | Largest difference | Pixels that differ | Pixels more than 8 levels off | PSNR |
|---|---|---|---|---|
| 2 to 4 at 160x100, 64 samples | 1 to 4 levels of 255 | 0.3 to 1.3 % | none | 71 to 77 dB |
| 3 at 160x100, 256 samples, threshold 0.02 | 4 levels | 1.0 % | none | 72 dB |

Metal on Apple Silicon can't run here, so pfx job and ranges on macOS are built but untested, and how close a Mac's frames sit to this machine's is not measured.
