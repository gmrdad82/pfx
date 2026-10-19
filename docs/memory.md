# Memory

pfx plans a render's memory before it loads anything heavy, refuses one that would pass a limit you give it, keeps the allocations it controls inside that limit while it runs, and prints the peak it reached beside the plan, so the limit can be set from numbers instead of guessed.

## Setting the limit

`--memory-limit <GB>` on `pfx shot` (a recipe's still and a scene file's frames), `pfx clip` (live and `--trace`), `pfx render --recipe`, `pfx shoot`, `pfx film` and `pfx bake`, or `PFX_MEMORY_LIMIT=<GB>` in the environment; the flag wins. A GB is 2³⁰ bytes, as `prlimit` and the summaries count, and may be a fraction (`7.5`). `pfx shoot` hands its limit to every `pfx shot` it starts.

The limit holds on each side on its own: the process's resident memory on the CPU, and its memory on the GPU (the driver's VRAM plus GTT for the process). A plan past it on either side is refused in one line, exit 1, before the scene is staged:

```
pfx render: the run plans 5.2 GB of memory while staging the scene, past the 4.0 GB memory limit: the staged pieces 3.2 GB (54.7 M placed triangles), the built scene 1.4 GB (54.7 M placed triangles), the textures 0.5 GB (4 maps up to 4096 px); fewer or lighter objects, smaller texture maps, or raise the memory limit
```

It names the phase that peaks, its three biggest parts, and what lowers each.

### Why not `prlimit --as`

An address-space cap counts what the GPU driver maps into the process: a run at 9 GB resident needs 10 to 16 GB of `--as`, and a cap tight enough to protect the machine aborts a healthy run mid-allocation. `--memory-limit` counts resident memory, the number the machine runs out of, and refuses before the run starts instead of aborting inside it. Run without `--as` when you give pfx a limit.

## The plan

pfx counts the scene first, from what the build already holds, without decoding or uploading anything (`pfx_core::memory::Census`):

- **placed triangles**: every drawn copy of every mesh;
- **distinct triangles**: each mesh once: a piece whose triangles, UVs and slots match an earlier one and whose positions are an affine copy of it is the stager's shared mesh, counted once; a scene file's meshes are counted once a geometry;
- **the built scene**: the bytes the build's meshes and encoded images hold now;
- **texture maps**: their number and texels at full size, from their headers;
- **the HDRI**: its size, from its file's header.

Then it weighs them for the run (`pfx_core::memory::plan`). A run goes through phases that free what the next one doesn't need, so the plan is the largest phase on each side, not the sum:

| Run | Phase | CPU | GPU |
| --- | --- | --- | --- |
| recipe live (`pfx shot --still`, `pfx clip`) | drawing | 430 MB + the built scene + 64 B a placed triangle + 14.2 B a texel + 16 B an HDRI texel | 640 MB + 110 B a placed triangle + 14.2 B a texel + 18 B an HDRI texel + 225 B a pixel |
| traced still (`pfx render --recipe`) | staging the scene | 110 MB + the built scene + 62 B a placed triangle + 7.7 B a texel + 12 B a pixel | none |
| | uploading to the GPU | 110 MB + 680 B a distinct triangle + 8.3 B a texel + 35 B an HDRI texel | 650 MB + 625 B a distinct triangle + 8.8 B a texel |
| | tracing | as uploading, + 12 B a pixel, or 70 B without the denoiser | 650 MB + 308 B a distinct triangle + 6.5 B a texel + 121 B a pixel |
| | the denoiser, when on | + 55 MB while uploading and tracing | + 290 MB while uploading and tracing, + 307 B a pixel while tracing |
| traced clip (`pfx clip --trace`) | uploading, tracing | as the still, + the built scene + 62 B a placed triangle beyond the distinct ones + 100 MB, held for the whole clip | as the still |
| scene file (`pfx shot <scene.pitofx>`, `pfx shoot`) | staging the scene | the loaded scene, read when it is planned, + 330 MB + 14 B a placed triangle + 13 B a texel | 730 MB + 160 B a distinct triangle + 13.8 B a texel + 18 B an HDRI texel + 182 B a pixel |
| | the scene reflections' capture, when on | + 196 B a placed triangle + 420 B a distinct triangle | + 420 MB + 630 B a distinct triangle + 8 B a texel |
| bake plates (`pfx bake`) | baking the plate | 150 B a placed static triangle + 480 B a distinct triangle + 96 B a plate texel | |

A texel is one texel of a map at full size; live draws every map at the size of the largest map of its kind (4096 at most), so its texels are counted that way. The tracer's staging copies every placed piece, then keeps one copy of each distinct mesh; the live recipe view draws every placed piece; a scene file is drawn as instances of its distinct meshes, while its reflections' capture builds a world-space BVH of every placed triangle on the CPU.

The fixed parts are the process, the GPU driver and the renderers' own buffers. The weights were measured on generated streets (below) with the driver's shader cache warm, then raised about 7% so the plan lands above the peak.

What the plan can't see:

- **Texture fitting.** The tracer shrinks each map to the level the frame needs while it stages, which depends on how big the map's surfaces are on screen; the plan counts every map at full size. A traced scene whose textured objects are small on screen uses less GPU memory than planned: about 30% less in the test with 30 small textured cars at 4K, and nearly half at 1080p.
- **A cold shader cache.** The first traced run after an update of pfx or the GPU driver compiles the tracer's shaders, which takes about 1.9 GB more on the CPU for a few seconds on RADV. Render one small frame first (`pfx render --recipe <x.pitofx> --camera <c> --size 64x36 --spp 1 --out tmp/warm.png`) before a run that has to stay under a tight limit.
- **Reading a scene file.** A scene file is read whole before pfx can count it, so its part is measured, not weighed: the plan reads the process's resident memory once the scene is loaded, and refuses before the scene is staged. Reading is itself heavy when one mesh is placed many times, about 400 B a placed copy's triangle in the streets, and `--set` on a scene shot reads it once more; pfx's own allocations in it are still held to the limit while it reads.
- **Denoise off.** A traced still without the denoiser plans up to 15% above its peak at 1080p.

## While it runs

The allocations pfx sizes itself go through the same limit (`pfx_core::memory::vec`, `filled` and `more`): before reserving 1 MiB or more, each checks the process's resident memory plus the request against the limit and refuses in one line, `the tracer's bvh nodes needs 265991040 bytes: past the 12.0 GB memory limit with 11.8 GB in use`, where an unchecked allocation would abort the process. They cover the tracer's BVH, its triangle and texture records, the shrunk textures, the HDRI's copies, the image decoders and the scene reflections' triangles and BVH. The tracer also checks its GPU buffers against the driver's figure before it creates them. What other libraries allocate (the GPU driver, the scene reader) is not checked; the plan covers it.

## The report

Every run ends with the actual peaks beside the plan, on stderr:

```
pfx render --recipe: 1920x1080, 4.0 samples a pixel on average, …; peak memory 4.87 GB on the CPU, 2.13 GB on the GPU (0.62 GB in the tracer's buffers), planned 5.21 GB on the CPU, 2.27 GB on the GPU
pfx shot: peak memory 5.67 GB on the CPU, 7.22 GB on the GPU, planned 6.13 GB on the CPU, 7.69 GB on the GPU
```

The CPU peak is the kernel's high-water mark of resident memory (`VmHWM`); the GPU peak is the most the driver reported for the process (`drm-memory-vram` plus `drm-memory-gtt` in `/proc/self/fdinfo`), sampled every 100 ms and at each frame. `pfx render --recipe --progress json` adds `cpu_peak_gb`, `gpu_gb`, `trace_gb`, `planned_cpu_gb`, `planned_gpu_gb` and `memory_limit_gb` to its `done` line; a scene shot's and a film's `manifest.json` carry a `memory` object with `limit_gb`, `planned_cpu_gb`, `planned_gpu_gb`, `cpu_peak_gb` and `gpu_peak_gb`. Where the system doesn't tell (`/proc` is Linux's) a peak reads `unknown` or `null`, and the limit is held by the plan alone.

To set a limit from numbers: run once without one, read the peak, and give the plan's figure or the peak plus a tenth, whichever is larger.

## Measured

Generated streets on the RX 9060 XT (RADV), each run its own `pgpu run --class bench` job, the peaks sampled from outside every 50 ms: cars of 183,072 triangles on 3 meshes (or 30 distinct ones), each with a 4096² base map, a 4096² ground map, a 4096×2048 HDRI.

| Street | Run | Size | Peak CPU | Planned | Peak GPU | Planned |
| --- | --- | --- | --- | --- | --- | --- |
| 30 cars on 3 meshes | live still | 1920×1080 | 1.79 GB | 1.91 GB (+6%) | 2.46 GB | 2.65 GB (+8%) |
| 30 cars on 3 meshes | live still | 3840×2160 | 1.82 GB | 1.91 GB (+5%) | 3.72 GB | 3.95 GB (+6%) |
| 90 cars on 3 meshes | live still | 1920×1080 | 2.71 GB | 2.85 GB (+5%) | 3.46 GB | 3.77 GB (+9%) |
| 90 cars on 3 meshes | live still | 3840×2160 | 2.70 GB | 2.85 GB (+5%) | 4.72 GB | 5.07 GB (+7%) |
| 300 cars on 3 meshes | live still | 1920×1080 | 5.68 GB | 6.13 GB (+8%) | 7.22 GB | 7.69 GB (+7%) |
| 300 cars on 3 meshes | live still | 3840×2160 | 5.71 GB | 6.13 GB (+7%) | 8.22 GB | 9.00 GB (+9%) |
| 30 distinct cars | live still | 1920×1080 | 7.48 GB | 7.95 GB (+6%) | 7.89 GB | 8.64 GB (+9%) |
| 30 distinct cars | live still | 3840×2160 | 7.44 GB | 7.95 GB (+7%) | 9.14 GB | 9.94 GB (+9%) |
| 30 distinct cars, 256² maps | live still | 1920×1080 | 0.97 GB | 1.04 GB (+7%) | 1.63 GB | 1.79 GB (+10%) |
| 30 distinct cars, 256² maps | live still | 3840×2160 | 0.96 GB | 1.04 GB (+9%) | 2.89 GB | 3.09 GB (+7%) |
| 30 small distinct cars | live still | 1920×1080 | 6.89 GB | 7.49 GB (+9%) | 7.38 GB | 8.08 GB (+10%) |
| 30 small distinct cars | live still | 3840×2160 | 6.89 GB | 7.49 GB (+9%) | 8.63 GB | 9.39 GB (+9%) |
| 30 cars, 1024×512 HDRI | live still | 1920×1080 | 1.69 GB | 1.79 GB (+6%) | 2.35 GB | 2.52 GB (+7%) |
| 30 cars on 3 meshes | traced still | 1920×1080 | 1.21 GB | 1.32 GB (+9%) | 2.12 GB | 2.31 GB (+9%) |
| 30 cars on 3 meshes | traced still | 3840×2160 | 1.29 GB | 1.39 GB (+8%) | 4.45 GB | 4.79 GB (+8%) |
| 90 cars on 3 meshes | traced still | 1920×1080 | 1.91 GB | 2.00 GB (+5%) | 2.12 GB | 2.31 GB (+9%) |
| 90 cars on 3 meshes | traced still | 3840×2160 | 1.98 GB | 2.07 GB (+4%) | 4.45 GB | 4.79 GB (+8%) |
| 300 cars on 3 meshes | traced still | 1920×1080 | 4.88 GB | 5.21 GB (+7%) | 2.13 GB | 2.31 GB (+8%) |
| 300 cars on 3 meshes | traced still | 3840×2160 | 4.90 GB | 5.28 GB (+8%) | 4.45 GB | 4.79 GB (+8%) |
| 30 distinct cars | traced still | 1920×1080 | 7.53 GB | 7.94 GB (+5%) | 7.62 GB | 8.37 GB (+10%) |
| 30 distinct cars | traced still | 3840×2160 | 7.50 GB | 8.01 GB (+7%) | 8.56 GB | 8.94 GB (+4%) |
| 30 distinct cars, 256² maps | traced still | 1920×1080 | 3.63 GB | 3.94 GB (+9%) | 3.97 GB | 4.12 GB (+4%) |
| 30 distinct cars, 256² maps | traced still | 3840×2160 | 3.64 GB | 4.01 GB (+10%) | 5.44 GB | 5.81 GB (+7%) |
| 30 small distinct cars | traced still | 1920×1080 | 4.16 GB | 4.51 GB (+8%) | 2.81 GB | 5.21 GB (+85%) |
| 30 small distinct cars | traced still | 3840×2160 | 4.16 GB | 4.58 GB (+10%) | 5.24 GB | 7.39 GB (+41%) |
| 30 cars, 1024×512 HDRI | traced still | 1920×1080 | 1.01 GB | 1.08 GB (+7%) | 2.12 GB | 2.31 GB (+9%) |
| 30 cars on 3 meshes | traced, denoise off | 1920×1080 | 1.24 GB | 1.38 GB (+11%) | 1.32 GB | 1.50 GB (+14%) |
| 30 cars on 3 meshes | traced, denoise off | 3840×2160 | 1.69 GB | 1.79 GB (+6%) | 1.98 GB | 2.13 GB (+8%) |
| 30 cars on 3 meshes | live clip | 1920×1080 | 1.80 GB | 1.91 GB (+6%) | 2.46 GB | 2.65 GB (+8%) |
| 30 cars on 3 meshes | traced clip | 1920×1080 | 1.79 GB | 1.85 GB (+4%) | 2.12 GB | 2.31 GB (+9%) |
| 30 distinct cars, 256² maps | traced clip | 1920×1080 | 3.65 GB | 4.18 GB (+15%) | 3.87 GB | 4.12 GB (+6%) |
| 30 cars on 3 meshes | scene file | 1920×1080 | 4.22 GB | 4.72 GB (+12%) | 3.10 GB | 3.38 GB (+9%) |
| 30 cars on 3 meshes | scene file | 3840×2160 | 4.23 GB | 4.72 GB (+12%) | 4.10 GB | 4.43 GB (+8%) |
| 90 cars on 3 meshes | scene file | 1920×1080 | 10.44 GB | 10.96 GB (+5%) | 3.10 GB | 3.38 GB (+9%) |
| 90 cars on 3 meshes | scene file | 3840×2160 | 10.44 GB | 10.96 GB (+5%) | 4.10 GB | 4.43 GB (+8%) |
| 30 distinct cars | scene file | 1920×1080 | 9.38 GB | 10.83 GB (+15%) | 14.96 GB | 16.20 GB (+8%) |
| 30 distinct cars | scene file | 3840×2160 | 9.38 GB | 10.83 GB (+15%) | 15.96 GB | 17.26 GB (+8%) |
| 30 distinct cars, 256² maps | scene file | 1920×1080 | 3.80 GB | 4.06 GB (+7%) | 5.31 GB | 5.68 GB (+7%) |
| 30 distinct cars, 256² maps | scene file | 3840×2160 | 3.80 GB | 4.06 GB (+7%) | 6.31 GB | 6.74 GB (+7%) |
| 30 small distinct cars | scene file | 1920×1080 | 6.82 GB | 6.95 GB (+2%) | 11.57 GB | 12.21 GB (+6%) |
| 30 small distinct cars | scene file | 3840×2160 | 6.82 GB | 6.95 GB (+2%) | 12.52 GB | 13.27 GB (+6%) |

A scene file's plan reads the resident memory once the scene is loaded; the table's scene rows use that figure as the runs printed it. The streets are made by a script that writes the cars' GLBs, their PNG maps, the HDRI and the recipe; it is not part of pfx.
