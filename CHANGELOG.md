# Changelog

## v0.30.0 — 2026-10-14

### Added

- sim: `shots`, pooled projectiles in 2D and 3D with swept boxes, emitters with integer fire charge (single, spread, ring, aimed with lead, burst, spiral), wind-ups, pierce, formations that widen or narrow, and a kinematic follow with slow and stun.
- sim: `director`, a spawn director from data: phases, ramps, lulls, weighted tables by tag, caps, groups in a line, per-entry start times, saved as bytes.
- sim: `journal`, a compact per-tick input log (about 3 bytes a tick for a moving pointer and four buttons) with seed, versions and checkpoints, and a headless verify that replays it.
- sim: `Overlaps3`, the broadphase in 3D.
- numbers: game number formatting with grouping, scientific, engineering and short suffixes, per-style thresholds, ceil and truncate rounding, and a policy for NaN and infinity.
- words: a seeded text generator for names, titles and barks: templates with slots, weights, avoid lists, linked name forms, run-fixed draws, casts with no repeated parts and keyed redraws.

### Changed

- sim: `Overlaps` bins small boxes in a uniform grid, about three times faster on a wide field of small boxes.
- the asset look checks render at 128 samples, the count their floors were set at.

## v0.29.0 — 2026-10-14

### Added

- sound: `Output::music_heard()`, the position of the current music track the speaker plays now, latency included, so a meter or a beat-synced effect stays on the sound you hear; `Mixer::music_position()` gives the mixer's own.
- sim: `Overlaps`, a deterministic sort-and-sweep broadphase over integer boxes; 2,000 against 400 boxes in 0.16 ms.
- game: a world sets the cursor (`Cursor::Shown`, `Hidden` or `Captured`); the window hides or confines it and gives it back while unfocused or minimized.
- game: `FrameTime::tick`, the frame's whole tick, so effects step on the game's clock.

### Changed

- docs: the brand now reads PITO MD, and the README names people and AI agents alike.
- effects: particle drag costs one exponential per recipe per step instead of one per particle.

## v0.28.1 — 2026-10-12

### Fixed

- trace: Sobol shifts each pixel's patterns by a random offset per dimension (Cycles' Cranley-Patterson rotation), so scenes lit by small or narrow lights match PCG's mean (Manfred's workshop: -0.11% before, +0.01% now) with the same noise gain.
- editor: the stored shots follow egui 0.36's tighter rows.

## v0.28.0 — 2026-10-12

### Added

- trace: Cycles' tabulated Sobol sampler, the default (Sampler::Pcg or PFX_TRACE_SAMPLER=pcg keeps PCG); the same noise in about half the samples.

### Changed

- pfx sends no crash reports and writes no job records: pfx-report left the engine and its CLI; a game that wants reports wires its own.
- render: finals take 16 to 64 samples and drafts 6 to 12, PCG's noise at 32 to 128 and 8 to 16; the neutral desk's 4K final traces in 7.9 s of GPU time instead of 15.
- trace: on scenes lit by small or narrow lights Sobol's mean sits up to 0.15% under PCG's (Manfred's workshop: 0.11%, about 0.25/255).

## v0.27.0 — 2026-10-12

### Added

- trace: ray queries over a TLAS where the adapter offers them (Vulkan, DX12, Metal on Apple Silicon), the software BVH elsewhere; 2.6x faster 4K finals and 2.3x faster drafts on the neutral desk.
- docs: wgpu30.md, the changes an app that uses wgpu or egui directly makes to move with pfx.

### Changed

- move to wgpu 30.0.1, naga 30 and egui 0.36.2.
- archive: a pre-release tag such as v1.0.0-rc.1 pins a master like vX.Y.Z, in archive import and render's --rev.
- editor: clear egui's texture deltas once painted.
- editor: the shell's App::ui takes a Ui instead of a Context.
- editor: drop the pgpu play hook.
- trace: cut a dense strip into column pieces before dropping lanes.
- bake: the batch Frame is 176 bytes with the column field.

## v0.26.0 — 2026-10-12

### Added

- denoise: port Open Image Denoise's RT filter to wgpu compute.
- trace: windows of a larger frame and a per-round adaptive report.
- live: build a plate off the render thread and swap it in.
- gpu: enable VK_EXT_image_drm_format_modifier when the adapter offers it.
- archive: import a master an app made itself.
- ci: lint and build the game crates and the denoiser on macOS.

### Changed

- trace: start every trace at 64 lanes, ten times faster on ordinary scenes.
- trace: aim its bands at 10 ms.
- render: denoise finals and drafts; finals stop at 128 samples, drafts at 16.
- ci: run the gate as a parallel matrix, nightly too.
- check every text file out with LF line endings.

### Fixed

- gpu: open Vulkan with DRM format modifiers on Linux only, so pfx-gpu builds on macOS.

## v0.25.3 — 2026-10-11

### Changed

- run: encode with the system ffmpeg instead of a downloaded pin.
- ci: install the system ffmpeg for the encode tests.
- ci: run on Ubuntu 26.04 for a current ffmpeg.
- run: check the alpha master's range tag only on ffmpeg 9 or later.

### Fixed

- gpu: keep the pacer's last slice timing per thread.

## v0.25.2 — 2026-10-08

### Fixed

- steam stage: accept builds under the tree's target link.

## v0.25.1 — 2026-10-08

### Fixed

- steam stage: accept builds under CARGO_TARGET_DIR.

## v0.25.0 — 2026-10-08

### Added

- The first public release: the engine's crates, the `pfx` tool and the game launcher, from one fresh commit.
