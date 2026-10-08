# Blender references of the asset renderer

One Blender render of each asset kind on the engine's neutral `sample` product (`crates/assets/tests/recipes`, marks in `crates/assets/tests/marks`), frozen on 2026-10-06 before Blender leaves the machine. They are the old look the asset port to the engine's tracer is compared against, and nothing renders them again.

- **Blender** 5.2.1 LTS (Arch `blender 17:5.2.1-2`), Cycles on the GPU (HIP, AMD Radeon RX 9060 XT), 256 samples, seed 7.
- **pfx** 0.20.1 (the installed build), run from the repo root through pgpu as pfx's own asset lane (`--class truth`), one render at a time.
- **Each file** is the 8-bit sRGB RGBA PNG delivery of the render, with the shadow in the alpha, copied unedited from `tmp/renders/sample/<kind>/`; the 16-bit masters were not kept. Stills are 512 px, the card is 1200 x 630.
- The `sample` marks are the tracked neutral SVGs `logos/sample.svg`, `wordmarks/sample.svg` and `lockups/sample.svg`; the wordmark is a blocky `SAMPLE` of plain rectangles, so no font is involved.
- Every file is listed in `tests/agnostic/fixtures.txt` with its sha256.

| file | kind | sha256 | command |
|---|---|---|---|
| `logo.png` | logo | `5916e5a56524d7881b1dff89f4c1928ba936f5e213747f77286af08bed226bf6` | `pfx render --asset logo --product sample --recipes crates/assets/tests/recipes --assets crates/assets/tests/marks --no-archive --samples 256 --size 512` |
| `wordmark.png` | wordmark | `6be1483f8255b8607d852f4a2b0f68ce41e9df0db398b87b8231b139de5bd63c` | `pfx render --asset wordmark --product sample --recipes crates/assets/tests/recipes --assets crates/assets/tests/marks --no-archive --samples 256 --size 512` |
| `lockup.png` | lockup | `fd2a5d5b358fc2084227a977e70f760fc4cc32ccd73cf6fc599cc9e1627c1abc` | `pfx render --asset lockup --product sample --recipes crates/assets/tests/recipes --assets crates/assets/tests/marks --no-archive --samples 256 --size 512` |
| `tile.png` | tile | `4dbb7ebe7c92c23efc1a1dc2f94b73797fea9a75bc6ef9409037c53b982a51bd` | `pfx render --asset tile --product sample --recipes crates/assets/tests/recipes --assets crates/assets/tests/marks --no-archive --samples 256 --size 512` |
| `card.png` | card (1200x630, its own size) | `19d8ddf892ae542bd98140d497ac6e08c4ef088f67c8b9a80ae74bb287da0e2e` | `pfx render --asset card --product sample --recipes crates/assets/tests/recipes --assets crates/assets/tests/marks --no-archive --samples 256` |
| `icon-shapes-dot.png` | icon set shapes, dot | `407e995f9ec82bc27f1c40574d1887d9a77acea2cd601e790b4fe6483a770a0f` | `pfx render --icons shapes --product sample --recipes crates/assets/tests/recipes --assets crates/assets/tests/marks --no-archive --samples 256 --size 512` |
| `icon-shapes-ring.png` | icon set shapes, ring | `750673087e6e09b436d9818958cfd68b004eb885b35d7a9546c6b3c0bd24b4bb` | `pfx render --icons shapes --product sample --recipes crates/assets/tests/recipes --assets crates/assets/tests/marks --no-archive --samples 256 --size 512` |
| `scene-shapes.png` | scene (shot) shapes | `2374ebb79147701d1fa2422beed9ae316eed9d81117763015c6ef2a462b0f690` | `pfx render --scene shapes --product sample --still --recipes crates/assets/tests/recipes --assets crates/assets/tests/marks --no-archive --samples 256 --size 512` |

Each command's output lands in `tmp/renders/sample/<kind>/`; the icon set writes `dot` and `ring`, and the scene is one still of the shot `shapes` (two icons in a row, `--still` at 0 s, the first key (turn -15)). The render's `.json` manifest beside each output names the Blender build, the device and the preset's sha256.

Reading them: the background is transparent and the ground shadow is part of the alpha, so view or compare them over a flat colour. A comparison against the tracer's render is by look, not by pixel: the tracer will not match Cycles pixel for pixel, so each kind is checked within a set tolerance and by eye on a contact sheet.
