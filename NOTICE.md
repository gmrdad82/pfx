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
- `crates/play/tests/characters/`: blocks for the character tests.
- `crates/sound/tests/fixtures/`: sine tones made with ffmpeg.
- `crates/project/src/templates/grid.glb`: the floor grid of the scaffold's first scene.
- `docs/images/`: the README's and the docs' pictures, rendered by pfx itself (the editor example's by `crates/editor-viewport/examples/editor.rs`); the mark they show is covered by [TRADEMARKS.md](TRADEMARKS.md).

## Logos, all rights reserved

pfx's mark and wordmark are © Catalin Ilinca, all rights reserved, and not covered by the MIT licence ([TRADEMARKS.md](TRADEMARKS.md)):

- `assets/marks/` and `.github/icon.png`;
- `crates/project/src/templates/badge.glb` and `crates/project/src/templates/wordmark.glb`, the mark and wordmark in 3D that `pfx new` places in a new game's first scene.

## Fonts, under the SIL Open Font License 1.1

Each font keeps its licence beside it:

- `crates/text/fonts/`: EB Garamond.
- `crates/text/tests/fonts/`: Bungee Color, DM Sans, IBM Plex Mono and a Noto Sans SC subset.
- `crates/theme/fonts/`: Exo 2 and JetBrains Mono, each with its `SOURCE.md`.
- `crates/editor-style/fixture/`: Inconsolata, the editor crates' test font, with its `OFL.txt` and `SOURCE`.
- `crates/project/src/templates/IBMPlexMono-Regular.ttf`: IBM Plex Mono, with `IBMPlexMono-OFL.txt`; `pfx new` ships it unmodified into a new game.
