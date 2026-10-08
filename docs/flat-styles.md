# Flat-pass viewports, shape styles and the character grid

Three building blocks on top of the flat pass (`docs/flat.md`), for a game that restyles a run: an inner viewport that lays the board out inside a frame, shape styles that redraw shapes as outlines, ghosts, bevels or flat, and a character grid that turns the board into a monospace text mode. The engine holds no look of its own: every size, colour, font and timing is the game's, passed as data. Nothing changes until a game sets one. A frame with no style set is byte-identical to one drawn before they existed.

They compose with the look stack's passes (low resolution, quantising, CRT), which run on the flat pass's output.

## Inner viewports

`flat::viewport` places a board inside a frame within the window's layout.

**The frame.** `Frame { size, bezel, radius, colour }`, in the window's layout units:
- `size` is the frame's outer size. Its shorter side is the width in portrait and the height in landscape, so either order works.
- The screen inside is the outer size less `bezel` on every side. Its corner radius is `radius − bezel`, or 0.
- `colour` paints the bezel.

**The viewport.** `Viewport { centre, frame, orientation, turn, mirror }`, built with `Viewport::new(centre, frame)` and the builders `orientation` and `mirror`. `viewport.at(now)` returns a `Placement` for that game time:

| Field | Meaning |
|---|---|
| `layout` | The inner layout the game lays out in now: the screen's size in the current orientation. |
| `orientation` | `Portrait` or `Landscape`, the one `layout` belongs to. |
| `angle` | How far the frame has turned, in radians, clockwise positive. |
| `progress` | The turn's progress, 0 to 1; 1 when settled. |

Everything is a function of the game time the caller passes, so a replay or a pfx shot reproduces it.

**Turning.** `viewport.turn_to(to, now, duration, ease, clockwise, reduced_motion)` starts a turn:
- During it the frame and the board turn together by up to a quarter turn, with the easing (`pfx_core::ease::Ease`) the game picks. The board keeps its old layout.
- At the end the layout re-flows: `Placement::layout` swaps sides, the angle is 0, and the game lays its board out again for the new orientation. The turned frame lands exactly on the new frame.
- With `reduced_motion` the turn is instant: the next placement is already settled in the new orientation.
- A turn to the orientation already laid out does nothing. A new turn mid-turn starts from the layout of that moment.
- `viewport.settle(now)` folds a finished turn into `orientation`.

**Placing draws.** `placement.place(&draw)`, `place_all(&mut draws)` and `transform(matrix)` map a draw from the inner layout to the window. A mirror moves positions but keeps each shape and label upright: a label stays readable, a gradient keeps its direction and a meter still fills clockwise. `reflected(matrix)` is the plain reflection, for a game that wants mirrored glyphs. A label is placed by its own anchor, which the mirror moves, so a label anchored at its centre stays centred in its card.

**The pointer.** The pointer map is the inverse of the drawing:
- `pfx_gpu::window::Viewport::pointer_to_layout`, built with `viewport_with(…, LetterboxRounding::Nearest)`, takes a window pixel to the window's layout through the same letterbox as `flat::Fit`.
- `placement.pointer(point)` takes that to the inner layout, or `None` outside the screen's rounded rect.
- `to_window` and `to_layout` convert without the screen test.

`the_pointer_map_matches_the_drawing_through_a_turn` checks it on the CPU twin at 1280×800 and 1920×1080, at eight instants through a turn, the re-flow included. Each of seven marks picks its own id where the map says it is drawn, the map takes its pixel back to within a pixel of its centre, and every 5th or 7th pixel's pick agrees with the map (a pixel and a half from any edge).

**Mirrors and gravity.** `Mirror::Vertical` turns the board top to bottom, `Mirror::Horizontal` left to right. `placement.gravity()` is the inner-layout direction that points down on the screen:
- `[0, 1]` unmirrored or mirrored left to right;
- `[0, −1]` mirrored top to bottom;
- turning with the frame during a turn.

A tumble that falls along `gravity()` falls down the screen. A game that wants things to fall up in an upside-down world negates it.

**Drawing the frame.** `placement.frame_draws(elevation, backdrop, reach)` returns the frame as flat draws, at the elevation the game gives it, above the board:
- the bezel, a rounded-rect stroke from the screen's edge out to the frame's;
- with `Some(backdrop)`, a mat in that colour from just inside the bezel's outer edge out to `reach` layout units. It covers anything that spills out of the screen, such as a card tumbling off the board, and its own edge sits under the bezel so no seam shows.

The frame clips by covering. With a backdrop that is not one colour (a gradient sky), pass `None` and keep the board inside the screen. `placement.screen_bounds()` gives the screen's window rect. Draws outside the frame go above its elevation. The frame's draws have id 0, so a pick on the frame finds nothing beneath it.

## Shape styles

`flat::style` redraws a shape in another style. `Draw::style(style)` sets it on one draw; `style::restyle(&mut draws, style)` and `style::restyle_group(&mut draws, group, style)` set it on a whole layer, and `None` clears it.

| `Style` | What it draws |
|---|---|
| `Outline(Outline { width, dash, colour, alpha })` | The shape's edge as an SDF stroke instead of its fill. `Style::outline(width)`, `Outline::dashed(width, on, off)`. |
| `Style::ghost(width, on, off, alpha)` | A faint dashed outline, for previews: an `Outline` with a dash and an alpha. |
| `Bevel(Bevel { width, light, dark, toward })` | Hard light and dark edges inside the shape instead of the soft shadow. `Style::bevel(width, light, dark)`. |
| `Flat` | The shape with no elevation shadow and no dials. |

Every style drops the cast and turned shadow and zeroes the material dials. A glow (`Shadow::Glow`) stays, so a neon outline can still glow. Elevation still sets painter's order. Text draws ignore the style.

**Outline.**
- The stroke is centred on the shape's edge, like every flat stroke.
- `width` and the dash lengths are layout units, whatever the shape's own scale, so every outline in a layer has the same weight.
- The colour is `colour` if set, else the draw's stroke colour, else its fill colour (a gradient's top stop), times `alpha`.
- Patterns are dropped.
- Dashes run along rounded rects and circles as before, and along rings and arcs too. A ring's or an arc's dashes follow the angle at its radius, so both edges and the caps dash together.
- A line icon's lines take the outline's width; icons do not dash.

**Bevel.**
- The band is `width` layout units inside the edge, rounded to whole target pixels and at least one. 2 is 2 pixels at 1920×1080, 4 at 4K and 1 at 1280×800.
- The sides facing `toward`, a layout direction toward the light (by default up and left), take `light`; the others take `dark`. The light is fixed to the layout, so a turned card keeps its lit side up and left.
- Where a lit side meets a dark one, the colours split along the corner's mitre, or across a rounded corner where its normal turns away from the light. The split is anti-aliased like every edge.
- A ring is raised: its outer edge is lit toward the light and its inner edge is dark there. Rects, circles, rings and arcs bevel; icons draw flat.
- The band paints over the fill and any pattern, and under the stroke.

`bevel_edges_are_whole_pixels_against_analytic_edges` and `outline_strokes_cover_like_analytic_edges` compare the CPU twin with the exact pixel overlap of the band, at 1× and 2× and on fractional edges, within 2e-4. `bevels_light_the_sides_facing_the_light_and_split_at_the_far_corners` checks the sides and corners. On the GPU (`the_gpu_draws_styles_like_its_twin`), a sheet of every style matches its twin to a mean of 0.065/255 (worst 0.6/255) at 1× and 2×, with every sampled id the same.

**Byte-identical when unset.** An unstyled draw packs through the unchanged code (`pack_plain`), into the same bytes. In the shader the style code is one call guarded by its flag. `unstyled_frames_are_byte_identical_on_the_gpu` renders every reference scene, flat and lit, through the shader and through `shader::unstyled_source()`, the same shader with the style code taken out: no pixel and no id differs.

**In the instance.** A bevel sets flag bit 24 (`style::BEVEL`) and uses slots only lit instances read: `material` holds the band's width in local units and the light direction in the shape's plane, and `frame[0]` and `frame[1]` hold the light and dark colours. A bevelled shape is never lit, so nothing collides. A motion-blurred draw (`Draw::blur`) keeps its blur shift in `material`, so a bevelled draw that is also blurred draws without its band while it moves.

## The character grid

`flat::grid` turns a board into text on a monospace cell grid in the caller's font. The engine's tests use its own IBM Plex Mono (OFL).

**The grid.** `Grid { origin, cell, columns, rows }`:
- `Grid::measure(&mut engine, &face)` gives the cell of a monospace face: its advance and its line height.
- `Grid::cover(layout, cell)` fits as many whole cells as the layout holds, centred.
- `cell_at`, `snap` and `corner` convert between layout points and cells.

**Mapping a board.** `CharGrid::new(grid, rules)`, then `map(&draws, groups, &labels)` maps a whole board in painter's order. `shape(&draw)` maps one shape, `text(at, text, ink, id)` writes a string and `put` one character. By rule:
- A shape covers the cells whose centres fall inside its bounds (a turned shape's axis-aligned bounds).
- Two or more cells each way make a box: `┌─┐│└┘`. A rect whose corner radius is at least `rules.rounded`, and every circle, ring and arc, gets rounded corners, `╭╮╰╯`, in the light weight.
- One row makes a text line, `─`, from the first cell's centre to the last; one column makes `│`. Its ends are half lines, `╶╴`.
- Smaller than that, a shape is `rules.dot`.
- Outlines and lines merge where they meet, into `├ ┤ ┬ ┴ ┼`.
- A filled box (fill alpha times opacity at least 0.5) covers what is beneath it, as an opaque card does: its edges replace earlier lines, and its interior is cleared to its ground. Rings and arcs never fill.
- A text draw writes `labels[index]`, a string and its colour, from its transform's translation, snapped to the nearest cell corner. Text snaps to cells, one character to a cell.
- There are no icons and no elevation: icons are dropped, and every cell is flat.

`Rules`, all data:

| Field | Meaning | Default |
|---|---|---|
| `line` | `Weight::Light`, `Heavy` or `Double` for every line | light |
| `raised` | `Some((elevation, weight))`: shapes at or above that elevation use that weight | none |
| `rounded` | the corner radius, in layout units, from which a rect's box has rounded corners | never |
| `interior` | a filled box's inside: `Background` (its ground colour), `Shade` (`░▒▓█` by fill alpha) or `Blank` | background |
| `dot` | the character for a shape smaller than two cells | `•` |
| `ink` | one colour for every line and character, such as a phosphor; or each shape's own | each shape's |
| `ground` | one background for every filled cell; or each shape's fill | each shape's |
| `thickness` | a light line's thickness, as a share of the cell width | 0.12 |

`lines()` and `char_at` give the grid as text in the box-drawing block (U+2500). `shapes_map_to_box_drawing_on_a_known_layout` checks a known board in the light, heavy, double and rounded rules, crossing outlines and a filled box covering another.

**Drawing it.** `chars.draws(Some(text_index), &mut draws)` emits flat draws, all at elevation 0 with no shadow or dials:
- grounds, merged into one rect per run of rows with the same colour and id;
- shade cells, in the ink at 25, 50, 75 or 100%;
- lines as geometry through the cell centres, merged into one rect per run, so they join without seams and need no box-drawing glyphs in the font. Light is `thickness`, heavy twice it, and double two light lines with a light line's gap, mitred at corners. Rounded corners are quarter arcs of half the smaller cell side;
- one text draw for the characters, at `text_index`.

A draw carries the id of the shape that last wrote its cells, so picking still works.

**Text.** `chars.render(&mut engine, &face, representation)` lays out every row in one `render_spans` call (one atlas, one span per run of one ink). It moves each glyph's pen to its cell's corner and its baseline to its row, centred in the cell when the cell is taller than the face's line. Upload `rich.atlas` as a `GpuAtlas` and pass the quads as `FlatText { glyphs: Glyphs::Quads(&quads), colour: Srgba::WHITE }` at `text_index`. Use MSDF, so the glyphs stay sharp at any render size. `text_snaps_to_cells_in_the_callers_mono_font` checks every pen on a cell edge and every baseline in its row.

**A terminal is data.** The grid's output is an ordinary flat scene, so the look stack's CRT passes (scanlines, phosphor glow, curvature) apply after it. A green-phosphor terminal is a dark `ground`, a green `ink`, a mono face and the CRT's parameters, all the game's.

**At Deck size.** IBM Plex Mono at 1280×800, the 1920×1080 layout scaled by 2/3, measured by `the_grid_font_holds_the_deck_floor` (rows covered at least 30%):

| Size at 1080 | Cell | Grid in 1920×1080 | x-height | cap |
|---|---|---|---|---|
| 16 | 9.6×24 | 199×45 | 6 px | 8 px |
| 20 | 12×30 | 160×36 | 7 px | 10 px |
| 24 | 14.4×36 | 133×30 | 8 px | 11 px |
| 26 | 15.6×39 | 123×27 | **10 px** | 13 px |

A grid in this font holds Valve's 9-pixel rule in lowercase from size 26, and in capitals from 20. A game's own face moves these numbers.

## Costs

GPU time for the flat and flat ids passes on this machine (RX 9060 XT, RADV), medians of 80 frames after a warm-up, by `styles_viewport_and_grid_cost`. The scene is the flat pass's spike scene without its labels: 1,800 draws.

| Variant | 3840×2160 | 1280×800 |
|---|---|---|
| no style (soft shadows) | 1.46 ms | 0.21 ms |
| every shape outlined | 0.80 ms | 0.12 ms |
| every shape a ghost | 1.37 ms | 0.20 ms |
| every shape bevelled | 1.07 ms | 0.15 ms |
| every shape flat | 0.87 ms | 0.13 ms |
| in a frame, turned 45°, with the mat | 1.81 ms | 0.26 ms |
| mirrored top to bottom | 1.51 ms | 0.21 ms |
| character grid, 160×36 cells, 1,495 glyphs | 0.48 ms | 0.074 ms |

Every style costs less than the soft shadows it replaces. A ghost's dashes cost the most of them, about as much as the shadows. The frame's mat covers the window outside the screen, which adds about a third of a millisecond at 4K. A mirror costs nothing: it only changes the transforms. A full terminal grid costs a third of the shaded board. Each is far inside the flat pass's 4 ms budget at 4K, and at 1280×800 under 0.3 ms, a few milliseconds on a Deck-class GPU by the flat pass's 8× to 16× estimate.

## Not covered

- The viewport clips by covering with a mat. A true clip mask needs a clip in the flat pass itself.
- Bevels on icons, and on slabs lit by the dials (a bevel draws its shape unlit).
- Dashed outlines on icons.
- In the grid, a wide (CJK) character still takes one cell. Mixed-weight junctions take the heavier weight, and a double tee's inner line is not broken.
