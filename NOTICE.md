# Notices

pfx's code is MIT licensed ([LICENSE](LICENSE)); its name and logos are not ([TRADEMARKS.md](TRADEMARKS.md)). The media files in this repository are covered as follows. [tests/agnostic/fixtures.txt](tests/agnostic/fixtures.txt) lists every font, image, SVG and audio file with its sha256 and where it came from, and the agnostic test keeps that list complete.

## The project's own files, under the MIT licence

Made for pfx by hand, by its own scripts and tests, or by its own renderers, and licensed with the code:

- `crates/assets/tests/marks/`: neutral sample marks (a grey disc and bar, the letters SAMPLE) for the asset renderer's tests.
- `crates/assets/tests/reference/`: reference renders of that sample product, kept as frozen data for the tracer's look check.
- `crates/bake/tests/data/`: the bake's neutral test rooms.
- `crates/editor/tests/scenes/` and `crates/editor/tests/shots/`: the editor's neutral scenes and the shots its offscreen harness takes of them.
- `crates/editor-doc/tests/data/` and `crates/editor-viewport/tests/data/`: the editor crates' TOML samples and their neutral sample scene, a stand with a block and a ball.
- `crates/game/examples/empty/world/`: the empty example's block.
- `crates/live/examples/bench/`: the bench's room.
- `crates/live/tests/flat/`: the flat pass's neutral reference scenes, made by `crates/live/tests/flat/scenes.py`.
- `crates/load/tests/data/`, `crates/load/tests/level/` and `crates/load/tests/scenes/`: minimal neutral meshes, images, skies and a sine tone.
- `crates/scene/tests/fixtures/` and `crates/scene/examples/demo/`: the scene format's neutral test meshes and image, and the demo project built from copies of them. The scene format, its checks, writer and tests began as pfx's own scene loader and were kept in the separate crate pfx-scene until v1.6.0 (commit `4b5e12d`), which moved back into this tree as `crates/scene` and `crates/file`.
- `docs/schema/`: the scene format's JSON Schemas, generated from `pfx-scene`'s types by `cargo run -p pfx-scene --features schema --example scene-schema -- docs/schema`.
- `library/`: the object library's models, `object.toml` files and thumbnails, original, baked by pfx's own generators from `library/sources.toml`, no third-party models or marks.
- `samples/`: pfx's own made-up sample recipes, their LUT and the chase-film sample's tapes.
- `tests/golden/` and `tests/fixtures/`: the recipe builder's golden scenes (cards, flatslab, table, toybox) with their GLBs and sidecars, built by `pfx build` from the recipes beside them, and the recipes' test sky `window.hdr`, made by pfx's tests (`tests/fixtures/sky/SOURCE`).
- `crates/glb/tests/golden/`, `crates/recipe-scene/tests/fixtures/`, `crates/build/tests/` and `crates/runtime/tests/`: the recipe crates' test meshes, images and skies, written by their own code. The recipe crates, their tests and these files began in a separate scene tool and moved into this tree at its commit `598059d`.
- `crates/play/tests/characters/`: blocks for the character tests.
- `crates/sound/tests/fixtures/`: sine tones made with ffmpeg.
- `crates/project/src/templates/grid.glb`: the floor grid of the scaffold's first scene.
- `docs/images/`: the README's and the docs' pictures, made from the engine's own neutral scenes; the mark they show is covered by [TRADEMARKS.md](TRADEMARKS.md):
  - `turntable.gif`: the sample shot `shapes`, traced by `pfx render --scene shapes --product sample --clip --recipes crates/assets/tests/recipes`, made a GIF with pcod's predecessor; `pcod <master> docs/images/<shot>.gif --size 1280 --fps 15` (pito-codecs) remakes it.
  - `desk.png` and `denoise.png`: the neutral desk (`crates/bake/tests/plates/desk.scene.toml`), traced by `crates/denoise/examples/rt_bench.rs` and denoised by `crates/denoise/examples/compare.rs`.
  - `live.gif`: the desk drawn by the live renderer, frames from `crates/live/examples/clip.rs`, made a GIF with pcod's predecessor; `pcod <master> docs/images/<shot>.gif --size 1280 --fps 15` (pito-codecs) remakes it.
  - `swarm.png`: the flat 2D pass under the swarm test's load, from `crates/live/examples/swarm.rs`.
  - `new.gif`: the terminal shot `start` in `render/terminal.toml`, recorded with prec and made a GIF with pcod's predecessor; `pcod <master> docs/images/<shot>.gif --size 1280 --fps 15` (pito-codecs) remakes it.
  - `editor.png`, `editor-drag.gif` and `editor-example.png`: the editor and `crates/editor-viewport/examples/editor.rs`, captured by the offscreen shot harness.
  - `live-frames.png`, `trace-speed.png`, `systems.png`, `size-binary.png`, `size-deps.png` and `size-build.png`: charts drawn with pito-compare from the measurements the README's "The numbers" section lists.

## Intel Open Image Denoise, under the Apache License 2.0

`crates/denoise` (the package `pfx-denoise`) is a port of Intel Open Image Denoise 2.5.1 (https://github.com/RenderKit/oidn, tag `v2.5.1`), Copyright 2018 Intel Corporation, licensed under the Apache License 2.0. Its text ships in [crates/denoise/LICENSE-APACHE](crates/denoise/LICENSE-APACHE); the package is MIT like the rest of pfx, and this notice and that licence go with the parts ported from OIDN and with its weights. The port is a translation into Rust and WGSL, changed by pfx's authors: the RT filter's U-Net layer graph (`core/unet_filter.cpp`), the weights reader (`core/tza.cpp`), the input and output processing with the PU transfer function and its constants (`core/color.h`, `devices/gpu/gpu_input_process.h`, `devices/gpu/gpu_output_process.h`), the autoexposure (`devices/gpu/gpu_autoexposure.h`, `devices/cpu/cpu_autoexposure.cpp`) and the tiling (`core/unet_filter.cpp`). The convolution kernels are pfx's own WGSL.

- `crates/denoise/weights/rt_hdr_alb_nrm.tza`: the trained weights of the RT filter for HDR colour with albedo and normal, unmodified from Intel's oidn-weights repository (https://github.com/RenderKit/oidn-weights, commit `28883d1769d5930e13cf7f1676dd852bd81ed9e7`, the one OIDN `v2.5.1` pins), Copyright Intel Corporation, under the Apache License 2.0 ([crates/denoise/LICENSE-APACHE](crates/denoise/LICENSE-APACHE)).
- `crates/denoise/tests/fixtures/desk.oidn.pfm`: generated by Intel Open Image Denoise 2.5.1's `oidnDenoise` (the official Linux x86_64 release, CPU device, default quality) from the three other files in that folder, which pfx's path tracer rendered from its neutral desk scene (`crates/bake/tests/plates/desk.scene.toml`) at 16 samples per pixel.

## Cycles, under the Apache License 2.0

`crates/trace/src/sobol.rs` and `crates/trace/src/sobol.wgsl` (in the package `pfx-trace`) are a port of Cycles' tabulated Sobol sampler (https://github.com/blender/cycles, tag `v5.2.0`, commit `3b97e190c5ff1a2ed2160d879ad5bf95bea7b8ba`), SPDX-FileCopyrightText: 2011-2022 Blender Foundation, licensed under the Apache License 2.0. Its text ships in [crates/trace/LICENSE-APACHE](crates/trace/LICENSE-APACHE); the package is MIT like the rest of pfx, and this notice and that licence go with the parts ported from Cycles. The port is a translation into Rust and WGSL, changed by pfx's authors: the 4D table generator after Helmer, Christensen and Kensler's "Stochastic Generation of (t, s) Sample Sequences" with its xor tables (`scene/tabulated_sobol.cpp`), the shuffled lookup (`kernel/sample/tabulated_sobol.h`), the Owen scramble (`kernel/sample/util.h`), the hashes (`util/hash.h`), the per-pixel seed and the table sizes (`kernel/sample/pattern.h`, `kernel/types.h`, `scene/integrator.cpp`). `crates/trace/src/levels.rs` and the instance traversal in `crates/trace/src/trace.wgsl` port, from the same tag, Cycles' BVH2 instance traversal (`kernel/bvh/traversal.h`, its `ENTRYPOINT_SENTINEL`) and its instance push and pop (`kernel/geom/object.h`), translated to Rust and WGSL and changed by pfx's authors.

## Filament, under the Apache License 2.0

`crates/post/src/flare.rs` and the `FLARE_*` shaders in `crates/post/src/wgsl.rs` (in the package `pfx-post`) are a port of Filament's screen-space lens flare (https://github.com/google/filament, tag `v1.56.0`, commit `003e5005713cba55cbd3f3a423c472000b96b868`), Copyright (C) 2016-2021 The Android Open Source Project, licensed under the Apache License 2.0. Its text ships in [crates/post/LICENSE-APACHE](crates/post/LICENSE-APACHE); the package is MIT like the rest of pfx, and this notice and that licence go with the parts ported from Filament. The port is a translation into Rust and WGSL compute, changed by pfx's authors: the ghosts, halo, ring and chromatic sampling of `filament/src/materials/flare/flare.mat` (after John Chapman's 2017 pseudo lens flare), the bloom input's threshold, highlight crush and firefly weights of `filament/src/materials/bloom/bloomDownsample2x.mat`, the flare's 9-tap blur and its 192-row buffer from `filament/src/PostProcessManager.cpp`, the starburst mask of `filament/src/materials/colorGrading/colorGrading.mat` with its 256-texel table, and the defaults of `BloomOptions` in `filament/include/filament/Options.h`.

`crates/materials/src/wgsl/lobes.wgsl` and `crates/materials/src/lobes.rs` (in the package `pfx-materials`) port parts of Filament (https://github.com/google/filament, tag `v1.78.0`), Copyright (C) 2015 The Android Open Source Project, licensed under the Apache License 2.0. Its text ships in [crates/materials/LICENSE-APACHE](crates/materials/LICENSE-APACHE); the package is MIT like the rest of pfx, and this notice and that licence go with the parts ported from Filament. The port is a translation into WGSL and Rust, changed by pfx's authors: the Charlie sheen distribution, Neubelt's visibility, the anisotropic GGX distribution and its height-correlated visibility (`shaders/src/surface_brdf.fs`), the sheen energy scaling, the anisotropic bent reflection normal and the cloth wrap diffuse with its subsurface tint (`shaders/src/surface_shading_model_standard.fs`, `surface_shading_model_cloth.fs`, `surface_shading_lit.fs`, `surface_lighting.fs`, `surface_light_indirect.fs`). The sheen albedo table is pfx's own integral of that lobe.

## kira, under the MIT licence

`hermite` in `crates/sound/src/interp.rs` (in the package `pfx-sound`), the 4-point, 3rd-order Hermite behind `Resample::Cubic`, is ported from kira's `interpolate_frame` (https://github.com/tesselode/kira, `crates/kira/src/frame.rs`, commit `74b3ef0fd8bd15f90e3b469208ee741f04374a62`), and `Drive` in `crates/sound/src/drive.rs` (the soft and hard clip curves, the drive gain with its division back out, and the equal-power dry and wet blend) from kira's `Distortion` effect (`crates/kira/src/effect/distortion.rs`, the same commit), and `gains` in `crates/film/src/duck.rs` (in the package `pfx-film`; the dB level, the amount over the threshold and its envelope follower with its attack and release speeds) from kira's `Compressor` effect (`crates/kira/src/effect/compressor.rs`, the same commit), Copyright 2021 Andrew Minnich, licensed MIT OR Apache-2.0 and used here under the MIT licence:

> Copyright 2021 Andrew Minnich
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.



## Signalsmith Stretch and Signalsmith Linear, under the MIT licence

`crates/sound/src/voicefx/stretch.rs` (in the package `pfx-sound`, feature `voicefx`), the pitch, formant and time stretcher, is ported from Signalsmith Stretch (https://github.com/Signalsmith-Audio/signalsmith-stretch, `signalsmith-stretch.h`, tag `1.4.0`, commit `a670068d9aeb64913331d5cc29337b19a457a7df`): the phase-vocoder prediction from the previous block and the neighbouring bins, the peak-based frequency map, the energy smoothing, the formant envelope and pitch estimate, the seek, pre-roll and flush of a whole buffer, and the presets' block and interval lengths. Copyright (c) 2022 Geraint Luff / Signalsmith Audio Ltd., licensed MIT:

> Copyright (c) 2022 Geraint Luff / Signalsmith Audio Ltd.
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

`crates/sound/src/voicefx/stft.rs` and `crates/sound/src/voicefx/fft.rs` are ported from Signalsmith Linear (https://github.com/Signalsmith-Audio/linear, `stft.h` and `fft.h`, tag `0.6.4`, commit `de55e6a50ffcf6f8f43f649692d94691c7025151`, the version Stretch `1.4.0` pins): the self-normalising STFT with its Kaiser window and perfect-reconstruction scaling, the half-bin-shifted real FFT, the split radix-3 and radix-5 FFT and the power-of-two radix-4 FFT. Copyright (c) 2025 Signalsmith Audio, licensed MIT:

> Copyright (c) 2025 Signalsmith Audio
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## mda VST plug-ins, under the MIT licence

In the package `pfx-sound` (feature `voicefx`), the channel vocoder in `crates/sound/src/voicefx/vocoder.rs` (its 8 and 16 band tables, the 4th-order resonator bank run at half rate with its zeros applied once, the pre-emphasis, the high band and high thru, the envelope followers and their speeds, the gain and Q formulas and the denormal and instability catches) is ported from mda Vocoder (`src/mdaVocoder.cpp`), the bit and rate reducer in `crates/sound/src/voicefx/crush.rs` (the hold counter, the averaging mode, the truncating quantiser and the 8-pole post filter with its `filterFreq` constants) from mda Degrade (`src/mdaDegrade.cpp`), and the ring modulator in `crates/sound/src/voicefx/ring.rs` (the sine carrier and its 0.95 feedback) from mda RingMod (`src/mdaRingMod.cpp`), all from https://sourceforge.net/p/mda-vst/code/ at SVN revision 12, Copyright (c) 2008 Paul Kellett, released under the MIT licence or the GPL version 2 or later and used here under the MIT licence:

> mda VST plug-ins
>
> Copyright (c) 2008 Paul Kellett
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## fundsp, under the MIT licence

In the package `pfx-sound` (feature `voicefx`), the chorus in `crates/sound/src/voicefx/chorus.rs` (four taps at multiples of the separation swung by the variation, their rates spread by 0.02 Hz, the LFO sampled every 10 ms, the 8-octave spline noise with roughness 0.45 and its 1.25 divisor, and the 0.2 mix that `mix: 0.8` reproduces), the flanger in `crates/sound/src/voicefx/flanger.rs` (a tap swung between the minimum and maximum delay by a sine, its output fed back through a tanh) and the delay line in `crates/sound/src/voicefx/line.rs` (the cubic tap, its 1.00001-frame floor and its power-of-two buffer) are ported from fundsp (https://github.com/SamiPerttu/fundsp, `src/prelude.rs` `chorus` and `flanger`, `src/delay.rs` `Tap`, `src/math.rs` `fractal_noise` and `spline_noise`, version 0.23.0 at commit `559584043aa3369e92d5aad12a5e3b003cfacfd5`), by Sami Perttu, licensed MIT OR Apache-2.0 and used here under the MIT licence:

> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## bevy_hanabi, under the MIT licence

In the package `pfx-core`, `crates/core/src/sim/particles.rs` ports bevy_hanabi's GPU-free parts (https://github.com/djeedai/bevy_hanabi, version 0.19.0 at commit `cf35d53d54fb973e30c635e7097ff3e6abf33099`): the spawner's fractional remainder (`src/spawn.rs`, `EffectSpawner::tick`), the gradient's sampling with its first-of-duplicates rule (`src/gradient.rs`, `Gradient::sample`) and the circle basis of `SetPositionCircleModifier` (`src/modifier/position.rs`), by Jerome Humbert, licensed MIT OR Apache-2.0 and used here under the MIT licence:

> Copyright (c) 2021 Jerome Humbert
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## librosa, under the ISC licence

In the package `pfx-sound` (feature `voicefx`), the spectral-flux onsets in `crates/sound/src/voicefx/syllables.rs` (the positive rise in dB averaged over bands with lag 1, `power_to_db`'s 1e-10 floor and 80 dB range, the min and max normalisation, `onset_detect`'s peak-picking windows of 30 ms, 100 ms and a 0.07 threshold, and the greedy `peak_pick`) are ported from librosa (https://github.com/librosa/librosa, `librosa/onset.py` and `librosa/util/utils.py`, commit `08e2c0a20d650fe00f10aaf3a1eaa9698ac19641`), and in the same package (feature `clean`) the background median of `crates/sound/src/clean/transient.rs` (the harmonic median of `decompose.hpss`, a median over time per frequency with `reflect` edges) is ported from librosa's `librosa/decompose.py` (tag `0.11.0`, commit `af8c839fb15317fa2712ea66e7a22da6a9267b32`), licensed ISC:

> Copyright (c) 2013--2026, librosa development team.
>
> Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.
>
> THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

## noisereduce, under the MIT licence

In the package `pfx-sound` (feature `clean`), the stationary spectral gate of `crates/sound/src/clean/gate.rs` (the noise print's mean and standard deviation in dB per frequency, the threshold at 1.5 standard deviations, `_amp_to_db` with its 80 dB range, the mask scaled by `prop_decrease`, the triangular smoothing kernel over 500 Hz and 50 ms, the 1024-sample FFT with a quarter hop) is ported from noisereduce (https://github.com/timsainb/noisereduce, `noisereduce/spectralgate/stationary.py`, `base.py` and `utils.py`, tag `v3.0.3`, commit `f33f3db55c28da06f278d2d4883370fc1af30fb6`), licensed MIT:

> The MIT License (MIT)
> Copyright (c) 2019, Tim Sainburg
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## enginesound, under the MIT licence

The engine voice in `crates/sound/src/shot/engine.rs` (in the package `pfx-sound`, feature `shot`), its crank cycle, piston and ignition pulses, valve-modulated cylinder waveguides, collectors, straight pipe, muffler elements, intake noise, waveguide damping and DC filter, with the tuned values of `example1.esc`, is ported from enginesound (https://github.com/DasEtwas/enginesound, `src/gen.rs`, commit `e5fcca587397c0c8ba9c9d24874b951fed74d260`), Copyright (c) 2020 DasEtwas, licensed MIT:

> Copyright (c) 2020 DasEtwas
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Mutable Instruments Clouds, under the MIT licence

The recorded engine's grain player in `crates/sound/src/shot/recording.rs` (in the package `pfx-sound`, feature `shot`), its two overlapping windowed grains each reading the source at its own phase increment (the pitch ratio) and the grain-size limit of a quarter of the source divided by the pitch ratio, is ported from Mutable Instruments Clouds (https://github.com/pichenettes/eurorack, `clouds/dsp/grain.h` and `clouds/dsp/granular_sample_player.h`, commit `f4cfa7697095a28d292bff758beb51deb49b5a86`), Copyright 2014 Emilie Gillet, licensed MIT:

> Copyright 2014 Emilie Gillet.
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Hash without Sine, under the MIT licence

`hash12` in `crates/post/src/look/mod.rs` (in the package `pfx-post`, re-exported by `crates/live/src/flat/sdf.rs`), `flat_hash12` in `crates/live/src/flat/shader.rs` (in the package `pfx-live`) and `look_hash12` in `crates/post/src/look/shader.rs`, the hash behind the flat pass's `Grain` and `Fibre` patterns and the flat looks' `Grain` pass, are ported from David Hoskins' "Hash without Sine" (https://www.shadertoy.com/view/4djSRW, `hash12`, its constants `.1031` and `33.33` kept), Copyright (c)2014 David Hoskins, licensed MIT:

> Copyright (c)2014 David Hoskins.
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## KinoMotion, under the MIT licence

`crates/post/src/motion_blur.rs` (in the package `pfx-post`), the live renderer's motion blur, is ported from Keijiro Takahashi's KinoMotion (https://github.com/keijiro/KinoMotion, commit `35d5d5179e60703d1db5eac967715f5ea416e387`, `Assets/Kino/Motion/Shader/Prefilter.cginc`, `Reconstruction.cginc` and `Script/ReconstructionFilter.cs`), itself after McGuire, Hennessy, Bukowski and Osman, "A Reconstruction Filter for Plausible Motion Blur" (I3D 2012), Copyright (c) 2016, 2017 Keijiro Takahashi, licensed MIT:

> Copyright (c) 2016, 2017 Keijiro Takahashi
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## ISF-Files, under the MIT licence

`ghost_weight` in `crates/post/src/flare.rs` and the ghost weight in `FLARE_FEATURES` (`crates/post/src/wgsl.rs`), in the package `pfx-post`, the lens flare's radial ghost falloff `pow(1 - |0.5 - uv| / |0.5|, 10)`, are ported from VIDVOX's ISF-Files (https://github.com/Vidvox/ISF-Files, `ISF/Lens Flare.fs`, commit `395072d48b3ce7351ccb20a5fda54470591324df`), itself after John Chapman's "Pseudo Lens Flare" (2013), licensed MIT:

> Copyright (c) 2018
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## FidelityFX SDK, under the MIT licence

The live renderer's glossy screen-space reflections in `crates/live/src/reflect.rs` (in the package `pfx-live`) port parts of AMD's FidelityFX SDK (https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK, tag `v1.1.4`, commit `c6efa6bf7f2027b3ec94f28578bb5965eabb9e55`): `ssr_vndf` and `ssr_sample_reflection` from the stochastic SSR's `SampleGGXVNDF` and `SampleReflectionVector` (`sdk/include/FidelityFX/gpu/sssr/ffx_sssr_intersect.h`, after Heitz, "Sampling the GGX Distribution of Visible Normals", JCGT 2018), and `PREFILTER_WGSL` from the reflection denoiser's prefilter (`sdk/include/FidelityFX/gpu/denoiser/ffx_denoiser_reflections_prefilter.h` and `ffx_denoiser_reflections_config.h`: its 15 sample offsets, the normal weight to the power 512, the radiance weight with its bias 0.6, variance factor 0.1 and floor 0.01, and the variance weight 4.4 with its bias 0.1). Copyright (C) 2024 Advanced Micro Devices, Inc., licensed MIT:

> Copyright (C) 2024 Advanced Micro Devices, Inc.
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files(the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and /or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions :
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

pfx's mark and wordmark are © Catalin Ilinca, all rights reserved, and not covered by the MIT licence ([TRADEMARKS.md](TRADEMARKS.md)):

- `assets/marks/` and `.github/icon.png`, copied by `bin/marks` from the PITO MD brand masters at `aa3aee8b232a1b848995bf3d9a012a01d4aff8ec`; the README shows the lockup at its Licence section, `lockup.svg` on a dark theme and `lockup-light.svg` on a light one;
- `crates/project/src/templates/badge.glb` and `crates/project/src/templates/wordmark.glb`, the mark and wordmark in 3D that `pfx new` places in a new game's first scene.

## Bevy, under the MIT licence

The outdoor height fog's ray march in `crates/live/src/height_fog.rs` (in the package `pfx-live`) is ported from Bevy's volumetric fog (https://github.com/bevyengine/bevy, `crates/bevy_pbr/src/volumetric_fog/volumetric_fog.wesl` and `crates/bevy_pbr/src/render/utils.wesl`, release 0.20.0, commit `b3bdd75becaffe9a16677cf5bae5bd6d198f052b`), Copyright the Bevy Contributors, licensed MIT OR Apache-2.0 and used here under the MIT licence:

`crates/live/src/clusters.rs` and the clustered lookup in `LIGHTS_WGSL` (`cluster_span`, in `crates/live/src/lights.rs`), in the package `pfx-live`, are ported from Bevy's clustered forward shading (https://github.com/bevyengine/bevy, tag `v0.16.1`, commit `383b3510455c431f34cf3f2c6e3c2d40eddce744`): the `ClusterConfig::FixedZ` grid of 4,096 clusters in 24 depth slices with a 5 m first slice and its x and y split by the aspect ratio (`crates/bevy_pbr/src/cluster/mod.rs`), the exponential depth slicing and its factors (`calculate_cluster_factors`, `crates/bevy_pbr/src/render/light.rs`; `view_z_to_z_slice` and `fragment_cluster_index`, `crates/bevy_pbr/src/render/clustered_forward.wgsl`), the far plane at the farthest light's reach, and the spot cone test against a cluster's bounding sphere (`crates/bevy_pbr/src/cluster/assign.rs`, after Bart Wronski's "Cull that cone"). Copyright (c) 2020 Carter Anderson, licensed MIT OR Apache-2.0 and used here under the MIT licence:

The round bloom in `crates/post/src/bloom.rs` (`round_glow`, `blend_factor`, `round_levels`) and the `BLOOM_ROUND_*` shaders and `ROUND_GLOW` in `crates/post/src/wgsl.rs`, in the package `pfx-post`, are ported from Bevy's bloom (https://github.com/bevyengine/bevy, tag `v0.16.1`, commit `383b3510455c431f34cf3f2c6e3c2d40eddce744`, `crates/bevy_core_pipeline/src/bloom/`): the 13-tap downsample and 3x3 tent upsample of Jorge Jimenez's "Next Generation Post Processing in Call of Duty: Advanced Warfare" with Brian Karis's firefly weights on the first downsample (`bloom.wgsl`), the mip chain of 8 levels with the first 512 texels tall (`prepare_bloom_textures`), the energy-conserving blend of each level into the one above and `compute_blend_factor` with the `Bloom::NATURAL` constants (low-frequency boost 0.7, curvature 0.95, high-pass frequency 1.0) (`mod.rs`, `settings.rs`). Copyright (c) 2020 Carter Anderson, licensed MIT OR Apache-2.0 and used here under the MIT licence:

> Copyright (c) 2020 Carter Anderson
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Filament, under the Apache License 2.0

The outdoor height fog's density and its closed-form optical path in `crates/live/src/height_fog.rs` (in the package `pfx-live`) are ported from Filament's fog (https://github.com/google/filament, `shaders/src/surface_fog.fs` and `filament/src/ds/ColorPassDescriptorSet.cpp`, tag `v1.56.0`, commit `003e5005713cba55cbd3f3a423c472000b96b868`), Copyright The Android Open Source Project, licensed under the Apache License 2.0. Its text ships in [crates/live/LICENSE-APACHE](crates/live/LICENSE-APACHE); the package is MIT like the rest of pfx, and this notice and that licence go with the parts ported from Filament. The port is a translation into WGSL, changed by pfx's authors: the density is sampled inside a ray march instead of applied once per surface, and the optical path is taken along each light's chord.

## lz4_flex, under the MIT licence

`crates/file/src/lz4.rs` (in the package `pfx-file`) is a port of the LZ4 block encoder and decoder of lz4_flex 0.14.0 (https://github.com/pseitz/lz4_flex, commit `1bffdcbbf906a234b913937cb2f57c0245915038`), altered, under the MIT licence, as its `LICENSE` file at that commit reads:

> The MIT License (MIT)
>
> Copyright (c) 2020 Pascal Seitz
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of
> this software and associated documentation files (the "Software"), to deal in
> the Software without restriction, including without limitation the rights to
> use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
> the Software, and to permit persons to whom the Software is furnished to do so,
> subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
> FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
> COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
> IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
> CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## crc32fast, under the MIT licence

`crates/file/src/crc32.rs` (in the package `pfx-file`) is a port of the slice-by-16 CRC-32 of crc32fast 1.5.0 (https://github.com/srijs/rust-crc32fast, tag `v1.5.0`, commit `dbf4f76cd71cdcc57d9164cbd46890d53ce0423c`), altered, licensed MIT OR Apache-2.0 and used here under the MIT licence, as its `LICENSE-MIT` file at that tag reads:

> MIT License
>
> Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

## Godot Engine, under the MIT licence

The route spline's arc-length table in `crates/motion/src/route.rs` (in the package `pfx-motion`) ports the arc-length baking of Godot Engine's Curve3D (https://github.com/godotengine/godot, `scene/resources/curve.cpp`, tag `4.5-stable`, commit `876b290332ec6f2e6d173d08162a02aa7e6ca46d`): the even-length subdivision of each segment, the distance table searched by bisection, and the nearest distance by projection onto the baked polyline, altered, under the MIT licence, as its `LICENSE.txt` at that commit reads:

> Copyright (c) 2014-present Godot Engine contributors (see AUTHORS.md).
> Copyright (c) 2007-2014 Juan Linietsky, Ariel Manzur.
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

## meshoptimizer, under the MIT licence

`crates/mesh/src/simplify.rs` (in the package `pfx-mesh`) is a port of the edge-collapse simplifier of meshoptimizer 0.25 (https://github.com/zeux/meshoptimizer, `src/simplifier.cpp`, tag `v0.25`, commit `6daea4695c48338363b08022d2fb15deaef6ac09`, the version the meshopt 0.6.2 crate vendors), altered, under the MIT licence, as its `LICENSE.md` at that tag reads:

> MIT License
>
> Copyright (c) 2016-2025 Arseny Kapoulkine
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

## bevy_mikktspace and MikkTSpace, under the MIT and zlib licences

`crates/mesh/src/tangents.rs` (in the package `pfx-mesh`) is a port, for triangles only, of bevy_mikktspace 1.0.0 (https://github.com/bevyengine/bevy_mikktspace, commit `9de78a281bca0505142da521eb5152146da28656`), licensed MIT OR Apache-2.0 and used here under the MIT licence, as its `LICENSE-MIT` file at that commit reads:

> Copyright (c) 2017 The mikktspace Library Developers
>
> Permission is hereby granted, free of charge, to any
> person obtaining a copy of this software and associated
> documentation files (the "Software"), to deal in the
> Software without restriction, including without
> limitation the rights to use, copy, modify, merge,
> publish, distribute, sublicense, and/or sell copies of
> the Software, and to permit persons to whom the Software
> is furnished to do so, subject to the following
> conditions:
>
> The above copyright notice and this permission notice
> shall be included in all copies or substantial portions
> of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
> ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
> TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
> PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
> SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
> CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
> OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
> IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
> DEALINGS IN THE SOFTWARE.

bevy_mikktspace is itself a reimplementation of MikkTSpace (https://github.com/mmikk/MikkTSpace, commit `3e895b49d05ea07e4c2133156cfa94369e19e409`), altered here, under the zlib licence, as the header comment of its `mikktspace.h` at that commit reads:

> /**
> Copyright (C) 2011 by Morten S. Mikkelsen
>  *
> This software is provided 'as-is', without any express or implied
> warranty.  In no event will the authors be held liable for any damages
> arising from the use of this software.
>  *
> Permission is granted to anyone to use this software for any purpose,
> including commercial applications, and to alter it and redistribute it
> freely, subject to the following restrictions:
>  *
> 1. The origin of this software must not be misrepresented; you must not
>    claim that you wrote the original software. If you use this software
>    in a product, an acknowledgment in the product documentation would be
>    appreciated but is not required.
> 2. Altered source versions must be plainly marked as such, and must not be
>    misrepresented as being the original software.
> 3. This notice may not be removed or altered from any source distribution.

## curvo, under the MIT licence

The loft in `crates/build/src/builtin/loft.rs` (in the package `pfx-build`) ports the loft of curvo (https://github.com/mattatz/curvo, version 0.3.2, commit `6f6a51847a50092507936e9fd5735feedfecb69c`): `NurbsSurface::try_loft`, the curve interpolation by knot averaging and the B-spline basis, altered, under the MIT licence, as its `LICENSE` file at that commit reads:

> MIT License
>
> Copyright (c) 2024 mattatz
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

## OpenSubdiv, under the Tomorrow Open Source Technology License 1.0

The same file ports OpenSubdiv's Catmull-Clark subdivision with semi-sharp creases (https://github.com/PixarAnimationStudios/OpenSubdiv, tag `v3_7_0`, commit `9dab8a47bfbb1388ec8388fe61f5f916e6123f38`): the masks of `opensubdiv/sdc/catmarkScheme.h` and `scheme.h` and the sharpness rules of `opensubdiv/sdc/crease.cpp`, changed from C++ into Rust by pfx's authors. `crease.cpp` is Copyright 2014 DreamWorks Animation LLC. OpenSubdiv is under the Tomorrow Open Source Technology License 1.0, which is the Apache License 2.0 with Section 6 replaced. Its `LICENSE.txt` at that commit ships unchanged in [crates/build/LICENSE-APACHE](crates/build/LICENSE-APACHE); its Section 6 reads:

> 6. Trademarks. This License does not grant permission to use the trade
>    names, trademarks, service marks, or product names of the Licensor
>    and its affiliates, except as required to comply with Section 4(c) of
>    the License and to reproduce the content of the NOTICE file.

Its `NOTICE.txt` at that commit reads:

> OpenSubdiv
> Copyright 2013 Pixar
> All rights reserved.
>
> This product includes software developed at:
>     Pixar (http://www.pixar.com/).
>     Dreamworks Animation (http://www.dreamworksanimation.com/)
>     Autodesk, Inc. (http://www.autodesk.com/).
>     Google, Inc. (http://www.google.com/).
>     DigitalFish (http://digitalfish.com/).

## The Mersenne Twister, CPython's random and NumPy's RandomState, under BSD, MIT and PSF licences

`crates/generators/src/pyrandom.rs` and `crates/generators/src/numpy_random.rs` (in the package `pfx-generators`) reproduce, byte for byte, the streams of CPython's `random` module (https://github.com/python/cpython, `Modules/_randommodule.c` and `Lib/random.py`, tag `v3.14.0`, commit `ebf955df7a89ed0c7968f79faec1de49f61ed7cb`) and NumPy's legacy `RandomState` (https://github.com/numpy/numpy, `numpy/random/mtrand.pyx` and its MT19937 core, tag `v2.3.0`, commit `0532af47d6a815298b7841de00bdbc547104b237`), so a generator first written in Python gives the same shapes in pfx. Both rest on MT19937 by Makoto Matsumoto and Takuji Nishimura (`mt19937ar.c`, initialization improved 2002/1/26, from http://www.math.sci.hiroshima-u.ac.jp/~m-mat/MT/MT2002/emt19937ar.html), whose licence block reads:

> A C-program for MT19937, with initialization improved 2002/1/26.
> Coded by Takuji Nishimura and Makoto Matsumoto.
>
> Before using, initialize the state by using init_genrand(seed)
> or init_by_array(init_key, key_length).
>
> Copyright (C) 1997 - 2002, Makoto Matsumoto and Takuji Nishimura,
> All rights reserved.
>
> Redistribution and use in source and binary forms, with or without
> modification, are permitted provided that the following conditions
> are met:
>
>   1. Redistributions of source code must retain the above copyright
>      notice, this list of conditions and the following disclaimer.
>
>   2. Redistributions in binary form must reproduce the above copyright
>      notice, this list of conditions and the following disclaimer in the
>      documentation and/or other materials provided with the distribution.
>
>   3. The names of its contributors may not be used to endorse or promote
>      products derived from this software without specific prior written
>      permission.
>
> THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
> "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
> LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
> A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT OWNER OR
> CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
> EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
> PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
> PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
> LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
> NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
> SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

NumPy's MT19937 core carries that licence and, beside it, the MIT licence of the randomkit functions it keeps. Its `numpy/random/src/mt19937/LICENSE.md` at that tag reads:

> # MT19937
>
> Copyright (c) 2003-2005, Jean-Sebastien Roy (js@jeannot.org)
>
> The rk_random and rk_seed functions algorithms and the original design of
> the Mersenne Twister RNG:
>
>   Copyright (C) 1997 - 2002, Makoto Matsumoto and Takuji Nishimura,
>   All rights reserved.
>
>   Redistribution and use in source and binary forms, with or without
>   modification, are permitted provided that the following conditions
>   are met:
>
>   1. Redistributions of source code must retain the above copyright
>   notice, this list of conditions and the following disclaimer.
>
>   2. Redistributions in binary form must reproduce the above copyright
>   notice, this list of conditions and the following disclaimer in the
>   documentation and/or other materials provided with the distribution.
>
>   3. The names of its contributors may not be used to endorse or promote
>   products derived from this software without specific prior written
>   permission.
>
> THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
> "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
> LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
> A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT OWNER
> OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
> EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
> PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
> PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
> LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
> NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
> SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
>
> Original algorithm for the implementation of rk_interval function from
> Richard J. Wagner's implementation of the Mersenne Twister RNG, optimised by
> Magnus Jonsson.
>
> Constants used in the rk_double implementation by Isaku Wada.
>
> Permission is hereby granted, free of charge, to any person obtaining a
> copy of this software and associated documentation files (the
> "Software"), to deal in the Software without restriction, including
> without limitation the rights to use, copy, modify, merge, publish,
> distribute, sublicense, and/or sell copies of the Software, and to
> permit persons to whom the Software is furnished to do so, subject to
> the following conditions:
>
> The above copyright notice and this permission notice shall be included
> in all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
> OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
> MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
> IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
> CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
> TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
> SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

NumPy is under the BSD 3-Clause licence, as its `LICENSE.txt` at that tag reads:

> Copyright (c) 2005-2025, NumPy Developers.
> All rights reserved.
>
> Redistribution and use in source and binary forms, with or without
> modification, are permitted provided that the following conditions are
> met:
>
>     * Redistributions of source code must retain the above copyright
>        notice, this list of conditions and the following disclaimer.
>
>     * Redistributions in binary form must reproduce the above
>        copyright notice, this list of conditions and the following
>        disclaimer in the documentation and/or other materials provided
>        with the distribution.
>
>     * Neither the name of the NumPy Developers nor the names of any
>        contributors may be used to endorse or promote products derived
>        from this software without specific prior written permission.
>
> THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
> "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
> LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
> A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
> OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
> SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
> LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
> DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
> THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
> (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
> OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

CPython is under the Python Software Foundation License Version 2, as section B of its `LICENSE` at the tag above reads:

> B. TERMS AND CONDITIONS FOR ACCESSING OR OTHERWISE USING PYTHON
> ===============================================================
>
> Python software and documentation are licensed under the
> Python Software Foundation License Version 2.
>
> Starting with Python 3.8.6, examples, recipes, and other code in
> the documentation are dual licensed under the PSF License Version 2
> and the Zero-Clause BSD license.
>
> Some software incorporated into Python is under different licenses.
> The licenses are listed with code falling under that license.
>
>
> PYTHON SOFTWARE FOUNDATION LICENSE VERSION 2
> --------------------------------------------
>
> 1. This LICENSE AGREEMENT is between the Python Software Foundation
> ("PSF"), and the Individual or Organization ("Licensee") accessing and
> otherwise using this software ("Python") in source or binary form and
> its associated documentation.
>
> 2. Subject to the terms and conditions of this License Agreement, PSF hereby
> grants Licensee a nonexclusive, royalty-free, world-wide license to reproduce,
> analyze, test, perform and/or display publicly, prepare derivative works,
> distribute, and otherwise use Python alone or in any derivative version,
> provided, however, that PSF's License Agreement and PSF's notice of copyright,
> i.e., "Copyright (c) 2001 Python Software Foundation; All Rights Reserved"
> are retained in Python alone or in any derivative version prepared by Licensee.
>
> 3. In the event Licensee prepares a derivative work that is based on
> or incorporates Python or any part thereof, and wants to make
> the derivative work available to others as provided herein, then
> Licensee hereby agrees to include in any such work a brief summary of
> the changes made to Python.
>
> 4. PSF is making Python available to Licensee on an "AS IS"
> basis.  PSF MAKES NO REPRESENTATIONS OR WARRANTIES, EXPRESS OR
> IMPLIED.  BY WAY OF EXAMPLE, BUT NOT LIMITATION, PSF MAKES NO AND
> DISCLAIMS ANY REPRESENTATION OR WARRANTY OF MERCHANTABILITY OR FITNESS
> FOR ANY PARTICULAR PURPOSE OR THAT THE USE OF PYTHON WILL NOT
> INFRINGE ANY THIRD PARTY RIGHTS.
>
> 5. PSF SHALL NOT BE LIABLE TO LICENSEE OR ANY OTHER USERS OF PYTHON
> FOR ANY INCIDENTAL, SPECIAL, OR CONSEQUENTIAL DAMAGES OR LOSS AS
> A RESULT OF MODIFYING, DISTRIBUTING, OR OTHERWISE USING PYTHON,
> OR ANY DERIVATIVE THEREOF, EVEN IF ADVISED OF THE POSSIBILITY THEREOF.
>
> 6. This License Agreement will automatically terminate upon a material
> breach of its terms and conditions.
>
> 7. Nothing in this License Agreement shall be deemed to create any
> relationship of agency, partnership, or joint venture between PSF and
> Licensee.  This License Agreement does not grant permission to use PSF
> trademarks or trade name in a trademark sense to endorse or promote
> products or services of Licensee, or any third party.
>
> 8. By copying, installing or otherwise using Python, Licensee
> agrees to be bound by the terms and conditions of this License
> Agreement.

## Fonts, under the SIL Open Font License 1.1

Each font keeps its licence beside it:

- `crates/text/fonts/`: EB Garamond.
- `crates/text/tests/fonts/`: Bungee Color, DM Sans, IBM Plex Mono and a Noto Sans SC subset.
- `crates/editor-style/fonts/`: the editor theme's fonts, each subset with `pfx font subset` and kept with its `SOURCE.md` (upstream, commit or release, sha256 before and after) and its `chars.txt`:
  - `jost/`: Jost, Copyright 2020 The Jost Project Authors (https://github.com/indestructible-type/Jost), the variable `Jost[wght].ttf` from Google Fonts at `6e4b84c9`, with its `OFL.txt`.
  - `jetbrains-mono/`: JetBrains Mono Regular and Bold, Copyright 2020 The JetBrains Mono Project Authors (https://github.com/JetBrains/JetBrainsMono), release v2.304, with its `OFL.txt`.
  - `icons/`: eighteen Font Awesome Free icon glyphs, Copyright (c) 2022 Fonticons, Inc. (https://fontawesome.com), as Nerd Fonts 3.5.1 patches them into JetBrains Mono, with JetBrains Mono's `OFL.txt` and Font Awesome's `FontAwesome-LICENSE.txt` (SIL Open Font License 1.1, Reserved Font Name "Font Awesome", which the subset does not use).
- `crates/editor-style/fixture/`: Inconsolata, the editor crates' test font, with its `OFL.txt` and `SOURCE`.
- `assets/fonts/`: JetBrains Mono (by JetBrains) patched with the Nerd Fonts icon glyphs (Nerd Fonts 3.5.1, OFL-1.1-no-RFN), the recipes' text family, which `pfx-build` and `pfx-recipe-scene` embed, with its `OFL.txt` and `SOURCE`.
- `crates/project/src/templates/IBMPlexMono-Regular.ttf`: IBM Plex Mono, with `IBMPlexMono-OFL.txt`; `pfx new` ships it unmodified into a new game.
