# Changelog

## v0.25.3 — 2026-10-11

### Changed

- run: encode with the system ffmpeg instead of a downloaded pin.
- ci: install the system ffmpeg for the encode tests.
- ci: run on Ubuntu 26.04 for a current ffmpeg.
- run: check the alpha master's range tag only on ffmpeg 9 or later.

### Fixed

- gpu: keep the pacer's last slice timing per thread.
