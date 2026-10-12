# Changelog

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
