# Render a still with the path tracer

`pfx render --scene` traces a shot on the GPU with pfx's path tracer, denoises it, and writes a 16-bit master and an 8-bit delivery PNG with a JSON manifest beside them. It runs headless. This page renders the engine's neutral sample shot from a checkout of this repository.

![pfx's path tracer on its neutral sample shot: a grey disc and a grey ring turning back and forth over a dark floor, in soft key, fill and rim light](images/turntable.gif)

## Find the shot

A product keeps its recipes in a folder: its look in `presets/<product>.toml` and its shots in `shots/<product>.toml`. The engine's own tests use a neutral product, `sample`, in [crates/assets/tests/recipes](../crates/assets/tests/recipes):

```
$ pfx list --scenes --product sample --recipes crates/assets/tests/recipes
sample	shapes	The sample icons in a row, turning
```

## Render it

```
$ pfx render --scene shapes --product sample --still --recipes crates/assets/tests/recipes --size 1920x1080
tmp/renders/sample/still/sample-still-shapes-1920x1080.png
```

The command prints the delivery's path. Output lands under the caller's git tree, in `tmp/renders/<product>/still/`:

```
sample-still-shapes-1920x1080.png          the 8-bit sRGB delivery, 1.6 MB
sample-still-shapes-1920x1080.master.png   the 16-bit sRGB master, 6.1 MB
sample-still-shapes-1920x1080.json         the manifest
```

Every pixel samples adaptively to a noise threshold of 0.01, and the frame then goes through the denoiser ([The denoiser](denoise.md)). The manifest records the device, the recipes it read with their sha256, the sampling and what the tracer did. On an AMD Radeon RX 9060 XT this shot's 1920×1080 frame took 1.35 s of GPU time:

```json
"tracer": {
  "gpu_ms": 1353.0,
  "mean_samples": 56.2,
  "most_samples": 64,
  "sampling": { "cap": 64, "denoise": true, "growth": 1.5, "min": 16, "threshold": 0.01 },
  "triangles": 8836,
  "version": "1.0.0"
}
```

## Change the frame

- `--draft` stops sooner (threshold 0.05, at most 16 samples), denoised too, for quick looks; `--samples N` sets the cap.
- `--size 3840x2160` renders at 4K.
- `--angle front|three-quarter|top`, `--turn`, `--tilt` and `--zoom` move the camera; `--background transparent|#rrggbb|hdri` and `--shadow on|off` change the setting.
- `--clip` renders the shot's keys as a 60 fps clip instead, encoded with the system's ffmpeg.

The asset renderer's recipes, masters and outputs are in [crates/assets/README.md](../crates/assets/README.md), and the `pfx` tool's commands are listed in [The crates](../README.md#the-crates). When `pgpu`, a GPU queue, is on `PATH` the render waits its turn there; without it the render runs directly.

## In a game's own code

The same tracer is a library: a game or app stages a scene with `pfx_trace::stage::scene` and traces it with `pfx_trace` ([Tracer cameras and staging](trace.md)), and `pfx bake` uses it for the light the live renderer shows ([Probe baking](bake.md)).
