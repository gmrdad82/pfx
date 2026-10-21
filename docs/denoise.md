# The denoiser

`pfx-denoise` is Intel Open Image Denoise's RT filter, ported from OIDN 2.5.1's source to Rust and WGSL compute on wgpu. It takes a path-traced HDR colour image with its albedo and normal, as `pfx-trace` writes them, and returns the denoised colour. It runs the trained weights OIDN ships for that filter (`rt_hdr_alb_nrm.tza`, OIDN's default quality, and for clean features `rt_alb.tza`, `rt_nrm.tza` and `rt_hdr_calb_cnrm.tza`), loaded as they are; it links no C++ and adds no dependency beyond wgpu and bytemuck. Its credits and licence are in [NOTICE.md](../NOTICE.md) and [INSPIRATION.md](../INSPIRATION.md); the package is licensed with pfx ([Licence](../README.md#licence)), and the parts ported from OIDN and its weights keep the Apache License 2.0 notice in [crates/denoise/LICENSE-APACHE](../crates/denoise/LICENSE-APACHE).

## The API

```rust
use pfx_denoise::Denoiser;

let mut denoiser = Denoiser::new(&gpu)?;
let clean: Vec<[f32; 4]> = denoiser.denoise(&gpu, width, height, &color, &albedo, &normal)?;
let clean = denoiser.denoise_textures(&gpu, &trace.color.texture, &trace.albedo.texture, &trace.normal.texture)?;
denoiser.denoise_textures_into(&gpu, &trace.color.texture, &trace.albedo.texture, &trace.normal.texture, &target)?;
```

- `Denoiser::new(&gpu)` parses the weights and uploads them. `Denoiser::with_aux(&gpu, aux)` picks how the albedo and normal are used ([Clean auxiliary features](#clean-auxiliary-features)); `new` is `Aux::Noisy`. The first call at a new image size builds the pipelines for it (about 0.2 to 0.6 s); later calls at that size reuse them.
- `denoise` takes and returns row-major RGBA `f32` pixels. The alpha channel passes through from `color`; the network sees RGB only.
- `denoise_textures` and `denoise_textures_into` read `Rgba32Float` textures with `COPY_SRC`, which is what `Trace::color`, `Trace::albedo` and `Trace::normal` are, so a trace feeds the denoiser with GPU copies and no readback. `denoise_textures` reads the result back; `denoise_textures_into` copies it into an `Rgba32Float` texture with `COPY_DST` and returns without waiting.
- Normals may be in any space, in [-1, 1]; albedo in [0, 1]. Colour is linear HDR; its exposure is found the way OIDN finds it.

## A traced frame

The denoiser runs once on a frame of `pfx render --recipe` (a `.png` or master by default, an `.exr` or `.hdr` with `--denoise on`) or of `pfx clip --trace` (by default), after its last sample. It is OIDN's RT filter (the `rt_hdr_alb_nrm` weights, OIDN's default quality, the only one pfx ships), run on the beauty with its albedo and normal guides, all three as the tracer averaged them, from its textures on the GPU. The guides are not prefiltered first (OIDN's `cleanAux` path, which denoises the albedo and normal before the beauty, is not used), and the filter runs at the one quality whatever the samples. Its output then goes through the recipe's finish. `--denoise off` skips it, and `pfx clip --trace --passes <dir>` writes the three inputs as 32-bit float EXR into a folder of their own ([recipe-clips.md](recipe-clips.md#denoise-and-the-raw-passes)), so a frame can be compared denoised against raw, or denoised again elsewhere, from the same inputs.

## What is ported

Everything that decides the output follows OIDN's source:

- **The network**: the U-Net of `UNetFilter::addUNet` (`core/unet_filter.cpp`), five encoder convolutions with 2×2 max pooling after the first four, a two-convolution bottleneck, four decoder stages that upsample by nearest neighbour and concatenate the matching encoder output, and a three-channel output convolution. Every convolution is 3×3 with zero padding and ReLU. The `.tza` reader (`core/tza.cpp`) loads its half-precision weights.
- **Input processing** (`devices/gpu/gpu_input_process.h`): colour scaled by the autoexposure, NaNs zeroed, clamped and put through OIDN's PU transfer function with its constants and normalisation; albedo clamped to [0, 1]; normals clamped to [-1, 1] and mapped to [0, 1]; channels padded and the tile placed in a buffer aligned to 16 pixels.
- **Autoexposure** (`devices/gpu/gpu_autoexposure.h`): the mean luminance of 16×16 bins, the geometric mean of the bins above 1e-8, and `0.18` over it.
- **Output processing** (`devices/gpu/gpu_output_process.h`): the inverse transfer and the inverse exposure.
- **Tiling** (`UNetFilter::init` and `execute`): OIDN's own split, tiles of at most 2160×2160 pixels overlapping by 96, so a 4K frame is two tiles and a 5120×2880 frame four, as in OIDN; smaller frames are one tile. The split is kept because a 5K frame in one piece needs a 3.8 GB activation buffer and over 6 GB in all.

The convolution kernels are pfx's own: each workgroup of 64 threads computes a 32×16-pixel block for eight output channels, staging the input block with its one-pixel border in workgroup memory one four-channel slice at a time; each thread accumulates 4×2 pixels in `f32`. Pooling, upsampling and concatenation are fused into the convolutions that use them. Activations are stored as `f32` in blocks of four channels. The work is cut into submissions of at most 24 billion multiply-adds each (a few milliseconds on a desktop GPU), so no submission holds the GPU for long.

## Equal to OIDN

`crates/denoise/examples/compare.rs` renders three neutral scenes at 16 samples per pixel with `pfx-trace` and writes their colour, albedo and normal as PFM: the desk (`crates/bake/tests/plates/desk.scene.toml`, 640×360), the room (`crates/load/tests/scenes/room.scene.toml`, 640×360) and the deep glass of `pfx-trace`'s tests (four stacked glass panes over a rippled liquid with smooth-union volumes, orthographic from above, 640×400). OIDN 2.5.1's own `oidnDenoise` (the official Linux release, CPU device, default quality) denoised the same files, and the example compares:

| Scene | Max abs. difference (linear) | Mean abs. difference | Max difference in sRGB display values | PSNR in display space |
|---|---|---|---|---|
| desk | 6.5e-5 | 4.0e-7 | 3e-6 (0.0008 of 255) | 129.3 dB |
| room | 8e-6 | 4.6e-7 | 3e-6 | 128.8 dB |
| deep glass | 5e-6 | 2.6e-7 | 3e-6 | 130.3 dB |

The tolerance is a gap of at most 1e-3 × (1 + |OIDN's value|) per channel. The GPU test `the_denoiser_matches_oidn_on_the_desk` (`crates/denoise/tests/oidn.rs`) holds it on a 32×24 crop of the desk against OIDN's stored output (`tests/fixtures/desk.oidn.pfm`); it measures 2.2e-6. The differences are float rounding: OIDN on the CPU sums in a different order.

To repeat it, render the inputs with `cargo run --release -p pfx-denoise --example compare -- stage <dir> 640 16`, run `oidnDenoise -d cpu --hdr <name>.color.pfm --alb <name>.albedo.pfm --nrm <name>.normal.pfm -o <name>.oidn.pfm` for each name, then `compare denoise <dir> <name>`.

## Clean auxiliary features

A path tracer's albedo and normal are only as clean as its samples. Where motion blur, depth of field, hair or glass mix several surfaces into one pixel, the albedo and normal are noisy too, and the default weights (`rt_hdr_alb_nrm`, trained on noisy features) treat them as a weak hint: edges the features blur, the colour keeps blurred. OIDN's answer is to prefilter the features and use the weights trained on clean ones, and `pfx-denoise` ports it:

```rust
use pfx_denoise::{Aux, Denoiser};

let mut denoiser = Denoiser::with_aux(&gpu, Aux::Prefiltered)?;
let clean = denoiser.denoise_textures(&gpu, &trace.color.texture, &trace.albedo.texture, &trace.normal.texture)?;
```

| `Aux` | What runs | Weights |
|---|---|---|
| `Noisy` (`Denoiser::new`) | the colour network, the features as they are | `rt_hdr_alb_nrm.tza` |
| `Prefiltered` | the albedo network, then the normal network, then the colour network on their outputs | `rt_alb.tza`, `rt_nrm.tza`, `rt_hdr_calb_cnrm.tza` |
| `Clean` | the colour network, the features taken as noise-free (from a pass of their own, or already filtered) | `rt_hdr_calb_cnrm.tza` |

`Prefiltered` is OIDN's documented prefiltering, the RT filter run on the albedo alone and on the normal alone before the colour, with `cleanAux` set. It follows OIDN's source as the colour pass does:

- **The networks** are the same U-Net with three input channels instead of nine (`UNetFilter::getWeights` picks `rt_alb` and `rt_nrm` for a filter given only an albedo or only a normal; `rt_hdr_calb_cnrm` for colour with `cleanAux`).
- **The albedo pass** (`RTFilter::newTransferFunc`, `gpu_input_process.h`, `gpu_output_process.h`): NaNs zeroed, clamped to [0, 1] and put through the sRGB transfer function on the way in; the inverse sRGB transfer and a clamp to 1 on the way out. No exposure.
- **The normal pass**: clamped to [-1, 1] and mapped to [0, 1] in, a linear transfer, mapped back to [-1, 1] and clamped out (the `snorm` path).
- **Order**: every tile of the albedo, then every tile of the normal, then the colour's tiles, so a colour tile reads prefiltered features its neighbours' tiles wrote. The three networks share one activation arena and the pipelines their identical layers generate, so prefiltering adds two image buffers the size of the colour image (16 bytes a pixel, about 133 MB each at 3840×2160) and no tile memory.

The weights are OIDN's base models (its `balanced` quality). At its default `high` quality OIDN picks larger models for clean features (`rt_alb_large`, `rt_nrm_large`, `rt_hdr_calb_cnrm_large`), about four times the size; pfx does not ship them.

Prefiltering runs three networks of the same size instead of one (only their first layers differ), so it does about three times the work of the denoise times under [Speed](#speed), which are measured with `Aux::Noisy`. The three clean-feature weights add about 5.5 MB to a binary that links `pfx-denoise`.

The GPU test `the_clean_aux_denoiser_matches_oidn_on_the_desk` (`crates/denoise/tests/oidn.rs`) checks the same tolerance as the noisy one on the same 32×24 desk crop, twice: `Prefiltered` from the noisy features against OIDN's three-step output, and `Clean` given OIDN's own prefiltered albedo and normal against the same image. OIDN 2.5.1's `oidnDenoise` (the official Linux release, CPU device, `-q balanced`) wrote the references: `--alb desk.albedo.pfm -o desk.albedo.oidn.pfm`, `--nrm desk.normal.pfm -o desk.normal.oidn.pfm`, then `--hdr desk.color.pfm --alb desk.albedo.oidn.pfm --nrm desk.normal.oidn.pfm --clean_aux -o desk.prefiltered.oidn.pfm`. That crop is one tile, so `tiled_denoising_matches_one_tile_in_every_mode` (`crates/denoise/src/lib.rs`, GPU) denoises a generated 1200×1200 image with sharp albedo and normal edges in four tiles and in one, in every mode, and holds the tiled result to the same tolerance against the single tile.

## Samples for a film-quality frame

How many samples a traced frame needs once denoised, measured once with pfx 3.1.3 on an AMD Radeon RX 9060 XT (the GPU seconds are that card's) on pfx's street template (`pfx new <out> --from street`: a low sun, analytic sky, height fog, f/5.6 depth of field) as a 2160×1200 crop of a 2160×3840 still, with `Aux::Noisy`. The reference is 1024 samples undenoised. "Display" error goes through ×1.5 exposure, a clamp and a 1/2.2 power; "against truth" removes the reference's own noise (its variance taken to fall as 1/spp); the last column compares each frame with the 256-sample adaptive (threshold 0.01) denoised frame.

| frame | mean spp | GPU s | against the 1024 reference (display PSNR, relMSE) | against truth (display) | against the 256 adaptive frame (display PSNR) |
|---|---:|---:|---|---:|---:|
| raw 256 | 256 | 74.3 | 37.07 dB, 1.51e-3 | | |
| 256, threshold 0.01, denoised | 244.2 | 84.9 | 42.00 dB, 1.26e-4 | 46.2 dB | — |
| 256 fixed, denoised | 256 | 85.9 | 41.95 dB, 1.26e-4 | 46.2 dB | |
| 128 fixed, denoised | 128 | 42.9 | 41.93 dB, 1.37e-4 | 46.1 dB | 55.7 dB |
| **64 fixed, denoised** | 64 | 21.5 | 41.86 dB, 1.51e-4 | **45.9 dB** | **52.2 dB** |
| 32 fixed, denoised | 32 | 10.7 | 41.63 dB, 1.81e-4 | 45.3 dB | 49.1 dB |
| 16 fixed, denoised | 16 | 5.4 | 41.23 dB, 2.22e-4 | 44.4 dB | 46.6 dB |
| 8 fixed, denoised | 8 | 3.1 | 40.52 dB, 2.75e-4 | 43.1 dB | 44.0 dB |
| adaptive 0.02, cap 256 | 198.6 | 78.3 | 41.42 dB, 1.30e-4 | | |
| adaptive 0.05, cap 256 | 98.0 | 52.4 | 40.20 dB, 1.48e-4 | | 43.1 dB |
| adaptive 0.01, cap 64 | 63.8 | 21.8 | 41.86 dB, 1.51e-4 | | |

Film quality here means within one 8-bit level RMS (48 dB in display terms) of the 256-sample adaptive frame. 32 samples pass (49.1 dB) and 64 pass with margin (52.2 dB, 0.3 dB under 256 against truth), so a film final takes 64 samples a pixel and a film draft 32. Adaptive sampling gains nothing on this scene: at 0.01 it runs nearly every pixel to the cap, and at 0.05 it is cheaper but scores below a fixed 64 in display terms, because its linear metric stops pixels the exposure later lifts.

The still has no motion blur, so its features are cleaner than a clip's: where motion blur and defocus mix surfaces in a pixel, the features are noisy, and that is what `Aux::Prefiltered` is for.

## Speed

On an AMD Radeon RX 9060 XT (RDNA 4, RADV), from `compare bench <width> <height>`, warm, with the pipelines built:

| Frame | Tiles | Textures in, texture out | `denoise` with upload and readback |
|---|---|---|---|
| 1920×1080 | 1 | 96 ms | 110 ms |
| 3840×2160 | 2 of 2016×2160 | 274 ms | 452 ms |
| 5120×2880 | 4 of 2656×1536 | 506 ms | 732 ms |

The first call at a size adds the pipeline builds: 192 ms at 1080p, 646 ms at 4K and 908 ms at 5K in all.

## Budgets

A frame of `pfx render` is traced adaptively and then denoised: a final to a noise threshold of 0.01 with 16 to 64 samples a pixel, a draft to 0.05 with 6 to 12 (the Sobol sampler reaches the noise PCG had at 32 to 128 and 8 to 16, docs/trace.md). The budgets on the RX 9060 XT are a 4K draft under 10 s, a 4K final under 2 minutes and a 5K final under 4 minutes, from the start of the trace to the denoised image in memory (the trace's buffers, every sample, the denoiser's weights and pipelines, the denoise and its readback). `crates/denoise/tests/budgets.rs` (`the_desk_renders_within_the_budgets`, GPU) renders the neutral desk (`crates/bake/tests/plates/desk.scene.toml`) at each and fails over budget. It is a timing, so it runs alone on the card, `PFX_DIRECT=1 pgpu run --class bench -- cargo test --release -p pfx-denoise --test budgets -- --ignored`, and the GPU gate skips it. It renders each frame twice, keeps the faster, and also fails a frame more than 5% slower than its line in `crates/denoise/tests/baseline.toml`, the times the last release measured (v0.27.0: 2.7 s, 15.3 s and 29.6 s); a release that changes them updates that file in the same commit:

| frame | size | time | budget | mean samples |
|---|---|---|---|---|
| 4K draft | 3840 × 2160 | 5.8 s | 10 s | 16 |
| 4K final | 3840 × 2160 | 40.4 s | 120 s | 128 |
| 5K final | 5120 × 2880 | 71.5 s | 240 s | 128 |

Before the tracer started at 64 lanes and aimed its bands at 10 ms, the same 4K frame at 1024 samples took about 169 minutes; a 1024-sample final now takes about 5 minutes, and the denoiser lets a final stop at 128. The figures are with the card to itself; the first run of these budgets, on a card shared with another job, read 6.2 s, 100.6 s and 198.4 s.

