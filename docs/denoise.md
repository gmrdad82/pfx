# The denoiser

`pfx-denoise` is Intel Open Image Denoise's RT filter, ported from OIDN 2.5.1's source to Rust and WGSL compute on wgpu. It takes a path-traced HDR colour image with its albedo and normal, as `pfx-trace` writes them, and returns the denoised colour. It runs the trained weights OIDN ships for that filter (`rt_hdr_alb_nrm.tza`, OIDN's default quality), loaded as they are; it links no C++ and adds no dependency beyond wgpu and bytemuck. Its credits and licence are in [NOTICE.md](../NOTICE.md) and [INSPIRATION.md](../INSPIRATION.md); the package is licensed with pfx ([Licence](../README.md#licence)), and the parts ported from OIDN and its weights keep the Apache License 2.0 notice in [crates/denoise/LICENSE-APACHE](../crates/denoise/LICENSE-APACHE).

## The API

```rust
use pfx_denoise::Denoiser;

let mut denoiser = Denoiser::new(&gpu)?;
let clean: Vec<[f32; 4]> = denoiser.denoise(&gpu, width, height, &color, &albedo, &normal)?;
let clean = denoiser.denoise_textures(&gpu, &trace.color.texture, &trace.albedo.texture, &trace.normal.texture)?;
denoiser.denoise_textures_into(&gpu, &trace.color.texture, &trace.albedo.texture, &trace.normal.texture, &target)?;
```

- `Denoiser::new(&gpu)` parses the weights and uploads them. The first call at a new image size builds the pipelines for it (about 0.2 to 0.6 s); later calls at that size reuse them.
- `denoise` takes and returns row-major RGBA `f32` pixels. The alpha channel passes through from `color`; the network sees RGB only.
- `denoise_textures` and `denoise_textures_into` read `Rgba32Float` textures with `COPY_SRC`, which is what `Trace::color`, `Trace::albedo` and `Trace::normal` are, so a trace feeds the denoiser with GPU copies and no readback. `denoise_textures` reads the result back; `denoise_textures_into` copies it into an `Rgba32Float` texture with `COPY_DST` and returns without waiting.
- Normals may be in any space, in [-1, 1]; albedo in [0, 1]. Colour is linear HDR; its exposure is found the way OIDN finds it.

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

