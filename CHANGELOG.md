# Changelog

## v0.32.0 — 2026-10-14

### Added

- shots: with pfx-core's `serde` feature, shot pools, emitters, telegraphs and movers save and load, and a reloaded arena runs on the same bits.
- shots: each `Strike` carries the shot's position when it struck.
- director: `set_weight` changes a kind's weight mid-run and saves with the run; `Cycle` repeats a quiet window every N ticks; spot sets let a kind spawn only at its own spots; `rate_jitter` varies the gaps between spawns while keeping the average rate.
- numbers: `sci_rounding` gives the scientific form its own rounding.
- words: `{$name}` context slots take text the game passes at draw time with `generate_with`.
- journal: `JournalWriter::bytes` returns the journal so far without ending it; `Journal::verify_sparse` asks for the state digest only on checkpoint ticks and the last tick.
- sound: `trigger_with`, `trigger_in_with` and `trigger_at_with` start a sound at its own volume.

### Changed

- shots: `resolve` is about 4x faster when a shot overlaps one target (15 to 3.5 µs a tick at 1,080 hits), and the pool walks stop at the highest live slot (24 to 18 µs a tick at 2,000 shots).
- director: quiet windows cost nothing between their edges: 180 windows take about 40 ns a tick instead of about 200.

### Fixed

- words: a cast member's parts keep the case they were printed in instead of coming back lowercased.

## v0.31.0 — 2026-10-14

### Added

- sound: `Resample::Cubic` and `Linear` beside `Sinc`: short pitched clips read through a cubic Hermite in one tight loop per block; jittered hits drop from 5.0 to 0.3 ms of CPU a tick.
- sound: tones as sounds (`Mixer::tone_sound`), sharing the clips' voice cap, stealing and pitch jitter, with an optional length.
- sound: `trigger_in` and `tone_in` start a sound at a frame offset inside the block, so rapid triggers spread through it.

### Changed

- sound: `SoundSpec` defaults to cubic resampling; music, streams and `play` keep the sinc.
- sound: sines read a table, and the mixer's envelopes, filters, fades and jitter use the bit-exact sim math, so the mix is the same bits on every platform; 20 sine tones cost 0.1 ms a tick instead of 0.22.

## v0.30.0 — 2026-10-14

### Added

- sim: `shots`, pooled projectiles in 2D and 3D with swept boxes, emitters with integer fire charge (single, spread, ring, aimed with lead, burst, spiral), wind-ups, pierce, formations that widen or narrow, and a kinematic follow with slow and stun.
- sim: `director`, a spawn director from data: phases, ramps, lulls, weighted tables by tag, caps, groups in a line, per-entry start times, saved as bytes.
- sim: `journal`, a compact per-tick input log (about 3 bytes a tick for a moving pointer and four buttons) with seed, versions and checkpoints, and a headless verify that replays it.
- sim: `Overlaps3`, the broadphase in 3D.
- numbers: game number formatting with grouping, scientific, engineering and short suffixes, per-style thresholds, ceil and truncate rounding, and a policy for NaN and infinity.
- words: a seeded text generator for names, titles and barks: templates with slots, weights, avoid lists, linked name forms, run-fixed draws, casts with no repeated parts and keyed redraws.
- game: `World::warp_pointer` puts the pointer back at a layout point after a menu, read as no motion.

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
