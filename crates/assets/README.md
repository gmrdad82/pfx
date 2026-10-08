# pfx render: assets

pfx's asset renderer, the workspace package `pfx-assets`, run through the `pfx` tool. It path-traces a product's logos, wordmarks, lockups, app tiles and icon sets from the product's own SVG masters and small icon specs, alone or as scenes of many, as stills or 60 fps clips. It runs on the engine's own tracer (`pfx_trace`), headless, under a memory cap.

```
pfx render --asset logo --product <product> --size 2560
pfx render --asset tile --product <product> --size 1024 --derive 512,256,128,64,32
pfx render --asset lockup --product <product> --size 3840x1080
pfx render --icons shapes --product sample --recipes crates/assets/tests/recipes --size 512
pfx render --icons shapes --product sample --only dot --size 512 --draft
pfx render --asset logo --product <product> --size 2048 --web 256
pfx list --presets [--recipes <dir>]
pfx list --scenes [--product <product>] [--recipes <dir>]
pfx render --scene shapes --product sample --still --size 3840x2160
pfx render --scene <shot> --product <product> --clip --size 1080p
pfx encode --manifest tmp/renders/<product>/clip/<product>-clip-<shot>-1920x1080-60fps.json
pfx render --scene wall --product <product> --still --use 'tile:*' --count 10 --layout scatter --jitter 0.4
pfx render --asset logo --product <product> --turn 30 --tilt 20 --zoom 1.4
```

`<product>` is the product's slug: lowercase letters, digits and `-`. pfx keeps no list of products.

## Where the recipes come from

Each product keeps its recipes in its own repository, under `render/assets/`:
- `presets/<product>.toml`, the look;
- `icons/<product>/<set>.toml`, one file per icon set;
- `shots/<product>.toml`, its shots (camera, keyframes, instances and scenes).

pfx finds that folder, in order:
1. `--recipes <dir>`;
2. `PFX_RECIPES`;
3. `<git toplevel of the working directory>/render/assets`.

A file that isn't there is an error naming the full path pfx looked for. `--preset-file` and `--shot-file` replace one file. Unknown keys are refused, and the message lists the keys that are allowed.

**Shots across products.** Every product a shot uses is read from the same folder, the caller's own. A shot file may list them to set the order `*` expands in:

```toml
[products.other]

[products.sample]
```

- An entry takes no keys: pfx fetches no other repository, and `--recipes` takes one folder.
- `use = "logo:*"` expands over the listed products in file order, then the shot's own `--product`.

**Masters.** A logo, wordmark, lockup, tile or card reads its SVG master from the caller's own copy, `logos/<product>.svg`, `wordmarks/<product>.svg` or `lockups/<product>.svg`, in the folder named by `--assets <dir>`, then `PITO_ASSETS`, then `<git toplevel of the working directory>/render/marks`. A missing master is an error naming the path it looked for.

**Pinned and recorded.** The manifest's `recipes` lists every recipe the render read, each with its `path`, `repo`, `commit`, `sha256` and whether it is `pinned`. Each `assets[]` entry carries its `recipe` (the preset) and, for an icon, its `icon_set`. A render is pinned only when every recipe is committed and unmodified in its repository.

**The engine's own tests** read only `crates/assets/tests/recipes/`, a neutral `sample` product with one preset, one icon set (`shapes`) and one shot (`shapes`), and the neutral marks in `crates/assets/tests/marks/`. Renders of `sample` are never archived.

## Flags and quality

Common flags: `--samples` (the adaptive cap), `--draft`, `--angle front|three-quarter|top`, `--background transparent|#rrggbb|hdri`, `--shadow on|off`, `--supersample`, `--set material.roughness=0.2`, `--recipes`, `--preset-file`, `--assets <dir>`, `--out`, `--class`.

Every pixel samples adaptively until it is clean: by default to a noise threshold of 0.01 with a cap of 4096 samples, about 1,700 samples a pixel on metal. `--draft` stops at 0.03 with a cap of 256, for quick looks. `--samples N` sets the cap.

## Output

Output lands in `tmp/renders/<product>/<kind>/` with a `.json` manifest beside each output. Clip masters are x264 CRF 12 on the CPU with a poster, from the ffmpeg build pinned in `crates/run/render.toml`. Stills keep 16-bit PNG masters and dithered 8-bit PNG deliveries.

When an archive root is set (`PITO_ARCHIVE`), deliberate masters, posters, a still's PNG deliveries (with its `--derive` sizes and `--web` copies) and their `manifest.json` are copied into a dated leaf, `$PITO_ARCHIVE/Masters/pfx/<product>/<shot>/<YYYY-MM-DD>_<HHMMSS>_<version>_<caller>/`, with one line appended to `$PITO_ARCHIVE/Masters/catalog.jsonl`. An offline archive is queued and `pfx archive --retry` finishes it. `--no-archive` skips the step.

`pfx --version` names the build, and so does the manifest's `tool.version`. A render needs a GPU wgpu can open (Vulkan on Linux). When the `pgpu` GPU queue is on the `PATH`, renders wait their turn on it and take a `pgpu turn` at least every 50 ms of GPU work; without it they run at once, paced the same way, and no GPU submission runs longer than a few milliseconds.
