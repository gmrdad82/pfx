# Inspiration

Code pfx ported from other projects, with where it came from and under which licence. [NOTICE.md](NOTICE.md) lists the media and the licence texts that ship with it.

| pfx | Ported from | Version | Licence |
|---|---|---|---|
| `crates/denoise` (`pfx-denoise`) | [Intel Open Image Denoise](https://github.com/RenderKit/oidn): the RT filter's U-Net (`core/unet_filter.cpp`), the `.tza` weights reader (`core/tza.cpp`), the PU transfer function (`core/color.h`), input and output processing (`devices/gpu/gpu_input_process.h`, `devices/gpu/gpu_output_process.h`), autoexposure (`devices/gpu/gpu_autoexposure.h`) and tiling, translated to Rust and WGSL compute on wgpu; its trained weights `rt_hdr_alb_nrm.tza` ship unmodified | tag `v2.5.1`; weights from [oidn-weights](https://github.com/RenderKit/oidn-weights) commit `28883d1769d5930e13cf7f1676dd852bd81ed9e7` | Apache-2.0 ([crates/denoise/LICENSE-APACHE](crates/denoise/LICENSE-APACHE)) |

[pissang/oidn-web](https://github.com/pissang/oidn-web) (MIT), a WebGPU runtime for the same weights, was read for its channel-blocked layout; no code was taken from it.
