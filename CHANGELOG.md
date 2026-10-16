# Changelog

## v2.5.0 — 2026-10-16

### Added

- A recorded engine in the shot mix: `[[engine]] sample = "<file.wav>"` extends a real recording to any length and rpm (seeded grains of the idle cycle, recorded revs crossfaded by rpm, start and stop one-shots, load shaping), and blends with the synthesised engine above the recording's range.
- Road surfaces: `[[road]] surface = "asphalt" | "wet" | "gravel" | "cobbles"`, or a list over time that crossfades between them; asphalt stays the default and sounds as before.
- `[[sample]]` cues take trims, fades and seamless loops.
- Round bloom: a wide, photographic glow, now the default when a finish's bloom names no kind (`kind = "box"` keeps the old one).

### Changed

- ffmpeg is gone: clip masters, audio muxing and audio decoding run through pito-codecs v1.12.1. Masters are H.264 High at CRF 12 with AAC 320k, and are larger than the old x264 ones; `pfx encode` refuses FFV1 masters. Clip scenes in films read AV1, and WebM with alpha keeps its transparency.
- Particles light like haze when the light is behind them, their streaks follow their motion on screen, and they no longer smear the image under them.
- `pfx shot --camera` takes the `object/node` names its manifest writes, the manifest says which scene frame file 000000 is, and an output folder left by an interrupted run is reused.

### Fixed

- Motion blur could turn still objects in front of moving ones into black blocks.
- `pfx shot` refuses a camera that can't form a view instead of drawing garbage.

## v2.4.0 — 2026-10-16

### Added

- The path tracer keeps one structure per mesh and a top level over instances: `Trace::set_instance_transforms` moves objects in a fraction of a second instead of rebuilding the whole scene, and a camera move rebuilds nothing.
- A staged scene plays its objects' animation clips itself, with attached lights and parented particles following, and a camera's focus keys stay on the loaded camera (`DepthOfField::distance_at`).
- Overlay hooks for tools that draw on top of a frame: `Renderer::last_jitter`, `Renderer::depth_view`, and opt-in deformed positions of skinned and morphed instances.
- `pfx shot` and `pfx film` show one progress line on a terminal; JSON only with `--json`.
- Film cards and scenes: SVG marks keep their gradients, strokes and opacity, scenes take PNG image art, clip and frame scenes take a `crop`, the score can duck named sounds under named cues (bass under the kick), and a scene can set its score gain.

### Changed

- Rough surfaces blur what they reflect: screen-space reflections sample the surface's GGX lobe and are denoised, so a wet road no longer mirrors windows as sharp squares.
- Lens flare ghosts come only from sources near the centre of the frame, so bright windows no longer scatter coloured ghosts across the image.
- The low sun glows through outdoor haze as it should.
- The pfx tool hides Mesa's non-conformant-driver warning unless you set it yourself.
- `pfx_load::scene::Camera` and `DepthOfField` are no longer `Copy` (they carry focus keys).

## v2.3.0 — 2026-10-16

### Added

- Wet surfaces and puddles: a material's `[wet]` (amount, puddles, puddle_scale, seed) darkens it, turns on a water film and lays world-space puddles that reflect, the same in the live renderer and the path tracer.
- Figurine materials: cloth sheen (`fabric`), anisotropic highlights for hair (`anisotropy`) and a cheap subsurface stand-in for skin and vinyl (`scatter`), in both renderers.
- A GLB's own materials are drawn when the library has no entry of that name: base colour, normal, metallic-roughness with occlusion, sheen and anisotropy extensions, texture transforms and a detail normal on the second UV set, in both renderers; maps of different sizes are resampled by pfx.
- Prebuilt levels of detail from a GLB's `MSFT_lod` nodes (with each level's error), morphed heads included, and `LodSet::add_chain` for levels made in code.

### Changed

- Scenes use pfx-scene v1.2.0.
- `pfx shot --from N` renders the frames before N without writing them, so frame N matches the same frame of a full run.
- The lens flare's ghosts and halo fade in smoothly above the threshold, so dim sources such as tail lights no longer paint ghosts over the frame.

## v2.2.0 — 2026-10-16

### Added

- 3D particles: emitters in scene files (`[[particles]]`) attached to an object or a glTF node, with rate keys, lifetime, speed cone, inherited velocity, gravity, drag, size, alpha and colour curves, lit by the sun, the sky and local lights, blended or additive, stretched by velocity and blurred by motion blur.
- Many lights: more than four local lights through clustered lighting, spot lights with smooth cones, and lights that ride an object or a glTF node (head and tail lights follow the car).
- Lens flare and sun glare (`[flare]`: ghosts, halo, starburst) and colour grades from `.cube` LUT files (`[finish] lut`, `lut_amount`; up to 64 per side).
- `pfx film`: assemble a film from screen recordings (decoded through pito-codecs), rendered frame folders with their WAV, and cards, with camera moves, looks, captions, marks and a score, written as frames, a WAV and the one-line encode command.
- Alpha masters are folders of 16-bit RGBA PNG frames with a manifest, written by pfx itself (no ffmpeg); docs/alpha-master.md has the format.

### Changed

- A material with any transmission is drawn as glass, tinted by its base colour, with the share that isn't transmitted lit like the surface.
- `Light` gained `attach`, and the renderer's `Effects` gained `lights` and `sprays`, so code that builds them by hand needs the new fields.

### Fixed

- Thin glass lenses (a few millimetres) and single-sheet lenses no longer drop to white stripes or vanish.
- Any flat look used to hide a scene's sprites.

## v2.1.0 — 2026-10-16

### Added

- `pfx shot`: render a scene file offline on the live renderer at a fixed clock (frame n at n/fps), any size up to vertical 4K, with `--draft`/`--final`, `--hour`, `--set key=value` for any scene key, a camera from an animated glTF node, `--json`, PNG or EXR frames, a 48 kHz WAV of the scene's sound script, and a manifest with the one-line encode command.
- Motion blur from each object's motion (`[motion_blur]`), and outdoor height fog with sun shafts (`[haze]` `height`/`falloff`).
- Morph targets: loaded from glTF, played by the animator, set per frame with `World::set_morph_weights`, and blended on the GPU before skinning.
- A sprite atlas the live renderer draws into (`FlatSprites::live`) and alpha from coverage, so lit 3D heads sit inside flat UI cards.
- pfx-film: scripted films on the flat view with camera moves, looks, captions and a score, now with a beat grid (bpm, tempo map, times in beats and bars, a snap check) and ducking under the score.
- pfx-sound: offline voice effects behind the `voicefx` feature (pitch, formant and time stretch ported from Signalsmith Stretch, ring modulation, vocoder, bitcrusher, chorus, flanger, syllable cutting) and an offline shot mix from a sound script (engine, road, squeal, spray, passing cars, city bed).
- Icon atlases saved and loaded as bytes (`Icons::to_raw`, `Icons::from_raw`), and a flat group baked into a sprite (`SpriteImage::from_premultiplied`).

### Changed

- Scenes use pfx-scene v1.1.0 (spot and attached lights, particles, flare, motion blur, outdoor haze, wet materials, LUT files, focus keys, scene sound).
- The pacer's GPU-queue and uncapped-frame hooks need the new `pfx-gpu/dev-hooks` feature, so a shipped game carries no dev tools.
- A pierce-0 shot lands on its earliest target without sorting its hits.
- `Primitive`, `Group` and `Animation` gained morph fields, so code that builds them by hand needs the new fields.

### Fixed

- A window size larger than the GPU allows is clamped instead of crashing at launch.
- SVG paths with compact arc flags (`a.5.5 0 01.2.3`) read correctly.

## v2.0.0 — 2026-10-15

### Added

- Flat pass: grain and fibre patterns, pattern offsets, icons that cast their own shadow, and contact shadows under curves.
- Flat looks: grain, grade, letterbox, fade and flash, and bloom of a layer, with crossfades between looks.
- Flat view: one transform for the whole board (pan and zoom), on the GPU and the CPU twin.
- SVG: curves, viewBox-only roots and linear gradients.
- Games follow the system's light or dark scheme, and `World::focused()` says whether the window has focus.
- The window opens hidden and shows its first frame already cleared, with no flash of an empty window.
- Sound: a drive stage per channel and glides scheduled ahead of time.
- `Renderer::set_hour` changes the time of day without uploading the sky again.
- Sound: `Prepared` clips, converted once and shared by every play.

### Changed

- The audio thread no longer waits on file reads, resampling or freeing memory, and game audio reaches the device through a lock-free ring.
- Post effects keep their buffers and bind groups between frames.
- Text draws in shared passes instead of one pass per item.
- The live renderer uploads only the instances and materials that changed, and keeps its bind groups between frames.
- The flat pass draws its pick pass only when a pick is waiting: a 4K frame of 2,000 shots and 20,301 particles now costs 0.36 ms of GPU time, down from 0.42 ms.
- Physics keeps its query tree between steps and updates only the colliders that moved.
- Text, the game's UI painter and the play loop stop allocating per glyph and per tick.
- Fullscreen on the current monitor leaves the choice of output to the compositor.

### Fixed

- A PNG header can no longer ask for a huge allocation, a glTF node cycle can no longer overflow the stack, and a glTF buffer can no longer point outside its mesh folder.
- A Vorbis clip is capped at 8 channels and 10 minutes of 48 kHz stereo.
- A mod's log lines now cost fuel, so a mod can't spend unlimited time logging.
- A saved shot pool can no longer claim a huge capacity.
- Glows and additive light in the UI overlay.
- The window reconfigures after a suboptimal frame, and a resize to the same size no longer rebuilds the swapchain.

### Breaking

- `driver::Event` gained `Scheme`, `Fit` gained `view`, `Pattern` gained `offset` and `CurvePoint` gained `contact`: struct literals and exhaustive matches need the new field or arm.
- Raised icons cast a shadow by default; use `Shadow::None` to opt out.
- `FullscreenPlan::Borderless` takes `monitor: Option<usize>`.

## v1.0.0 — 2026-10-14

pfx reaches 1.0.0.

### Added

- journal: `JournalWriter::snapshot` hands out the journal so far in about 5 µs after 25 minutes of input, and `seal()` adds its checksum later, off the game's thread.
- live: two examples, `clip` and `swarm`, that write the live renderer's and the flat pass's frames as PNGs.
- docs: every release carries its documentation as `docs-vX.Y.Z.tar.gz` (guides, install, the CLI and format references, the API reference and the changelog), built by `bin/docs-bundle`, with three new first-task guides: make your first game, benchmark a scene, render a still.

### Changed

- shots: a saved arena keeps only its live shots, so a churned arena of 205 live shots saves as 23 KB in about 22 µs. The save format changed: saves from v0.32.0 don't load.
- render: stills and clips start the app's adapter with no display, and while `PFX_HEADLESS` is set no game, editor or GPU window can open, so a render never puts a window on the desktop.
- The README shows every feature with a picture or a clip and the engine's measured numbers: frame times, render times, denoise time, per-tick costs, and an empty game's size against Bevy's.

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
