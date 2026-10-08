# Flat looks

`pfx_live::flat::look` restyles a flat frame: a game sets a look, a list of passes with their parameters, and switches to another mid-run over a time and an easing it chooses. The passes are building blocks. A game composes its own looks from them and passes its own palettes, token sets, sizes and colours as data; the engine holds none.

The pixel passes themselves live in `pfx_post::look` (`crates/post/src/look/`), with a CPU twin of each, so the gate checks them without a GPU. They read and write Rgba16Float holding sRGB-encoded premultiplied colour, the flat target's own encoding.

A frame with no look, or with a look that has no passes, takes the flat pass's own path: its bytes are the bytes of `render_flat` (`no_look_frames_are_byte_identical`). Gradient fills add a branch to the flat shader and a texture to its view group; frames without them hash the same as before the look stack existed, 35 of 35 (the thirteen reference scenes through `FlatPass` at 2×, and the eleven without groups through the `Renderer` at Rgba16Float 2× and sRGB 1×).

## Using it

```rust
let mut stack = LookStack::new(Look::new())?.base(game_tokens);
stack.switch(retro_look, game_time, Transition::wipe(0.6, Easing::InOut, 0.0, 48.0))?;
renderer.render_flat_look(&scene, &stack.frame(game_time), &output)?;
```

- `LookStack::new(look)` starts on a look; `base(tokens)` names the token set the scene is drawn in, which token swaps replace (below).
- `switch(look, time, transition)` starts a transition at `time`, in the game's own seconds. `set(look)` switches at once.
- `frame(time)` gives the `LookFrame` to draw: the look switched from, the look switched to, the eased progress and the blend. `LookFrame::still(&look, &tokens)` draws one look with no stack.
- `Renderer::render_flat_look` and `submit_flat_look` draw it. `render_flat` and `submit_flat` are unchanged.

Everything follows from the time the game passes in: the stack never reads a clock, so a pfx shot at the same time draws the same frame. `transitions_render_deterministically` renders the middle of a wipe on two renderers and twice on one, and gets the same bytes each time; at progress 0 it gets the look switched from, at progress 1 the look switched to.

**Switching mid-transition.** The new transition starts from the look that is showing more, the one switched from before half way and the one switched to after it. At most half a transition jumps.

**Reduced motion.** `stack.motion(Motion::Reduced { longest })` turns every wipe into a crossfade, drops the easing, and caps the duration at `longest` seconds. `longest: 0.0` makes every change instant. Nothing in a look moves on its own: scanlines, glow and curvature are still images.

## Transitions

`Transition { duration, easing, blend }`:

| Field | Values |
|---|---|
| `duration` | seconds, 0 and up; 0 is instant |
| `easing` | `Linear`, `In` (cubic), `Out`, `InOut`, `Steps(n)` |
| `blend` | `Blend::Crossfade`, or `Blend::Wipe { angle, softness }`: an edge moving along `angle` (radians, 0 is left to right, +y down) with a soft band of `softness` layout units, sweeping the whole target |

The two looks are drawn in full and mixed per pixel, the frame's ids coming from the look switched to.
- In a crossfade both sides take the two token sets mixed perceptually at the progress; in a wipe each side keeps its own.
- A crossfade between looks whose passes differ only in tokens and shadow tint draws once, with the mixed set and tint.

## Passes

`Look::new().with(LookPass::…)`. A look holds at most one `LowRes`, one `Tokens`, one `Shadow` and one `Backdrop`; `Look::validate` says which rule a look breaks.

The order the passes run in:
1. `Tokens` recolours the scene, and `Shadow` retints its cast shadows.
2. The scene draws at the render size: the low size with a `LowRes`, the target's otherwise.
3. `Backdrop` goes under it.
4. `Quantise` and `Vignette` run in the order listed, at the render size.
5. The upscale to the target runs just before the first `Crt`, or at the end; crisp text draws right after it.
6. `Crt` and everything listed after it run at the target size.

So a pixel-art look quantises and dithers on the low-resolution pixels, and a CRT draws its scanlines on the screen's.

### Low resolution

`LowRes { size, upscale, text }` draws the flat layer at `size` pixels, the layout fitted into it as always, then scales it up nearest-neighbour.

- `Upscale::Integer` takes the largest whole scale that fits and centres it; with no whole scale it fits instead. 480×270 into 3840×2160 is ×8 with no border; into 1280×800 it is ×2, 960×540 with a border.
- `Upscale::Fit` fills the target on its limiting axis with uneven pixels: 480×270 into 1280×800 is 1280×720.
- Source pixels follow from whole-number arithmetic, `(x − offset) · source / extent`, the same on the CPU and the GPU. `low_resolution_upscales_pixel_exactly` checks every pixel at 1920×1080, 3840×2160 and 1280×800 (whole and fit) against the low frame rendered alone.
- `LowText::Pixelated` draws text with the shapes, at the low size. `LowText::Crisp` leaves text out of the low frame and draws it at the target size after the upscale, placed exactly where the upscaled board puts it, so labels stay sharp over a pixelated board. Crisp text then draws over everything; it is no longer occluded by shapes above it.
- The border around an upscale is the scene's clear colour, or transparent with a backdrop.
- Picking is unchanged: ids are drawn at the target size from the same scene.

At 1280×800 a quarter-layout look reads better with `Upscale::Fit` than with `Integer`, which leaves a 960×540 board.

### Quantise

`Quantise::new(palette, dither)` maps each pixel to the nearest palette colour in OKLab.

- `Palette::new` takes 2 to 256 sRGB-encoded colours, `Palette::hex` takes `0xRRGGBB`, and `Palette::cube(levels)` builds an evenly spaced cube; `cube(6)` is the 216-colour web-safe cube. Any palette is data.
- Ties go to the first colour.
- `Dither::None`, or `Dither::Bayer { order, spread }` with order 2, 4 or 8: the pixel's OKLab lightness moves by `(threshold − 0.5) · spread` before the search, with the threshold from the standard Bayer matrix, `(M[y mod n][x mod n] + 0.5) / n²`, in the render target's pixels. A spread near the lightness gap between neighbouring palette colours dithers shadows and gradients between them.
- Alpha is kept: the colour is quantised straight and premultiplied back.

The checks:
- `bayer_matrices_are_the_standard_ones` checks the 2×2 and 4×4 matrices and the post chain's own 8×8.
- `dither_patterns_match_their_matrices` quantises five greys to black and white at each order and finds the pattern the matrix predicts, pixel by pixel.
- `quantising_is_exact_for_in_palette_colours`: every colour of three palettes, opaque and at half alpha, comes back bit for bit on the CPU. On the GPU it comes back to the byte (`the_gpu_quantises_like_its_twin`): RADV rounds toward zero when it writes an f16 target, one f16 step under the nearest value, far under half a byte.
- `quantised_looks_hold_their_palette` renders a 16-colour look with Bayer 4 at 1920×1080 and 1280×800 and finds every pixel inside the upscaled board in the palette.

### CRT

`Crt { scanlines, glow, curvature, aberration }`, each `Option`, each separately on.

| Part | Fields | What it does |
|---|---|---|
| `Scanlines` | `period` (layout units), `strength` 0..=1 | multiplies by `1 − strength · (½ + ½ cos 2πy/period)`, y in layout units from the board's top, so a period of the layout-to-low ratio (4 for 480×270 of 1920×1080) puts one dark line between low pixels |
| `Glow` | `threshold`, `radius` (layout units), `strength`, `tint` | a tinted bloom: pixels above the luminance threshold, box-downsampled by the power of two up to 4 that keeps σ at 2 pixels or less where it can, blurred by a separable Gaussian of σ = radius, then added times tint · strength |
| `Curvature` | `amount`, `corner`, `bezel` | barrel warp `w = uv · (1 + amount · uv.yx²)` over the whole target; outside it, the bezel colour; within `corner` (in uv units) of the edge, a smooth falloff into the bezel |
| `Aberration` | `shift` (layout units) | red sampled `shift` outward from the centre, blue inward, scaled by the distance from the centre |

`Curvature::source(pixel, size)` maps an output pixel to the pixel it shows, `None` on the bezel, so a game can carry its pointer through the warp.

### Token swaps

`TokenSet` is colours by key. `LookPass::Tokens(set)` swaps the stack's base set for it: every colour in the scene that matches a base token to the byte (fills, gradient stops, strokes, patterns, glows, text, the shadow curve's colour and the clear colour) takes that key's new colour, and keeps its own alpha relative to the token's. A key the set leaves out keeps the base colour; a colour no token names is left alone. A dark set or a neon set is data.

`TokenSet::mix(&other, t)` crossfades two sets key by key in OKLab, alpha linearly, which is how a crossfade between token sets runs.

**By role.** Matching bytes cannot tell two roles apart when they share a colour, so a draw can name the token each of its colours stands for:
- `TokenKey::new("card.fill")` is a 64-bit hash of the name, a `Copy` value a `const` can hold. It is the key of the `TokenSet` entry of that name, so a game assigns names, not numbers.
- A draw carries `roles: Roles { fill, fill_end, stroke, pattern, glow }`, each an `Option<TokenKey>`, set with `Draw::fill_token`, `fill_end_token`, `stroke_token`, `pattern_token` and `glow_token`. `fill` is a solid fill, or the top of a `Fill::Vertical`, whose bottom is `fill_end`.
- `Shape::Text(index)` draws take the colour of `scene.text[index]` from their `fill` key; the first draw that places a text and names a key decides it (`text_keys`).
- A tagged slot recolours by its key only. If the look's token set holds a different colour for the key than the stack's base set, the slot takes it, keeping its own alpha relative to the base token's (a key the base lacks counts as opaque). If the look leaves the key as the base has it, the slot keeps the colour the draw gave it, even where that differs from the token by a byte or a hundred.
- An untagged slot, a gradient's stops, the shadow curve's colour and the clear colour match by bytes as before, so a scene with no keys recolours exactly as it did.
- Crossfades mix the sets as before and the keyed colours follow: `keyed_tokens_crossfade_with_the_sets`.
- A frame with no keys, or whose tokens equal the base set, keeps its bytes: `keyed_roles_render_apart_and_keep_the_bytes_of_an_unkeyed_frame` renders two squares of one grey as red and blue, then compares tagged, untagged and identity frames bit for bit.

### Shadow tint

`LookPass::Shadow(ShadowTint { colour, alpha })` retints every cast shadow of the scene for the look, which a dark look needs where the game's curve is made for a light board.
- `colour: Option<Srgba>` replaces the curve's colour, alpha included, when set; `alpha` scales the colour's alpha afterwards (default 1.0, finite and zero or more), the result capped at 1. The curve's own per-elevation alphas, offsets and σ stay the game's.
- It applies to `Cast` and `Turned` shadows. A `Glow` is light the draw chose a colour for, and keeps it.
- It runs after the token swap, so a tint's colour is exactly what the look says.
- A crossfade mixes the tinted curve colours in OKLab, alpha linearly, with the transition's progress, and draws once when the passes differ only in tokens and tint; a wipe draws each side with its own tint. `ShadowTint::new()` and no pass are the same frame.
- `shadow_tints_crossfade_with_the_transition_and_wipe_apart` checks the colours at progress 0, ½ and 1 and in a wipe. `a_look_shadow_tint_draws_the_frame_of_the_curve_it_stands_for` renders the board through the tint, and a half-way crossfade, on the GPU and finds them bit for bit the frames of plain `render_flat` with the curve those colours make.

### Backdrop

`Backdrop { colour, gradient, grid }` goes under the flat layer, which then draws on transparent instead of its clear colour. All three are in layout units and cover the whole target, border included.

- `gradient`: a `Gradient` below, such as a sky behind the board.
- `grid`: `Grid { spacing, origin, width, colour, major }`, lines anti-aliased by their pixel coverage; `major: Some(Major { every, width, colour })` draws every nth line over them.

### Gradients

`Gradient::linear(from, to, stops)` and `Gradient::radial(centre, radius, stops)` take 1 to 8 stops of `(offset, colour)`, with sRGB-encoded colours and straight alpha.
- Offsets are clamped to 0..=1 and to the offset before them, as SVG does; a repeated offset is a hard stop.
- Past the ends the end colours hold.
- `.space(Interpolation::Oklab)` mixes between stops in OKLab. The default, `Srgb`, mixes encoded colour, as SVG and `Fill::Vertical` do.
- A radial gradient's `radius` is two radii, for an ellipse.

As a fill, `Fill::Gradient(gradient)` paints a shape in its own local units, so the gradient turns and scales with the shape, and works under strokes, patterns, opacity, groups and the lit dials. The pass packs each frame's distinct gradients into one small Rgba32Float texture, twelve texels a row, read with `textureLoad`; a scene with none binds a 1×1 placeholder. `gradient_fills_match_the_cpu_twin_on_the_gpu` holds the GPU to the CPU twin at 1920×1080 and 1280×800 (mean under 0.15/255, worst 2/255).

### Vignette

`Vignette { centre, radius, falloff, floor, colour }` lights the frame as a pool: full light inside the ellipse of `radius` around `centre` (layout units), falling with a smoothstep over `falloff` (a fraction of the radius) to `floor`, the light left outside. The shade mixes toward `colour`, whose alpha is the strength. It runs at the render size.

## The floor

The look passes are full-screen triangles into Rgba16Float with core WebGPU only:

| Need | Default | Look passes |
|---|---|---|
| bind groups | 4 | 1 |
| sampled textures per stage | 16 | 2 |
| samplers per stage | 16 | 1 |
| uniform buffers per stage | 12 | 2 (the palette pass) |
| dynamic uniform buffers per layout | 8 | 2 |
| uniform binding size | 65536 | 8192 (256 palette entries in OKLab and sRGB) |

`the_look_shader_validates_and_fits_webgpu_defaults` checks both layouts. The flat pass's view group gains one non-filterable texture for gradient stops (3 sampled textures in all); `every_flat_layout_fits_webgpu_defaults` still holds it to the defaults.

Each pass is one draw of a few texture reads per pixel, except the quantiser's palette search (up to 256 distances a pixel, so at full size it is the costliest pass) and the glow's downsample. On a Deck-class GPU the costs below scale as the flat pass's do (`docs/flat.md`, 8× to 16× this machine).

## Costs

GPU time on this machine (RX 9060 XT, RADV), medians of 60 frames after a warm-up round, from `look_passes_cost_at_4k_and_deck_size`. The scene is the engine's neutral test board of the look tests: 48 draws (cards with shadows and strokes, discs, bars, a gradient pill, a patterned ring) over a 1920×1080 layout. "Flat" is the flat pass at the render size; the look passes are the rest. The total leaves out `flat ids`, which looks don't change.

| Look | 3840×2160 flat | look passes | total | 1280×800 flat | look passes | total |
|---|---|---|---|---|---|---|
| no look | 0.957 | — | **0.957** | 0.124 | — | **0.124** |
| tokens (a dark set) | 0.960 | — | 0.960 | 0.124 | — | 0.124 |
| backdrop: gradient and grid | 0.958 | backdrop 0.406 | 1.364 | 0.124 | backdrop 0.052 | 0.176 |
| vignette | 0.962 | vignette 0.219 | 1.184 | 0.124 | vignette 0.029 | 0.153 |
| quantise 16, full size | 0.961 | quantise 0.334 | 1.296 | 0.124 | quantise 0.046 | 0.169 |
| quantise 16, Bayer 4, full size | 0.959 | quantise 0.461 | 1.419 | 0.124 | quantise 0.062 | 0.186 |
| quantise 216 (web cube), Bayer 4, full size | 0.943 | quantise 1.942 | 2.888 | 0.123 | quantise 0.264 | 0.387 |
| scanlines | 0.965 | crt 0.270 | 1.232 | 0.124 | crt 0.029 | 0.154 |
| phosphor glow (radius 6) | 0.960 | glow 0.603, crt 0.221 | 1.788 | 0.124 | glow 0.054, crt 0.030 | 0.208 |
| curvature and aberration | 0.967 | crt 0.221 | 1.188 | 0.125 | crt 0.030 | 0.155 |
| low resolution 480×270 | 0.025 | upscale 0.221 | **0.246** | 0.039 | upscale 0.045 | **0.084** |
| low resolution, quantise 16 with Bayer 4 | 0.025 | quantise 0.012, upscale 0.221 | 0.258 | 0.038 | quantise 0.018, upscale 0.045 | 0.102 |
| full stack: low resolution, quantise and Bayer 4, then CRT (scanlines, glow, curvature, aberration) | 0.025 | quantise 0.012, upscale 0.221, glow 0.604, crt 0.262 | **1.125** | 0.039 | quantise 0.018, upscale 0.045, glow 0.083, crt 0.048 | **0.233** |

What sets them:
- A full-size pass that reads and writes Rgba16Float once (vignette, CRT, upscale) costs about 0.22 ms at 4K and 0.03 ms at 1280×800: it is bandwidth.
- The quantiser keeps its palette sorted by OKLab lightness and searches outward from the pixel's lightness, stopping once lightness alone is farther than the best match; that took the 216-colour cube at 4K from 3.18 to 1.94 ms with the same answers. A pixel-art look quantises at its low size, where 16 colours cost 0.012 ms.
- The glow downsamples by at most 4: at 8, each pixel's 64 serial reads cost more than the full-size reads they save (1.10 ms against 0.60 at 4K).
- A low-resolution look draws the flat pass at 480×270, so the full stack at 4K costs less than the flat pass alone at 4K.
- Runs differ by up to 0.03 ms at 1280×800 with the GPU's clocks.

At 8× to 16× this machine for a Deck-class GPU, the full stack at 1280×800 is 1.9 to 3.7 ms of 16.7 (60 fps) or 11.1 (90 fps), and the costliest single look here, the 216-colour cube at full size, 3.1 to 6.2 ms.

## Not covered

- Shapes snapping to a character grid, outline-only and bevel styles and inner viewports are the flat pass's shape styles, not looks; a terminal look is a character-grid scene under a `Crt`.
- Glow and curvature have no CPU twin; their GPU test checks that they run, stay finite and draw the bezel.
- A third look switched in mid-transition jumps by up to half a transition.
