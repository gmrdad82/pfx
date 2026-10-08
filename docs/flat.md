# The flat pass

`pfx_live::flat` draws a game board: flat 3D surfaces of analytic shapes under a fixed orthographic camera, with analytic soft shadows, at native 4K. It is a pass of its own inside `pfx-live`, not the PBR opaque pass. It shares the `Renderer`'s device, picking ids, text shader and post chain. The engine is generic: a game brings its own layout, colours, shadow curve, dial values and patterns, and the pass carries none of them.

Every dial and every option defaults to zero or off. With them off, a frame reproduces a reference SVG renderer (headless Chromium): under 0.5/255 mean difference for every test scene, at 1× and 2× (below).

## The scene

A `FlatScene` is everything one frame needs:

| Field | Meaning |
|---|---|
| `layout` | The game's units, e.g. `[1920, 1080]`. Everything below is authored in them. |
| `clear` | The colour behind the board and the letterbox, or `None` for transparent (a HUD). |
| `curve` | The game's `ShadowCurve`: shadow offset, σ and alpha by elevation. |
| `light` | `Light { shadow, key, ambient }`: the layout direction shadows fall in, the key light for the dials, and the ambient floor of slab shading. |
| `draws` | The ordered list of `Draw`s. |
| `groups` | `Group { opacity }`s that draws join by index. |
| `text` | `FlatText`s that `Shape::Text(index)` draws place. |
| `icons` | The scene's `FlatIcons`, an uploaded distance-field atlas. |
| `sprites` | The scene's `FlatSprites`, an uploaded colour atlas that image fills sample, or `None`. |
| `environment` | The one prefiltered `Environment` the reflection dial samples. |
| `post`, `frame`, `seed` | Whether the frame runs through the renderer's post chain, and the chain's frame number and seed. |

A `Draw` is one shape:

| Field | Meaning |
|---|---|
| `transform` | A full 3D transform (column-major 4×4) from the shape's local plane to layout units. x runs right, y down the layout, z toward the viewer. |
| `elevation` | Height above the board in the curve's units. Sets painter's order and the shadow. |
| `shape`, `fill`, `stroke` | What is drawn, below. |
| `opacity` | Per instance. The fill and stroke composite as one, then fade, as SVG's element `opacity` does. |
| `shadow` | `Cast` (default), `Turned`, `Glow(Glow)` or `None`. |
| `material` | The dials, all zero by default. |
| `pattern` | An optional pattern over the fill. |
| `id` | Written to the ids target for `pick`. |
| `group` | `Some(index)` joins `groups[index]`. |
| `style` | An optional shape style: outline, ghost, bevel or flat (`docs/flat-styles.md`). `None` draws as above, byte for byte. |
| `roles` | Optional token keys per colour slot, for looks (`docs/flat-looks.md`). All `None` by default. |

`Draw::rect`, `Draw::circle` and `Draw::new(shape)` build a draw. The builder methods (`at`, `transform`, `elevation`, `fill`, `vertical`, `stroke`, `opacity`, `shadow`, `material`, `pattern`, `id`, `group`, `style`, and the role keys `fill_token`, `fill_end_token`, `stroke_token`, `pattern_token`, `glow_token`) set its fields. `rect_corners` builds a rect with four radii. `place`, `translation`, `rotation`, `scaling`, `tip_x`, `tip_y` and `multiply` build transforms.

### The camera

The camera is fixed: it looks straight down the board, orthographically. `Fit::new(layout, target)` maps layout units to target pixels:
- one uniform scale, the smaller of the two axis ratios;
- centred, with the offset rounded to the nearest whole pixel, so a layout edge on a whole unit stays on a pixel edge.

For example, 1920×1080 into 3840×2160 is scale 2, offset 0. Into 1280×800 it is scale 2/3, with 40 pixels of letterbox above and below. `to_target`, `to_layout`, `viewport` and `clip_from_layout` convert. The render size is the renderer's, so a weaker GPU renders the same layout at a lower size.

**One letterbox.** `Fit` is built from the window layer's letterbox, `pfx_gpu::window::letterbox_nearest`, which is `letterbox_with(layout, target, LetterboxRounding::Nearest)`. `fit.letterbox` holds it.
- A game that builds its viewport with `viewport_with(window, scale, layout, LetterboxRounding::Nearest)` gets `Viewport::pointer_to_layout` from the very same map, so the pointer and the board can't disagree by a pixel.
- The window layer's default stays `LetterboxRounding::Floor`, which floors the visible extent and the centring gap, as before.
- Where both give whole offsets the two roundings are identical: 16:9, 21:9, 4:3, 4K and 1280×800. They differ by a pixel where centring leaves a half pixel: 100 into 201 is 50 floored and 51 nearest.
- `flat::tests::the_layout_letterboxes_into_the_target` checks `Fit` against the window layer for six targets.
- Which layout a game uses on which screen, native 16:10 on a Deck and boxed elsewhere, comes from its aspect policy: `ScreenPolicy::fit` gives the layout, the content and safe rects, the bar colour and a UI scale for the Deck's 7-inch panel, and its letterbox is this same map (`docs/screens.md`).

A shape's transform reaches the screen as the 2D affine map of its local plane, since the projection drops z. A shape tipped out of the plane is foreshortened, its edges stay analytic, and it casts from that projected footprint.

## Primitives

All primitives are instances of one shared quad: four vertices, a triangle strip, one 240-byte instance each (15 `vec4` attributes). Each is evaluated as a signed distance field in its local plane, in the fragment stage.

| `Shape` | Local geometry | Fill paints |
|---|---|---|
| `Rect { half, radii }` | rect centred on the origin with four corner radii, `[top left, top right, bottom right, bottom left]`; each is clamped to the smaller half extent | the rect |
| `Circle { radius }` | circle on the origin | the disc |
| `Ring { radius, width }` | band of `width` on the circle | the band |
| `Arc { radius, width, start, sweep }` | the band from `start` through `sweep` radians, measured from +x toward +y (clockwise on screen), with round caps | the band |
| `Icon(Icon)` | a field from the scene's icon atlas, in the icon's own path units | a filled icon's inside; a line icon has no fill |
| `Text(index)` | `scene.text[index]`, placed by the transform | |

**Corners.** `Shape::Rect` carries one radius per corner, clockwise from the top left (y runs down, so top is −y). `Draw::rect(centre, size, radius)` and `Shape::rounded(half, radius)` give all four the same radius; `Draw::rect_corners(centre, size, radii)` and `Shape::corners(half, radii)` take four.
- Every part of the rect honours them: the distance field and its CPU twin, fills, strokes, dashes (the perimeter runs top edge, top-right arc, right edge, and so on, each piece as long as its own radii leave it), patterns, outlines, ghosts, bevels, lit slabs, picking, motion blur and the shadow (the row integral takes the left and right radius of the half it is in).
- A square corner (radius 0) next to round ones is the one shape a band needs to sit flush in a rounded edge.
- Equal radii run the same arithmetic as the single radius before: the thirteen reference scenes and every byte-identity test hold (`equal_corners_are_the_single_radius_field`, `equal_corners_render_the_bytes_of_one_radius`).
- A motion-blurred rect whose corners differ takes the marched blur, not the closed form, which needs one radius; `blur_matches_the_integral_of_moving_shapes` holds it to the brute-force integral with "corner rect", "corner outline" and "corner dashes".
- Not done: clipping a draw to another draw's footprint. It does not fall out of this change, since it needs a second field per pixel in the shader.

An SVG progress meter is an `Arc`. A circle with `stroke-dasharray="L C"`, `stroke-linecap="round"` and a −90° rotation is `Arc { start: 0, sweep: L / r }` under that rotation.

**Fills.** `Fill::Solid(Srgba)`, `Fill::Vertical(top, bottom)` or `Fill::Gradient(Gradient)`, an n-stop linear or radial gradient in the shape's local units (`docs/flat-looks.md`).
- `Vertical` is a two-stop linear gradient across the shape's own bounding box (SVG's `objectBoundingBox` with `y2="1"`), so it turns with the shape.
- Each stop is sRGB-encoded colour plus straight alpha.
- Stops mix as straight colour, then premultiply.
- `Fill::Image(SpriteFill { frame, tint })` paints a frame of the scene's sprite atlas (`FlatScene::sprites`) across the shape's own box: `frame` is the frame's `[u0, v0, u1, v1]` in the atlas, as `pfx_core::anim::Atlas::uv` and `SpritePlayer::uv` give it, and each texel is multiplied by `tint` (white by default). `Draw::sprite(centre, size, frame)` is a square-cornered rect with it; any rect, circle, ring or arc takes it, its corners and outline cutting the frame as they cut a fill. [animation.md](animation.md) has the sprite clips that step it by tick.
  - The atlas is a `FlatSprites`, uploaded once from a `SpriteImage` (RGBA8 bytes, sRGB-encoded like every flat colour, filtered `Linear` or `Nearest` for pixel art). It is sampled as stored, without decoding, so a texel shows its own bytes.
  - An icon, a style (outline, ghost, bevel, flat) or motion blur draws an image fill as its tint; a pattern is ignored on it, since both use the instance's icon slot. Picking takes the shape's footprint, whatever the texels' alpha.
  - The CPU twin samples the same atlas (`SpriteImage::sample`, bilinear or nearest with clamped edges) from `scene.sprites`; `cpu::paint_sprite` paints one instance with it.

**Strokes.** `Stroke { colour, width, dash }` is centred on the outline, as SVG's is, and paints over the fill.
- `Stroke::dashed(colour, width, on, off)` dashes a rounded rect's perimeter, or a circle's.
- The perimeter starts where SVG's `<rect>` path starts, at the top edge just right of the top-left corner, and runs clockwise.
- Dashes have butt ends. `Dash::phase` is `stroke-dashoffset`.

**Icons.** An `IconShape` is filled polygons, or open polylines plus circles. `Icons::bake` turns a list of them into one R16Float atlas of exact distances, 160 texels along each icon's longest side plus its `reach`.
- A filled icon stores the signed distance to its polygons, so a stroke on it is an outline with round joins.
- A line icon stores the distance to its centre lines, so its stroke has round caps and round joins at any width.
- Widths are in the icon's units, so a stroke stays proportional when the icon scales.

`flat::icons::standard()` is a small generic set in its own units:

| Icon | Geometry |
|---|---|
| `pointer` | an arrow pointer, `(0,0) (0,52) (12,41) (21,60) (30,56) (21,37) (37,37)`, filled |
| `tick` | `(−10,1) (−3,9) (11,−8)`, line |
| `plus`, `cross` | arms of 16 and 14 units, lines |
| `arrows` | a left and a right arrow 140 units apart, lines |
| `menu` | three 50-unit bars, lines |
| `search` | a ring of radius 9 and a handle, lines |
| `page`, `fold` | a page with a cut corner, and its fold, filled |
| `funnel` | a filter funnel, filled |

**Opacity.** Per instance, as above. Per group, a group's members draw into a layer, and the layer composites at the group's opacity. A stack at 0.5 therefore reads as one faded card, with no darker overlap, and its shadow fades with it, as SVG's `<g opacity=".5" filter="…">` does.

**Patterns.** `Draw::pattern(Pattern { kind, scale, angle, width, colour })` lays a second paint over the fill, so colour need not be the only cue. The kinds are `Stripes`, `Dots`, `CrossHatch` and `Chevrons`.
- Each is a signed distance field in the shape's own plane. `scale` is the period in local units, `angle` turns the pattern, and `width` is the line or dot size as a fraction of the period.
- `colour` is the second colour or alpha, packed as RGBA8.
- The pattern composites over the fill, inside the shape's own analytic edge. It is anti-aliased by its screen gradient like every edge, so it stays crisp at any size.
- It works on rects, circles, rings and arcs. On icons it is ignored.
- A pattern on a shape with no fill paints over transparent.
- Unused, it costs nothing. The kind lives in three flag bits, and the parameters ride in the instance's icon slot and spare `info` word. A shape without a pattern packs the same bytes and skips the evaluation.

`patterns_match_their_reference_renders` compares each kind with a Chromium render of the same geometry, drawn as explicit paths clipped to the shape, at 1× and 2×.

**Patterns at Deck size.** At 1280×800 (layout scale 0.667), four patterns stay distinct inside an 8-unit stripe (5.3 px). They were tested in white at 30% over a saturated fill:

| Pattern | `Pattern` | Distinct down to a period of | at 1280×800 |
|---|---|---|---|
| diagonal stripes | `Stripes`, angle 45°, width 0.45 | 8 layout units | 5.3 px |
| dots | `Dots`, width 0.5 | 10 | 6.7 px |
| horizontal bars | `Stripes`, angle 90°, width 0.45 | 6 | 4.0 px |
| cross-hatch | `CrossHatch`, angle 45°, width 0.25 | 10 | 6.7 px |

At a period of 12 layout units (8 px at 1280×800, 24 px at 4K) all four read apart in the stripe.

The minimums were judged on a Deck-size render at periods 12, 10, 8, 6, 5 and 4: below them, dots and hatch dissolve into texture. They share a rule, which `patterns_hold_their_shape_at_deck_size` checks:
- the period is at least 4 px;
- the smallest line or gap is at least 1.1 px;
- in an 8× supersampled ideal, the pattern keeps at least half its contrast.

`the_pattern_sheet_renders_at_deck_size` draws the sheet.

**Scrims.** A full-layout `Rect` with a fill alpha or an opacity.

**Glyphs.** A `FlatText` names a `GpuAtlas`, either a `Paragraph` or `RichQuad`s, and a colour.
- A `Shape::Text(index)` draw places it with its transform, at its elevation, in painter's order with the shapes.
- It runs the existing text shader (`text::shader_source`, `vs_main` and `fs_main`) in surface space, with the camera times the draw's transform as its matrix.
- MSDF and alpha atlases are colour-space neutral. Colours are sRGB-encoded, like every flat colour.

## Shadows

Every caster's shadow is an analytic Gaussian blur of its rect (with its four corner radii) or circle, in closed form along one axis:
- The blur of a box is exact through `erf` along x.
- Along y it integrates numerically over 8 intervals spaced by `asin` across the box, so they gather at the rounded corners. Each interval takes the exact Gaussian mass between its ends, through `erf`, times the row's `erf` integral at its middle.

This is the known fast rounded-rect shadow (Evan Wallace's), with exact interval weights and corner-dense spacing. Against a brute-force 2D blur:
- the worst error over six casters is 0.007 of full shadow, for a 40-unit circle at σ 4;
- a 180×60 card at σ 5 is within 0.0022.

The erf is Abramowitz and Stegun 7.1.26 (error under 1.5e-7). Rings and arcs cast as their outer circle; icons and text cast nothing.

**The curve.** `ShadowCurve::new(colour, points)` takes `CurvePoint { elevation, offset, sigma, alpha }`, in layout units.
- Between points it is piecewise linear.
- Below the first point it falls linearly to nothing at elevation 0. Above the last it holds.
- Elevation 0 and below cast nothing.
- `ShadowCurve::none()` casts nothing at all. There is no built-in curve: the game sets its own.

An example, the one the engine's tests use:

| Elevation | Use | offset | σ | alpha |
|---|---|---|---|---|
| 0 | the board | — | — | — |
| 1 | tiles and pills | 4 | 5 | 0.22 |
| 1.5 | interpolated | 8 | 10.5 | 0.24 |
| 2 | sheets | 12 | 16 | 0.26 |

Its colour is `#2b3442`. Layout units scale with the fit, so the curve is the same at every resolution.

**Direction.** `Light::shadow` is the layout direction the offset falls in. Moving it moves every `Cast` shadow; an effect can animate it. `Shadow::Turned` takes the direction in the shape's own frame instead, so the offset turns with the shape, as SVG does for a filter on a rotated group.

**Glow.** `Shadow::Glow(Glow { colour, sigma })` is the same blur with offset 0 and its own colour and alpha. A glow-only draw has no fill, like SVG's `<circle opacity=".25" filter="url(#halo)">` with a plain Gaussian blur.

**Order.** Draws composite in painter's order by elevation.
- Ties keep the list's order.
- A group is one unit, at its lowest member's elevation.
- Each draw's shadow composites first, over everything beneath it, then the draw, as SVG and CSS do.
- A label or icon on a card shares the card's elevation and follows it in the list.

**Tipped shapes.** The shadow is evaluated in the caster's local plane, with σ divided by each local axis's length on the board. That is exact for a shape turned in the plane or tipped about one of its own axes. A sheared footprint is approximated by its two axis lengths.

## Edges and colour

Coverage is analytic. Each field returns its distance and its local gradient. The gradient maps to screen space through the instance's inverse affine, and the coverage is `clamp(0.5 − d / |∇d|, 0, 1)`. That is a one-pixel ramp, exact for straight edges and correct under foreshortening. Icon fields use the transform's mean scale. There is no TAA, no MSAA and no tone mapping. Colours are premultiplied.

**Compositing is in gamma-encoded sRGB**, as browsers (headless Chromium, Skia) composite SVG. This was measured, not assumed. `references_composite_in_gamma_encoded_srgb` renders the reference scenes with partial alpha both ways, against Chromium's PNGs:

| Scene | gamma-space mean | linear-light mean |
|---|---|---|
| card | 0.36/255 | 1.02/255 |
| faded | 0.43/255 | 1.99/255 |
| dashed | 0.13/255 | 4.86/255 |
| glow | 0.25/255 | 1.42/255 |
| scrim | 0.02/255 | 27.47/255 |
| pill | 0.31/255 | 1.10/255 |

The flat target is Rgba16Float holding sRGB-encoded premultiplied colour, so a shadow over the background lands on the reference's exact values. Where the frame goes next:
- **An Rgba16Float output with no post:** the flat pass draws straight into it, already encoded, as the post chain's `Encode` leaves an Rgba16Float output.
- **An sRGB output** (`Rgba8UnormSrgb`, `Bgra8UnormSrgb`): the "flat finish" pass decodes to linear, and the hardware encodes it back.
- **`post: true`:** the finish decodes into the frame's linear HDR target, then the renderer's post chain runs (`set_finish`, `set_tape`, the tape glitch included). Exposure is fixed at 1. A chain with no tone (`Chain { passes: vec![] }` plus a `Tape`) keeps the colours exact.
- **A HUD over a 3D frame:** drawn after the 3D frame's post, its text and its ids, into a transparent flat target, then composited over the frame. It blends encoded over an encoded Rgba16Float output, or premultiplied linear over the linear display intermediate of an sRGB output. `post` is ignored there.

## Material dials

`Material { thickness, bevel, gloss, roughness, reflection }` defaults to zero, and at zero nothing changes.
- A draw whose dials are all zero packs the very same instance bytes as a draw before the dials existed.
- It is drawn by the dial-free pipeline, whose shader is exactly `shader::flat_source()`, the code without the lit path. `dials_at_zero_render_the_bytes_of_the_dial_free_shader` renders every test scene through both and compares every bit of colour and every id.
- A lit draw uses a second pipeline, `flat_colour_lit`, created only when one appears.

**Thickness and bevel** (rounded rects, circles and icons): the shape is a slab of that depth below its plane, with a rounded edge of radius `bevel`. It is evaluated as a 3D SDF: a rounded box, a rounded cylinder, or the icon's own field with its outline, extruded.
- Seen straight down (no tip), the slab needs no march. Its surface at a pixel follows in closed form from the 2D footprint distance `d` and its gradient:
  - flat top where `d ≤ −bevel`;
  - on the rim, height `√(bevel² − (d + bevel)²) − bevel` with the matching normal;
  - coverage is the footprint's own analytic edge.
- A tipped slab is sphere-marched along the view ray, at most 32 steps, inside the projected bounds grown by its reach, and shows its side and bevel. Its normals are the SDF's central-difference gradient, and its silhouette coverage comes from the ray's closest approach.
- Shading is `1 + (1 − ambient) · (clamp(n·key / up·key, 0, 1.5) − 1)`, which is exactly 1 on a face looking straight up.
- Dial values are layout units. They become the shape's local units through the length of its transform's x axis, so a scaled icon gets the same depth as a card.
- An icon's thickness is capped at an eighth of the shorter side of its cell on the board. Its bevel is capped at half that thickness, and, for a line icon, at half its stroke width. A small pointer and a tick then share the cards' light without being swallowed by it.

**Gloss**: GGX specular from `light.key`, with `roughness` and Schlick Fresnel (F0 0.04), scaled by the dial.

**Reflection**: Fresnel-weighted, sampled from the scene's one `Environment` along the reflected view vector, at the mip `roughness` picks, scaled by the dial.
- `Environment::new` takes a linear equirect and box-filters its mips. u comes from `atan2(x, −y)`, and v from the angle to +z, the viewer.
- There are no screen-space or traced reflections.

**Zero on top.** Both gloss and reflection are weighted by how far the normal leans from up (`length(n.xy)`, the sine of its tilt), so they light only bevels, sides and tipped faces.
- The lit colour is composed as the flat colour plus the change the lighting makes. An up-facing face therefore keeps its exact bytes at any dial setting, dark colours included.
- `up_facing_faces_keep_their_token_bytes_at_bold` checks 16 colours (rects, turned circles, a pointer icon) at high dials with an environment, bit for bit.

Lighting runs in linear light: the fill colour is decoded, lit, then encoded back to the board's sRGB.

**Batching.** Painter's order interleaves lit and flat draws, such as a lit tick above its flat trail, 500 times over. Each switch between the flat and lit pipelines would cost a draw call, so a lit run absorbs any following flat instance smaller than 128×128 pixels.
- The lit entry points draw a flat instance through the same `flat_paint` the flat pipeline uses.
- A large flat shape still starts a flat run.
- A scene with every dial at zero never builds the lit pipelines at all.

### Parameters and defaults

Every field, its unit and its default. Defaults are zero or off: an unset field changes nothing.

| Type | Field | Unit and range | Default |
|---|---|---|---|
| `Draw` | `transform` | 4×4 column-major, local plane to layout units | identity |
| | `elevation` | curve units; order and shadow | 0 (on the board, no shadow) |
| | `shape` | see Primitives | — |
| | `fill` | `Fill::None`, `Solid(Srgba)`, `Vertical(top, bottom)`, `Gradient(Gradient)` | `None` |
| | `stroke` | `Stroke { colour, width, dash }`, width in local units | none |
| | `opacity` | 0..=1 | 1 |
| | `shadow` | `Cast`, `Turned`, `Glow(Glow { colour, sigma })`, `None` | `Cast` |
| | `material` | `Material` below | all zero |
| | `pattern` | `Option<Pattern>` below | none |
| | `id`, `group` | u32; `Option<u16>` index into `groups` | 0; none |
| | `style` | `Option<Style>`, see `docs/flat-styles.md` | none |
| `Dash` | `on`, `off`, `phase` | local units along the perimeter | — |
| `Pattern` | `kind` | `Stripes`, `Dots`, `CrossHatch`, `Chevrons` | — |
| | `scale` | period, in the shape's local units | — |
| | `angle` | radians, turning the pattern in the shape's plane | — |
| | `width` | line or dot size as a fraction of the period, clamped 0..=1 | — |
| | `colour` | sRGB plus alpha, packed to RGBA8 | — |
| `Material` | `thickness` | layout units, slab depth | 0 |
| | `bevel` | layout units, edge radius; capped at thickness/2 and the shape's half size | 0 |
| | `gloss` | GGX scale, 0 and up | 0 |
| | `roughness` | GGX roughness and the environment mip, clamped 0.02..=1 | 0 (used as 0.02) |
| | `reflection` | environment scale, 0 and up | 0 |
| `Light` | `shadow` | layout direction shadows fall in | `[0, 1]` |
| | `key` | key light direction (x right, y down, z toward the viewer) | `normalize(0, −0.5, 1)` |
| | `ambient` | slab shading floor, 0..=1 | 0.55 |
| `ShadowCurve` | points | `CurvePoint { elevation > 0, offset, sigma ≥ 0, alpha 0..=1 }`, layout units; piecewise linear, from zero at elevation 0, held past the last | `ShadowCurve::none()` casts nothing |
| `Environment` | texels, `intensity` | linear equirect up to 4096 a side, box-filtered mips; intensity ≥ 0 | none: reflection has nothing to sample |
| `Group` | `opacity` | 0..=1, the layer's opacity | — |

Inner viewports, shape styles and the character grid are in `docs/flat-styles.md`.

## Picking

Every shape instance writes its `id` to the renderer's ids target (R32Uint) in a second pass, "flat ids", in painter's order. It writes wherever its footprint's coverage reaches 0.5 and it has a visible paint.
- Shadows write nothing.
- An id of 0 still covers what is beneath.
- Text draws with a nonzero id write it where their glyphs cover.

`Renderer::pick` and `picked` work unchanged. On a flat frame `Picked::depth` is 0. `flat::cpu::pick` gives the same answer on the CPU; the GPU's ids match it on 99.9% of a scene's pixels.

## The Renderer

- `render_flat(&FlatScene, output)` and `submit_flat(...)` draw a flat scene alone.
- `render_flat_look(&FlatScene, &LookFrame, output)` and `submit_flat_look(...)` draw it through a look: low resolution, palettes, CRT, token swaps, backdrops and transitions (`docs/flat-looks.md`).
- `render_with_flat(scene, text, effects, style, &FlatScene, output)` and `submit_with_flat(...)` draw a 3D frame with a flat HUD over it.
- `render` and `submit` are unchanged. The `FlatPass` is created on first use, so a renderer that never draws a flat scene builds no flat pipelines.

A frame through `render` is byte-identical before and after this pass existed: ten frames of a test scene, Rgba16Float and sRGB, hash the same on the base commit and here. An empty HUD leaves a 3D frame's bytes unchanged.

Pass order, as `last_pass_order` reports it:
- alone, Rgba16Float output: `flat`, `flat ids`;
- alone, sRGB output: `flat`, `flat finish`, `flat ids`;
- alone with post: `flat`, `flat finish`, `post`, `flat ids`;
- as a HUD: the 3D frame's passes, then `flat`, `flat finish`, `flat ids`.

**Warming.** `Renderer::warm_flat(format, size, looks)` builds what the first flat frame would otherwise build, without drawing a frame:
- the flat pass: its colour, ids, clear, composite and text pipelines;
- the lit pipelines, which a scene with every dial at zero never needs, so a game that raises a dial later pays nothing on that frame;
- the finish for `format`, the finish into the post chain's HDR target, and the HUD finish (all three, when they differ);
- the look passes: `FlatLooks` with its nine pipelines, and a flat pass in every slot the given looks draw through (the board's own side, a low-resolution side, and the crisp-text pass of each), each with its lit pipelines and group layer;
- the colour target and group layer at `size`, so a frame at that size allocates neither.

`looks` is any iterator of `&Look` (`&Vec<Look>`, `&[look_a, look_b]`); pass every look the game switches between, since a transition draws the look it leaves and the look it enters. A look that fails `validate` is an error, and so is a zero `size`. An identity look needs nothing. Warming twice makes nothing the second time, and it is safe at any point: call it while the game loads, or after a settings change picks a new output format.

`Renderer::flat_pipelines_made_on_render_thread()` counts the flat pipelines this renderer has made on the thread that calls it, warming included, as `Frame::opaque_made_on_render_thread` does for the opaque pass: six for the flat pass, two more for its lit pair, one per finish, and the same for each look-pass slot, plus the nine of the look passes' own set (`LookGpu::pipelines()`, made together the first time a look draws or warms). A game warms, reads the counter, draws its first real frame with every look it uses, and asserts the counter has not moved.

`a_warmed_renderer_makes_no_flat_pipeline_on_its_first_frames` (`--ignored`, by name under pgpu) does exactly that on an Rgba16Float and an sRGB output: it warms five looks (a gradient backdrop with tokens, crisp-text and pixelated low resolution with a palette, a CRT, a vignette), draws a lit scene plain, through post, through every look, and through a wipe and a crossfade mid-transition, and requires the count unchanged after each. `warming_plans_a_pass_for_each_slot_a_look_draws_in` holds the slot plan on the CPU.

**Headless and offscreen.** The renderer renders into any texture view, so a tool renders a board to the same pixels a game shows. `examples/flat_board.rs` renders the engine's neutral test board (`crates/live/tests/flat/board.svg`) through the scene API, with the dials as flags. It can put the reference render beside it, cut 2× crops, and lift and tip one card:

```
cargo build --release -p pfx-live --example flat_board
pgpu run --class clip --as pfx -- target/release/examples/flat_board --look soft --out tmp/flat-board.png
```

`--look` takes `zero`, `soft` (thickness 2, bevel 1, gloss 0.15, reflection 0.1, roughness 0.45) or `strong` (10, 4, 0.6, 0.5, 0.25). `--thickness`, `--bevel`, `--gloss`, `--roughness` and `--reflection` set the dials one by one. `--svg` reads any board in the supported subset.

**Instances are pooled.**
- The instance, glyph, text-uniform and layer buffers grow by powers of two and are rewritten in place each frame.
- The scratch lists are cleared, not reallocated.
- Bind groups are cached by the views they bind.
- Text uniforms use dynamic offsets into one buffer, so text adds no bind group per label.

Thousands of short-lived instances a second cost a buffer write, not an allocation.

## The floor

Every flat pipeline fits `wgpu::Limits::default()`, WebGPU's guaranteed limits, with no optional feature and no downlevel flag beyond core WebGPU. A phone GPU (Vulkan or Metal) can therefore run it at a lower render size.

| Need | Default | Flat pass |
|---|---|---|
| bind groups | 4 | 2 (the layer composite) |
| sampled textures per stage | 16 | 4 (icons, environment, gradient stops, sprite atlas); 1 for text and layers |
| samplers per stage | 16 | 3 |
| uniform buffers per stage | 12 | 1 |
| dynamic uniform buffers per layout | 8 | 1 (text) |
| storage buffers or textures | 8, 4 | 0 |
| vertex attributes | 16 | 15 |
| vertex stride | 2048 | 240 |
| inter-stage components | 60 | 60 |
| formats | — | Rgba16Float (render, blend), R32Uint (render), R16Float (filtered), Rgba32Float (loaded), Rgba8Unorm (filtered sprite atlas) |

Two tests hold this:
- `every_flat_layout_fits_webgpu_defaults` checks the layouts with `layout_shortfall`, in the gate.
- `the_flat_pipelines_run_on_a_device_at_webgpu_defaults` opens a real device with exactly `Limits::default()` and no features. It builds every pipeline, the lit ones included, and draws a scene through every finish variant under a validation error scope. The scene has icons, a group, a lit slab and an environment.

## Costs

GPU time per pass on this machine (RX 9060 XT, RADV), medians of 80 frames after a warm-up round. The spike scene has:
- 400 static and slow shapes;
- 40 stacks of five;
- 100 pointers, each with its ghost;
- 500 ticks in flight with trails;
- 80 labels re-laid out every frame through `TextEngine::render_spans`;
- 1,264 glyphs in all.

That is 1,881 draws and 1,938 instances.

| Size | Variant | flat | flat ids | total |
|---|---|---|---|---|
| 3840×2160 | dials at zero | 1.16 ms | 0.35 ms | **1.51 ms** (budget 4) |
| 3840×2160 | thickness 6, bevel 2 | 1.39 ms | 0.43 ms | 1.82 ms |
| 3840×2160 | gloss 0.4, roughness 0.3 | 1.32 ms | 0.40 ms | 1.71 ms |
| 3840×2160 | reflection 0.5, roughness 0.3 | 1.32 ms | 0.40 ms | 1.71 ms |
| 3840×2160 | 40 patterned cards | 1.18 ms | 0.35 ms | 1.53 ms |
| 1280×800 | dials at zero | 0.16 ms | 0.05 ms | **0.21 ms** |
| 1280×800 | thickness 6, bevel 2 | 0.19 ms | 0.06 ms | 0.26 ms |
| 1280×800 | gloss 0.4, roughness 0.3 | 0.18 ms | 0.06 ms | 0.24 ms |
| 1280×800 | reflection 0.5, roughness 0.3 | 0.18 ms | 0.06 ms | 0.24 ms |
| 1280×800 | 40 patterned cards | 0.16 ms | 0.05 ms | 0.21 ms |

Each dial's cost, applied to every shape in the scene, over dials at zero:

| Dial | at 4K | at 1280×800 |
|---|---|---|
| thickness and bevel | +0.32 ms | +0.05 ms |
| gloss | +0.21 ms | +0.03 ms |
| reflection | +0.21 ms | +0.03 ms |
| 40 patterned cards | +0.02 ms | +0.003 ms |

**A Deck-class GPU, estimated.** Take integrated RDNA 2 with 8 CUs at 1.6 GHz (1.6 TFLOPS, 88 GB/s) at 1280×800, against frame targets of 60 and 90 fps: 16.7 and 11.1 ms. The RX 9060 XT is 32 RDNA 4 CUs at 3.13 GHz (12.8 TFLOPS single-issue, 25.6 dual-issue) with 320 GB/s. The flat pass is fragment-ALU bound (shadows and fields), so the ratio is 8× single-issue, or 16× if RDNA 4's dual issue were fully used. Bandwidth alone is 3.6×.

The Deck spike sets real text sizes at a 140% text scale in DM Sans, the engine's own OFL test font:
- 40 numbers at 28 px and weight 800;
- 40 changing labels across sizes from 13 to 38 px;
- 14 lines of body at 19 to 24 px;
- 1,354 glyphs in all, over the spike scene's 1,894 draws.

| 1280×800, text at 1.4 | here | at 8× to 16× | 60 fps (16.7 ms) | 90 fps (11.1 ms) |
|---|---|---|---|---|
| dials at zero | 0.22 ms | 1.7 to 3.5 ms | fits | fits |
| thickness 2, bevel 1, gloss 0.15, reflection 0.1, roughness 0.45, icons included | 0.29 ms | 2.4 to 4.7 ms | fits | fits |

A real device check comes later.

**Text size against a 9-pixel rule.** Valve's Deck check asks that the smallest character be at least 9 pixels tall at 1280×800. `text_roles_measure_against_the_deck_9_pixel_rule` renders DM Sans "H" and "x" at weight 400 through the flat pass at 1280×800 (layout scale 0.667) and counts the rows covered at least 30%:

| Size at 1080 | text scale 1.0: cap / x-height | text scale 1.4: cap / x-height |
|---|---|---|
| 13 px | 6 / 4 px | 9 / 7 px |
| 14 px | 7 / 5 px | 9 / 7 px |
| 16 px | 8 / 5 px | 11 / 8 px |
| 19 px | 9 / 6 px | **13 / 9 px**, passes |

At 140%:
- 19 px passes with a 9 px x-height;
- 13, 14 and 16 px stay under 9 px in lowercase;
- 13 px reaches a 9 px x-height only at a text scale of 1.85, and a 9 px cap at 1.35.

A game's own typeface moves these numbers; the test measures any font it is given.

## Against a reference renderer

`crates/live/tests/flat/scenes.py` draws the engine's own neutral test scenes as SVG and screenshots each in headless Chromium at 1× and 2×. The scenes are generic cards, pills, rings, arcs, dashed outlines, gradients, icons, patterns, groups and shadows at two elevations, in their own colours and layout, plus a full 1920×1080 board. `FLAT_CHROMIUM` can name the browser binary.

`flat::svg::read` reads that SVG subset into draws, so each test scene is built from the same numbers as its reference:
- elements: `rect`, `circle`, `path` (M, L, H, V, A and their relative forms), `use`, `g`, two-stop vertical gradients, dashes;
- `translate`, `rotate` and `scale`;
- element and group opacity;
- filters, mapped by the caller to an elevation or a glow; the tests map `lift1`, `lift2` and `halo`.

Mean and worst difference per channel, GPU against Chromium's PNG:

| Scene | What it holds | 1× mean / worst | 2× mean / worst |
|---|---|---|---|
| card | two stacks of tiles with their shadows, one tilted | 0.37 / 70 | 0.30 / 97 |
| faded | tiles in groups at 0.5 opacity with shadows | 0.41 / 63 | 0.33 / 59 |
| sheet | a raised sheet's shadow over a scrim, and a badge | 0.29 / 63 | 0.27 / 46 |
| panel | outlined boxes, solid strokes, bars | 0.02 / 36 | 0.01 / 59 |
| pill | gradient chip with stroke, gradient pill, columns, fade | 0.32 / 46 | 0.29 / 45 |
| dashed | dashed outlines on light and dark | 0.13 / 20 | 0.12 / 41 |
| arc | two meters with round caps | 0.21 / 83 | 0.14 / 131 |
| glow | a halo and a shadowed disc | 0.25 / 29 | 0.24 / 36 |
| icons | every icon above | 0.10 / 52 | 0.06 / 52 |
| pointer | the pointer and its ghost at 1.6× | 0.05 / 56 | 0.02 / 67 |
| scrim | a dark scrim at 0.6 and white at 0.7 | 0.15 / 40 | 0.14 / 55 |
| board | the full neutral board | 0.47 / 82 | 0.42 / 131 |
| patterns | four patterns and a patterned circle, against explicit clipped paths | 0.59 / 51 | 0.27 / 71 |

Every mean is under the 1/255 target. The worst pixels are single edge pixels, a few in a thousand at most, for three reasons:
- Chromium flattens curves into polygons, so a large arc's edge drifts by part of a pixel.
- Skia's area coverage differs from a one-pixel ramp on diagonals.
- Skia approximates the Gaussian with three box blurs.

The CPU twin (`flat::cpu`) runs the same instances, fields, shadows, order and layers on the CPU. In the gate, without a GPU, it matches the references to the same means. It matches the GPU to within 1 or 2/255, which is f16 blending. It draws text too (next section).

## Text on the CPU twin

`flat::cpu::render` and `pick` draw no text, as before. `render_with_text(scene, icons, texts, size)` and `pick_with_text(scene, icons, texts, size, pixel)` draw it, so a game's gate can check its labels without a GPU:

```rust
let texts = [CpuText::paragraph(&paragraph, colour)];
let image = flat::cpu::render_with_text(&scene, None, &texts, [3840, 2160])?;
let id = flat::cpu::pick_with_text(&scene, None, &texts, [3840, 2160], [x, y])?;
```

- `CpuText { atlas, glyphs, colour }` is a `FlatText` whose atlas is the text crate's own `Atlas` (the bytes a `GpuAtlas` is made from), so no device is needed. `texts[i]` goes with `scene.text[i]`: a `Shape::Text(i)` draw reads `texts[i]`. On a machine with no GPU `scene.text` is `&[]`, since a `GpuAtlas` cannot be made. A draw that names a text `texts` lacks is an error, as in the GPU pass; plain `render` and `pick` still skip text.
- `Glyphs::Paragraph` (`CpuText::paragraph`) and `Glyphs::Quads` (`rich_quads` of a `RichParagraph`, with its `atlas`) both work. Coverage atlases (one channel) and MSDF atlases (three) are drawn; the colour-glyph atlas (`Rgba16Float`, emoji) is not.
- **The GPU's coverage rule, ported.** Per glyph quad the twin finds each pixel centre's place in the quad and its UV, and takes the UV's screen derivatives from the quad's affine map:
  - a coverage atlas samples trilinearly at `log2` of the footprint in texels, clamped to 0 and the smaller of 3 and the cell's own safe level, with the UV clamped half a texel inside the cell;
  - an MSDF atlas reads the level-0 field bilinearly, takes the median of the three channels and turns the signed distance into coverage with `fwidth`, which the GPU computes per 2×2 quad (coarse), so the twin differences the quad's top-left pixel with its right and lower neighbours;
  - above an anisotropy of 1.2 it blends to the same up-to-8-tap filter along the major axis that the shader uses (see `docs/text.md`, grazing views);
  - the glyph's clip rectangle, the draw's opacity (up to 1), the span's colour and the `FlatText` tint, premultiplied and blended over, as the text pass does.
- A quad covers the pixels whose centres fall in it, with its corners snapped to the rasteriser's 1/256 pixel grid, so a label on a half-pixel edge lands as it does on the GPU.
- `pick_with_text` returns a text draw's `id` where its coverage reaches 0.5 and the glyph's alpha is above 0, and a draw with no id is skipped, as the id pass does.
- **Tolerance, against the GPU** (`the_twin_draws_labels_like_the_gpu_within_its_stated_tolerance`, `--ignored`, by name under pgpu): a label of 19 characters, MSDF and coverage, at 1×, 2×, 0.4×, 0.5×, squashed and turned, and skewed, compared as 8-bit pixels:

| Case | Mean | Worst | Ink, twin over GPU |
|---|---|---|---|
| MSDF 1× / 2× / 0.4× | 0.0036 / 0.0032 / 0.0007 | 2 / 2 / 2 | 1.0000 / 1.0000 / 0.9997 |
| MSDF squashed and turned, skewed | 0.0017, 0.0034 | 2, 1 | 1.0000, 0.9999 |
| coverage 1× / 2× / 0.5× | 0.0020 / 0.0053 / 0.0007 | 1 / 1 / 1 | 1.0000 / 1.0000 / 0.9998 |

  The test fails above a mean of 0.02/255, a worst pixel of 4/255, 0.5 % of ink, or 0.5 % of the label's pixels picking a different id. The remaining difference is f16 blending and the hardware's filter weights.
- Against the outlines: `msdf_and_coverage_labels_agree_on_the_cpu` draws one string both ways on the CPU alone and requires the same ink within 8 % and a mean difference under 3/255, so a gate without a GPU still has an independent check on the field.

## Not covered

- Text in the board comparison: the references carry no text. Glyphs are the existing text shader's, unchanged. The CPU twin does not draw colour-glyph atlases (emoji), and a text draw's lit and deformed passes are not twinned.
- A real measurement on a Deck-class device.
- Strokes dash only on rounded rects and circles, and on rings and arcs under an outline style; icons and text cast no shadow.
