# The alpha master

H.264 carries no alpha, so a clip whose shot asks for cutouts (`cutouts = true`) gets a second master beside its H.264 `.master.mp4`: a folder of 16-bit RGBA PNG frames that pfx writes itself, with no encoder in between. `pfx_run::alpha` writes and reads it.

```
<stem>.alpha.master/
  000000.png
  000001.png
  …
  manifest.json
```

## The frames

- One PNG per master frame, numbered from `000000.png`, in play order, at the render's size and fps.
- 16-bit RGBA, straight (unpremultiplied) alpha.
- sRGB: BT.709 primaries, D65 white, the sRGB transfer (IEC 61966-2-1), full range. Each PNG carries an `sRGB` chunk (perceptual intent) and the `gAMA` (45455) and `cHRM` fallbacks the PNG specification gives for it, so a reader that ignores `sRGB` still gets the right curve and primaries.
- Alpha is linear coverage, 0 to 65535. Colour where alpha is 0 is 0.
- Lossless: decoding a frame gives back exactly the values pfx wrote.

## manifest.json

```json
{
  "format": "pfx-alpha-master",
  "version": 1,
  "size": [1280, 720],
  "fps": 60,
  "frames": 30,
  "colour": "sRGB (IEC 61966-2-1): BT.709 primaries, D65 white, the sRGB transfer, full range; …",
  "alpha": "straight",
  "channels": "RGBA",
  "bit_depth": 16,
  "rev": "<the app's commit>",
  "files": [{"file": "000000.png", "bytes": 1234567, "sha256": "…"}]
}
```

`rev` is the commit the adapter was built from. `files` lists every frame in order with its byte count and sha256. The render's own manifest lists each frame and `manifest.json` again under `<stem>.alpha.master/`, with the same sha256, so the archive copies and checks the folder like any other master; the `manifest.json` entry carries `"tier": "alpha master"`.

## How the cut-out is made, and where it is quantised

For every traced frame the harness asks the adapter for the frame twice, over a black ground and over a white one (the `frame` request's `ground`). The adapter hands each as 16-bit sRGB-encoded integers, its own one rounding from its float render target. The harness then works in 32-bit float linear light and rounds once:

1. Each capture decodes to linear through an exact 65,536-entry sRGB table.
2. The matte solves alpha from the difference between the two grounds and unpremultiplies the colour, in f32.
3. A loop's crossfade frames mix two mattes premultiplied, in f32. The two captures of every frame a crossfade reads stay in scratch, so a mixed frame is made from the adapter's captures, never from an already rounded cut-out.
4. The result is encoded once to 16-bit: colour through the sRGB curve and alpha linearly, each rounded to the nearest code.

A frame that is not crossfaded is written byte for byte as step 4 made it. Decoding a 16-bit code to f32 linear and encoding it back gives the same code for all 65,536 values, so nothing in the chain rounds twice.

## Cost

Writing a 2160×3840 frame (PNG compression `Fast`) takes 0.126 s on the CPU and about 38 MB on disk for a synthetic busy frame, and reading it back 0.134 s (release build, one core of an AMD Ryzen 9 9950X); a 600-frame vertical 4K clip spends about 76 s writing its alpha master. Nothing runs on the GPU.

## Without encoding

`--no-encode` makes no master: the cut-outs land as loose frames in `<stem>.cutouts/` (`00000.png` …, 16-bit RGBA, straight alpha, sRGB), with no `manifest.json`, and the archive skips the render as frames-only.

## Older renders

Alpha masters made before this format were ProRes 4444 files, `<stem>.alpha.master.mov`. The archive still recognises that name, so queued and archived renders keep working. `pfx encode --manifest` on an older render that kept its `<stem>.cutouts/` writes the new folder from those frames, value for value.

## Archive imports

`pfx archive import --clip` takes a `<name>.alpha.master` folder among its files: every file in it is listed as `<name>.alpha.master/<file>`, and the folder counts as the clip's master once it holds its `manifest.json`.
