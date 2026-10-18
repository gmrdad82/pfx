# Benchmark a scene

`pfx bench` plays a scene (a scene's `.pitofx`, [.pitofx files](pfx-file.md#the-user-file)) headless for a fixed number of game seconds and reports how its frames went, so the same scene gives the same load on every run. It opens no window: frames go to an offscreen target. Use it to check a scene against the 16.7 ms a 60 fps frame allows, and to compare two builds of a game.

## Run it

```
pfx bench <scene.pitofx> --seconds N [--camera <object>] [--replay <recording.json>] [--size WxH] [--stats <path>] [--seed S]
```

A run is `ceil(N × 60)` frames, each advancing the game by exactly 1/60 s, whatever the wall clock does. The size defaults to 1920×1080. On the first scene of a game made by `pfx new` ([Make your first game](first-game.md)), at 4K for 5 seconds, on an AMD Radeon RX 9060 XT under Linux:

```
$ pfx bench content/first.pitofx --seconds 5 --size 3840x2160 --stats tmp/bench/first.jsonl
{"bench":true,"camera":{"at":[2.5,1.65,4.0],"look_at":[0.0,0.495,0.0]},"dropped":0,"frames":300,"gpu_ms":{"max":2.78,"p50":2.608,"p95":2.721,"p99":2.768},"inputs":0,"over_16_67":0,"scene":"content/first.pitofx","schema_version":1,"seed":0,"sim_ms":{"max":0.26,"p50":0.029,"p95":0.075,"p99":0.102},"size":[3840,2160],"stats":"tmp/bench/first.jsonl","submit_ms":{"max":3.135,"p50":0.761,"p95":1.035,"p99":1.223},"ticks":300}
```

## Read the summary

The last line on stdout is the summary:

- `frames` and `ticks`: 300 frames for 5 seconds, every one presented (`dropped` 0).
- `gpu_ms`: the GPU time of a frame, 2.608 ms at the median and 2.768 ms at p99.
- `sim_ms`: the game's simulation, 0.029 ms at the median.
- `submit_ms`: the CPU time to encode and submit a frame, 0.761 ms at the median.
- `over_16_67`: how many frames went over a 60 fps frame's 16.67 ms, on the CPU or the GPU; 0 here.

The first frame is left out of the spreads, since it holds pipeline creation. Every key is in [Frame stats](frame-stats.md#the-summary).

## Read the frames

`--stats` names the frame stats file, one JSON line per frame and an end line (by default `tmp/pfx/bench/<scene>-<unix seconds>.jsonl` in the caller's git tree):

```
{"schema_version":1,"tick":1,"t_ms":13.893,"sim_ms":0.016,"submit_ms":0.897,"gpu_ms":2.570,"gpu_tick":1,"present_interval_ms":7.274,"kind":"full","size":[3840,2160],"device":"desktop","hud_ms":null}
...
{"schema_version":1,"end":true,"frames":300,"dropped":0}
```

GPU time resolves a frame or two after its frame is submitted, so a reader joins `gpu_ms` to frames by `gpu_tick`, never by line ([Frame stats](frame-stats.md#the-line)).

## Move the camera

`--camera <object>` stands the camera at a named object of the scene. `--replay <recording.json>` drives it from an input recording, which `pfx gui` saves with F9 while a scene plays; the same scene, seed and recording give the same ticks and the same final camera on every run.

## When it refuses

Arguments it cannot parse get clap's answer, with exit 2:

```
$ pfx bench content/first.pitofx
error: the following required arguments were not provided:
  --seconds <SECONDS>

Usage: pfx bench [OPTIONS] --seconds <SECONDS> <SCENE>

For more information, try '--help'.
```

Anything else it refuses before the GPU opens, in one line with exit 1:

```
$ pfx bench content/missing.pitofx --seconds 1
pfx: content/missing.pitofx: no such scene file
$ pfx bench content/old.scene.toml --seconds 1
pfx: content/old.scene.toml is a plain TOML file; import it once with `pfx scene migrate content/old.scene.toml`, which writes content/old.pitofx beside it
```

The bench caps itself at 60 fps and takes turns on the GPU ([Pacing an app's bench](pacing.md)), so a run takes about as long as the seconds it plays.
