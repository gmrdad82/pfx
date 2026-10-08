# Screens: modes, the device, aspects and scale

`pfx_gpu::screens` is what a game needs to look right on a Steam Deck, a 16:9 desktop and an ultrawide: the window mode, which device it runs on, which aspects it supports, where the safe area is, how big its UI should be, and how far to drop the render scale to hold a GPU budget. Everything in it is a pure function or a small value. It opens no window, reads no clock and writes no file. `pfx_live::upscale` is the GPU half: it draws a frame rendered at a lower size into the window, crisply, with the bars and a flat HUD at native resolution.

The window layer underneath (`docs/window.md`) is unchanged in what it does. This module chooses its inputs.

## Window modes

`WindowMode` is `Windowed`, `Borderless` or `Fullscreen`. Borderless is the default.

| Mode | Window layer | What the player gets |
|---|---|---|
| `Windowed` | `DisplayMode::Windowed` at the stored size | decorations on, resizable |
| `Borderless` | `DisplayMode::BorderlessFullscreen` on the stored monitor | the desktop's own resolution, no mode change |
| `Fullscreen` | `DisplayMode::ExclusiveFullscreen` with `VideoModeChoice::Native` | exclusive where the platform allows it (Windows, macOS, X11), else borderless with `Fallback::ExclusiveUnsupported` |

`VideoModeChoice::Native` is the monitor's own size at its current refresh (within 1 Hz), the deepest bit depth first; with no known refresh it takes the highest. `ModePlan::resizable` is true for a decorated window, and `apply_plan` calls `WindowOps::set_resizable` before the size. `set_resizable` has a default that does nothing, so an existing `WindowOps` keeps compiling.

`Display` holds the device and a `DisplaySettings`:

```rust
let settings: DisplaySettings = game_store.load().unwrap_or_default();
let mut display = Display::new(device, settings);
apply_display(&window, &display)?;
match display.set_mode(WindowMode::Windowed) {
    Change::Switched { .. } => { apply_display(&window, &display)?; }
    Change::Same => {}
    Change::FixedOnDeck => show("The Deck always runs full screen"),
}
game_store.save(display.settings());
```

- `DisplaySettings { mode, size, monitor, position }` is serde, so the game keeps it wherever it keeps its settings (a TOML file, Steam Cloud, its save) and hands it back at the next start. The engine writes no file. `size` is the windowed size (1600×900 when missing), `monitor` is the monitor's name, which survives a change in monitor order better than an index, and `position` is the windowed window's top-left relative to that monitor's top-left (none when missing, which leaves the placement to the desktop).
- `resized(size)` remembers a windowed size the player dragged to; it ignores sizes in the other modes. `moved_to(name)` remembers the monitor. `placed(monitor, position)` remembers where a windowed window was moved to, on which monitor, relative to it (`pfx_gpu::window::placement(&window)` reads both from winit); it ignores moves in the other modes.
- **The position comes back on its monitor.** `display_mode_on(&monitors)` is the mode with the stored position resolved against the monitors there are now: on the named monitor when one of that name still exists (or the current monitor when no name was stored), offset by the monitor's own position and clamped so the window stays on it, and no position at all when the monitor is gone, so the desktop places the window rather than off screen. `windowed_at(&monitors)` is that point alone. `plan`, `apply`, `switch` and `apply_display` all use it; `display_mode()` stays the mode without a position. On Wayland a window has no global position: winit reports no move and ignores the request, so nothing is stored or restored there.
- `switch(mode, platform, monitors, window)` sets the mode and plays the plan on any `WindowOps`, so a test drives it with a stand-in window. A plan that fails puts the old mode back.
- `apply_display(&winit_window, &display)` (the `window` feature) does the same through winit.

**On a Steam Deck** the window is always borderless fullscreen on the current monitor, at the panel's native size. `mode()` says `Fullscreen`, every `set_mode` and `switch` returns `Change::FixedOnDeck` and changes nothing, and `resized` and `moved_to` leave the stored settings alone, so a settings file carried from a desktop comes back unchanged.

## The device

`detect(&SystemSource)` returns a `Detected { device, evidence }`. `Device` is `SteamDeck { model: Lcd | Oled }` or `Desktop`. The checks, in order:

0. `PFX_DEVICE=deck-lcd|deck-oled|desktop` (the full labels `steam-deck-lcd` and `steam-deck-oled` work too) overrides everything below, so a desktop player inside gamescope can opt out of Deck treatment, and a desktop can try the Deck's layout. Any other value is ignored and the checks go on.
1. `SteamDeck=1` (Steam sets it): a Deck. The model comes from the product name when it names one, else LCD.
2. The DMI product name, `/sys/class/dmi/id/product_name`: `Jupiter` is the LCD, `Galileo` the OLED.
3. A gamescope session: `XDG_CURRENT_DESKTOP=gamescope` (SteamOS game mode) or a non-empty `GAMESCOPE_WAYLAND_DISPLAY` (a process gamescope started): a Deck LCD. A desktop game run inside gamescope is treated as a Deck, which is what a Deck-sized gamescope run wants.
4. Otherwise `Desktop`.

`evidence` says which rule matched (`Override`, `SteamDeckVar`, `ProductName`, `Gamescope`, `Nothing`). `DeviceSource` is the trait behind it (`var`, `product_name`), so tests and tools inject any answer. `Device::label()` is `steam-deck-lcd`, `steam-deck-oled` or `desktop`, and `Device::from_label` reads it back. Frame stats writes it in every line's `device` field: the renderer's `FrameStats` detects the device once, when it opens its file (`docs/frame-stats.md`).

## The aspect policy

A game declares the aspects it supports per device as content, in a `[screen]` table. Keep it in the game's own settings or content file (for example `game.toml` beside its scenes) and pass the text to `ScreenPolicy::from_toml`; other keys in that file are ignored. `ScreenPolicy::from_table` takes an already parsed table. A file without `[screen]` gives the defaults.

```toml
[screen]
deck = ["16:10"]
desktop = ["16:9", "21:9"]
tolerance = 0.03
bars = "#000000"
layout_height = 1080

[screen.safe]
deck = 0.03
desktop = { left = 0.0, right = 0.0, top = 0.0, bottom = 0.0 }

[screen.ui_scale]
deck = 1.4
desktop = 1.0
```

| Key | Meaning | Default | Range |
|---|---|---|---|
| `deck`, `desktop` | supported aspects, `"w:h"` | `["16:10"]`, `["16:9"]` | 1:4 to 8:1; an empty list takes any window natively |
| `tolerance` | how far a window's aspect may be from a supported one and still run native, as a fraction of the ratio | 0.03 | 0 to 0.25 |
| `bars` | the bar colour, sRGB, `#rrggbb` or `#rrggbbaa`, or `"backdrop"`: the game draws its own background in the bars (below), black while it draws nothing | `#000000` | |
| `layout_height` | the layout's height in units; the width follows the aspect | 1080 | 240 to 8640 |
| `layout_width` | the layout's width in units instead; the height follows the aspect, so `layout_width = 1920` gives 1920×1200 at 16:10 and 1920×1080 at 16:9. Set it or `layout_height`, not both | none: the height anchors | 240 to 15360 |
| `safe.deck`, `safe.desktop` | the UI inset, one fraction for all sides or `{ left, right, top, bottom }`, of the content's shorter side | 0.03, 0 | 0 to 0.25 per side |
| `ui_scale.deck`, `ui_scale.desktop` | the UI scale below | 1.4, 1.0 | 0.25 to 4 |

An unknown key or a value out of range is refused with a `PolicyError` that names it, for example `[screen] tolerance: must be from 0 to 0.25`.

### The fit

`policy.fit(device, window)` returns a `ScreenReport`:

- **Native** when the window's aspect is within `tolerance` of a supported one. The layout takes the window's exact ratio at `layout_height` (2560×1080 is 2560×1080 units, 3440×1440 is 2580×1080), so it fills the window with no bar. `aspect` is the declared aspect it matched.
- **Otherwise the nearest supported aspect** (by the ratio of ratios), with bars. A window wider than that aspect is pillarboxed (bars left and right, an ultrawide on a 16:9 game); a taller one is letterboxed (bars top and bottom, a 16:10 screen on a 16:9 game). The layout is that aspect at `layout_height`.
- **Width-anchored.** With `layout_width` the same choice is made and the layout keeps that width instead, its height the width over the ratio, rounded (`policy.layout_size(ratio)`): a 1280×800 Deck is native at 1920×1200 units, a 1920×1080 window on the same Deck policy is pillarboxed with a 1920×1200 layout in a 1728×1080 content rect at x 96, and a 1920×1090 window within tolerance of 16:9 is native at 1920×1090. Everything downstream (the content rect, the letterbox, the pointer, the safe area, `UiFrame::layout`) follows the layout it gives, through the same one letterbox. Without `layout_width` every result above is unchanged.

| Window | `deck = ["16:10"]` on a Deck | `desktop = ["16:9"]` | `desktop = ["16:9", "21:9"]` |
|---|---|---|---|
| 1280×800 (16:10) | native, 1728×1080 units | letterbox, 1280×720 at y 40 | letterbox, 1280×720 at y 40 |
| 1920×1080 (16:9) | pillarbox, 1728 wide at x 96 | native | native |
| 3840×2160 (16:9) | pillarbox, 3456 wide at x 192 | native | native |
| 2560×1080 (21:9) | pillarbox, 1728 wide at x 416 | pillarbox, 1920 wide at x 320 | native 21:9, 2560×1080 units |
| 3440×1440 (21:9) | | pillarbox, 2560 wide at x 440 | native 21:9 (2.4% off), 2580×1080 units |
| 5120×1440 (32:9) | pillarbox, 2304 wide at x 1408 | pillarbox, 2560 wide at x 1280 | pillarbox at 21:9, 3360 wide at x 880 |

The report holds `device`, `window`, `aspect`, `native`, `bars` (`None`, `Pillarbox`, `Letterbox`), `bar_colour`, `backdrop` (the policy said `bars = "backdrop"`), `layout` (the game's units), `letterbox`, `content` (the drawn rect in window pixels), `safe` (in window pixels), `safe_layout` (in layout units) and `ui_scale`. Ask for it each frame, or on each resize, and lay the UI out from it.

**One letterbox.** `report.letterbox` is `letterbox_nearest(layout, window)`, the window layer's map with nearest rounding. The flat pass's `Fit::new(report.layout_units(), size)` and `report.viewport()` (`viewport_with(window, Native, layout, LetterboxRounding::Nearest)`) compute the very same map, so the board, the bars and the pointer agree to the pixel. `one_letterbox` in the tests checks all three for every window above.

**Pointer.** Pass `report.viewport()` to `Input::set_viewport`. The pointer lands in layout units, and in the bars `inside` is false (`report.in_bars(x, y)` says the same). The pointer always maps at native resolution, whatever the render scale. A mouse button pressed while the pointer is outside the frame (`inside` false) fires none of the actions bound to it until it is released: `Input::mouse_held` still says it is down and `Input::mouse_outside(button)` says it went down outside, so a game that wants clicks in the bars can read them. Keys, pads and the wheel are unchanged, a button pressed inside still releases outside, and with no pointer yet (or no viewport) a press fires as before. A recording keeps the held outside buttons in its start, so a replay holds them off too.

**Bars.** A flat scene's `clear` is the colour behind the board and the bars, so a flat-only game sets it to `bar_colour` and draws its own background rect over the layout if it wants a different one. A 3D frame drawn through `pfx_live::upscale` gets the bars from `Compose::bars`.

**Backdrop.** `bars = "backdrop"` (`ScreenPolicy::backdrop`) asks for the game's background to continue to the window's edges instead of bars. The fit is unchanged: the content rect, the layout, the safe area and the pointer map are the same as with coloured bars, so wide screens show no more of the game. `report.window_units()` is the whole window in layout units (`window / pixels_per_unit`) and `report.content_units()` is the content rect in them, so the backdrop is laid out at the layout's own scale. `Compose::backdrop` is a `FlatScene` laid out in `window_units()`: the upscale draws it first across the whole output through its own flat pass, cleared to the bar colour, then the framed view only inside the content rect (the pass leaves the rest); the HUD follows through `encode_overlay`. Without it the bars are cleared to `bars` as before. pfx-game wires this from `Ui::backdrop` (`docs/game.md`, "The backdrop").

## The safe area

The safe rect is the content rect inset on each side by its fraction of the content's shorter side. The Deck's default, 3%, is 24 px of its 800 (32 layout units of 1080), which keeps text off the bezel and out from under the thumbs. The desktop default is none. The game anchors its HUD to `safe_layout`.

## Scale

**UI scale.** `report.ui_scale` multiplies the game's UI sizes (text, icons, hit targets) in layout units. Layout units already follow the window, so at 1.0 a 1080-unit layout reads the same on a 1080p and a 4K desktop (one unit is 1 px and 2 px). The Deck shows the layout on a 7-inch panel at 800 px, held closer: one unit is 0.74 px there. The default of 1.4 comes from the flat pass's measurement against Valve's 9-pixel rule (`docs/flat.md`, Text size): at 1.4, 19 px text at 1080 has a 9 px x-height on the Deck. `units_for_pixels(9.0)` gives the layout size that reaches 9 pixels on the current screen (12.15 units on the Deck). `pixels_per_unit` is the fit's scale.

**Render scale.** `report.render_size(scale)` is the content rect times a factor: the size the live `Renderer` draws the 3D frame at. The renderer keeps its targets at the content size and draws into the top-left `size` of them (`set_viewport`, below), so a scale change costs nothing. The frame goes into `Upscale::source_target(device, content, format)`, a texture made once at the content size, and `Upscale::encode` reads its top-left `source_size` and draws it into the window:

```rust
let report = policy.fit(device, window_size).unwrap();
let content = report.render_size(1.0).unwrap();
if renderer.capacity() != (content.width, content.height) { renderer.resize(content.width, content.height)?; }
let scale = resolution.begin(renderer.frames_submitted());
let size = report.render_size(scale).unwrap();
renderer.set_viewport(size.width, size.height)?;
let target = upscale.source_target(&gpu.device, [content.width, content.height], format)?;
let submitted = renderer.submit(&scene, &text, &effects, finish, &target)?;
for timings in &submitted.arrived { resolution.observe_timings(timings); }
upscale.encode(&gpu.device, &gpu.queue, &mut encoder, &Compose {
    source: &target, source_format: renderer.output_format(), source_size: [size.width, size.height],
    output: &swapchain_view, output_format: format, output_size: [window_size.width, window_size.height],
    content: report.content, bars: report.bar_colour, filter: Filter::Sharp,
    backdrop: None,
}, None)?;
upscale.encode_overlay(&gpu.device, &gpu.queue, &mut encoder, &Overlay {
    scene: &hud, look: None, output: &swapchain_view, output_format: format,
    output_size: [window_size.width, window_size.height], content: report.content,
}, None)?;
```

`source_size` is the sub-rect the frame occupies at the top-left of `source`; the upscale clamps every tap to it, so a source larger than the frame upscales exactly as a source of the frame's own size would, with nothing copied (`a_sub_rect_of_a_larger_source_upscales_like_an_exact_source`).

- `Filter::Sharp` is a 4×4 Catmull-Rom clamped to the four nearest texels, so edges stay crisp and never ring. `Filter::Nearest` is for pixel art at whole factors. At a factor of 1 both copy the source exactly. `upscale::cpu` is the CPU reference the GPU matches.
- The HUD is a `FlatScene` laid out in `report.layout_units()`, drawn by `Upscale::encode_overlay` through the upscale's own flat pass at the window's size after the upscale, so text and UI stay at native resolution whatever the 3D scale. Its picking ids are not written; pick the 3D frame through the renderer, at its own size. It is clipped to the content rect (`upscale::clip`, a scissor on the flat finish, the pixels whose centres fall inside the rect), so no text or control laid out past the layout reaches the bars or the backdrop; only `Compose::backdrop` draws outside the frame. `Upscale::encode_overlay(device, queue, encoder, &Overlay { scene, look, output, output_format, output_size, content }, profiler)` draws it over whatever the output already holds, and with a `LookFrame` it draws the scene through the look (`docs/flat-looks.md`), clipped the same way; `Upscale::warm_overlay(device, queue, format, size, looks)` builds its pipelines and the looks' ahead of the first frame, and `overlay_pipelines_made()` counts them. pfx-game draws its UI this way after the game's own paint (`docs/game.md`, "Painting").
- The bars are cleared to `bars`, decoded for an sRGB output and premultiplied. With `Compose::backdrop` the backdrop takes their place and the bar colour is its fallback beneath it.

### The colour path

A frame is encoded to sRGB exactly once, and which target holds what decides where:

| Target | Holds |
|---|---|
| The renderer's `Rgba16Float` output (the upscale's source in a game) | encoded values: the finish chain ends with its own `Encode` pass |
| The renderer's `Rgba8UnormSrgb` or `Bgra8UnormSrgb` output | encoded bytes; the chain stops linear and the blit's hardware write encodes |
| The renderer's internal HDR targets and its display intermediate | linear values |
| An sRGB window surface or offscreen target | encoded bytes; a shader writes linear values and the hardware encodes them |

`Compose::source_format` tells the upscale what the source holds. `Transfer::between(source_format, output_format)` decodes an encoded source (a float or unorm format) before writing into an sRGB output, encodes a linear-reading source (an sRGB format) before writing into a unorm or float output, and copies otherwise. It runs after the filter, so `Filter::Sharp` still works on the values the source holds and scale 1 still copies it exactly. Before this, a game's `Rgba16Float` frame went straight into its sRGB surface and was encoded twice: a flat background at `#b1` in the renderer's own sRGB output and in the editor's viewport came out `#d9` in the game's window, the sRGB encode of `#b1` (`an_encoded_float_source_reaches_an_srgb_output_unchanged`; `crates/editor/tests/colour.rs`).

Measured on the RX 9060 XT, medians of 56 frames (`the_upscale_costs_little_at_4k_and_deck_size`):

| Source to output | Sharp | Nearest |
|---|---|---|
| 1920×1080 to 3840×2160 | 0.43 ms | 0.22 ms |
| 2880×1620 to 3840×2160 | 0.43 ms | 0.22 ms |
| 640×400 to 1280×800 | 0.05 ms | 0.03 ms |

**Dynamic resolution in the renderer.** One call turns it on, and one call a frame applies it:

```rust
renderer.set_dynamic_resolution(Some(Budget::new(12.0)))?;
loop {
    let (width, height) = renderer.apply_dynamic_resolution([content_width, content_height])?;
    let target = upscale.source_target(&gpu.device, [content_width, content_height], format)?;
    renderer.submit(&scene, &text, &effects, finish, &target)?;
    upscale.encode(/* … source_size: [width, height] … */)?;
}
```

The renderer feeds the controller itself: every full frame it submits is begun at the scale it actually rendered at (its viewport against the content size last given to `apply_dynamic_resolution`), and every GPU time its timestamps resolve is observed against the frame it measured, the same sum of passes and the same tick that frame stats writes as `gpu_ms` and `gpu_tick`. Partial, reprojected and idle frames are left out, so a cheap frame never raises the scale. `apply_dynamic_resolution(content)` resizes the renderer only when the content size (times the budget's `max_scale`) changed, a window resize, and otherwise sets its viewport to the scale's size, which costs nothing; it returns the size to render at. With dynamic resolution off it leaves the renderer alone and returns its size. `dynamic_resolution()` exposes the controller (its scale, its changes, its cost estimate), and `set_dynamic_resolution(None)` turns it off. `the_renderer_feeds_dynamic_resolution_from_its_own_gpu_times` (GPU) checks that a budget the neutral room can't meet drops the renderer to half size after the first timing, and a roomy one leaves it alone.

### Viewport dynamic resolution

The renderer allocates its targets once, at its capacity (`Renderer::new` or `resize`), and draws each frame into the top-left rect of them that `set_viewport(width, height)` names. `size()` is that rect and `capacity()` the allocation. A viewport change allocates nothing, compiles nothing and keeps the TAA history: the next frame simply draws a different rect.

- **Every screen-space pass takes the rect.** The prepass, the opaque pass, impostors, plates, background motion and the sky set the viewport and scissor to it; linear depth, SSR, TAA, the sharpen, depth of field and the post run their compute over it; glass, volumes, particles, creatures, surface and overlay text, the HUD, the ids and the output blit draw inside it. UVs, texel sizes and clamps come from the rect, never from a texture's size: the volume's quarter-size rect is the rect divided by 4 rounded up, SSR traces at half the rect, and the post chain's bloom levels are what a chain of the rect's size would allocate.
- **The scene colour mips** that SSR and glass sample are built over the rect: level `n` covers `rect >> n`, and each level gets a one-texel border copied from its edge, so a bilinear or trilinear tap at the rect's edge reads what `ClampToEdge` would read on a texture of the rect's size. Taps scale their UVs by the rect over the capacity.
- **TAA resamples its history.** The history remembers the rect it was written at. The resolve reprojects each pixel by its motion, as before, and reads the history with the bilinear and nearest taps at the history's own size, so a step from one scale to another blends the old frame into the new one instead of dropping it. SSR does the same with its half-size history. `history_restarts()` counts the times the history was dropped (a cut, a time gap, a resize); a viewport change is not one.
- **The output** is any texture at least as large as the rect; the renderer writes its top-left rect, which is what `Compose::source_size` reads. Overlay text (`TextSpace::Overlay`), a HUD drawn inside the frame and picks are in the rect's pixels, `size()`; a HUD meant to stay sharp goes through the upscale instead, at the window's size.
- **Limits.** Frames on demand and flat looks draw at the full size: `set_viewport` refuses a smaller rect while frames on demand are on, and `set_frames_on_demand` and a flat look refuse while the viewport is smaller than the capacity. The mip chain matches a renderer resized to the rect exactly when the rect and the capacity are multiples of 64 (the seventh level's factor); otherwise its coarse levels sit a fraction of a texel apart, which only the blur of rough reflections and frosted glass reads.

At scale 1 every byte is what the renderer drew before viewports existed: the neutral room (`crates/live/tests/support/viewport.rs`: a desk, a chrome ball, a glass slab, steam, dust, surface and overlay text) drawn 24 frames at 512×256, with and without depth of field and the sharpen, gives the same bytes on this tree and on the commit before it, and `a_full_viewport_draws_the_same_bytes_as_a_plain_renderer` keeps the dynamic-resolution path byte-identical to a plain renderer. At a fixed viewport the frame matches a renderer resized to the rect: after 32 frames, 384×192 inside 512×256 differs by at most 0.016/255 (mean 6e-7/255) with and without the finish, and 256×128 is identical (`a_fixed_viewport_matches_a_renderer_resized_to_it`).

**A step barely shows.** `a_scale_step_keeps_the_history_and_barely_shows` settles the room for 32 frames at one scale, steps, and compares the following frames with a renderer that rendered at the new scale all along, against the same step done with `resize`. Differences are in 8-bit steps of the encoded output, mean over the frame and worst pixel, with the share of pixels more than 12/255 apart ("visible"):

| Step | Frame after | Viewport: mean, worst, visible | Resize: mean, worst, visible |
|---|---:|---|---|
| 0.75 to 0.7 | +0 | 0.32, 130, 0.60% | 1.28, 187, 4.42% |
| 0.75 to 0.7 | +1 | 0.23, 107, 0.32% | 0.69, 125, 1.49% |
| 0.75 to 0.7 | +3 | 0.12, 85, 0.11% | 0.40, 100, 0.76% |
| 0.75 to 0.7 | +7 | 0.04, 29, 0.04% | 0.17, 40, 0.38% |
| 0.75 to 0.5 | +0 | 0.37, 179, 0.71% | 1.43, 193, 4.77% |
| 0.75 to 0.5 | +1 | 0.26, 109, 0.46% | 0.81, 125, 1.72% |
| 0.75 to 0.5 | +3 | 0.12, 46, 0.12% | 0.48, 104, 0.89% |
| 0.75 to 0.5 | +7 | 0.04, 20, 0.01% | 0.20, 59, 0.41% |
| 0.5 to 1 | +0 | 0.54, 163, 1.32% | 1.19, 199, 4.57% |
| 0.5 to 1 | +1 | 0.35, 116, 0.80% | 0.59, 148, 1.39% |
| 0.5 to 1 | +3 | 0.18, 76, 0.35% | 0.34, 79, 0.70% |
| 0.5 to 1 | +7 | 0.07, 46, 0.05% | 0.13, 36, 0.29% |
| 1 to 0.8 | +0 | 0.27, 203, 0.51% | 1.24, 187, 4.38% |
| 1 to 0.8 | +1 | 0.20, 114, 0.28% | 0.66, 128, 1.51% |
| 1 to 0.8 | +3 | 0.10, 87, 0.10% | 0.38, 89, 0.75% |
| 1 to 0.8 | +7 | 0.04, 60, 0.04% | 0.16, 54, 0.34% |

On the step's own frame the viewport path is a quarter to a half of the resize path's mean and a sixth to a third of its visible share, and its mean and visible share stay below the resize path's as both converge over the next 7 frames. The worst pixels are thin edges (text, glints, dust) whose history was resampled from the other scale; they settle over the next frames at the TAA blend's pace. With `resize` the history restarts, so the step's frame is the raw jittered frame.

**The controller.** `DynamicResolution::new(Budget::new(gpu_ms))` holds the render scale to a GPU budget. `begin(frame)` records the scale a frame renders at; `observe(frame, gpu_ms)` or `observe_timings(&FrameTimings)` feeds the time that frame took, whenever it arrives, and returns `Some(scale)` when the scale changes.

- The cost model is GPU time over scale squared, smoothed (a quarter of each new sample). The target scale is `sqrt(budget × headroom / cost)`, floored to the step grid between `min_scale` and `max_scale`.
- A frame over budget that was rendered at or below the current scale drops the scale at once, with its own cost. Otherwise the scale moves only after `settle_frames` frames at the current one, and rises only by at least a step, so it does not flicker between two sizes.
- A change only moves the renderer's viewport, so the defaults are tuned to respond fast without flapping: headroom 0.9, scales 0.5 to 1.0, step 0.05, 6 frames to settle.
- A settled scale is lowered only when its predicted time is over the budget itself, not over the budget times the headroom, and raised only when the headroom target is at least a step above it. Between the two the scale holds, so noise around a grid line does not flip it every few frames.
- `begin_at(frame, rendered_scale)` records the scale a frame really rendered at, so frames still in flight at the old size after a change are costed at their own size.
- GPU time comes from the renderer's own timestamps, a frame or two late and matched to the frame they measured. Inside the renderer that is automatic (above). A game driving its own controller reads them from `Submitted::arrived`, or from a frame stats file's `gpu_ms` against its `gpu_tick`.

In `dynamic_resolution_converges_under_the_budget_and_follows_the_load`, a synthetic GPU time of 2 + 12 × scale² ms with ±3% noise and a 10 ms budget runs for 700 frames, with a heavier stretch (3 + 18 × scale²) from frame 300 to 449 and timings arriving two frames late throughout. The trace of scale against frame, against the previous defaults (30 frames to settle, lowering whenever the headroom target is below the scale):

| Frames | Load | Now: scale (GPU ms) | Before: scale (GPU ms) |
|---|---|---|---|
| 0–2 | light | 1.0 (14.0), before the first timing | 1.0 (14.0) |
| 3–299 | light | 0.8 (9.68) | 0.8 (9.68), then 0.75 (8.75) from 33 |
| 300–302 | heavy | 0.8 (14.52), timings in flight | 0.75 (13.13) |
| 303–449 | heavy | 0.6 (9.48) | 0.6 (9.48), then 0.55 (8.45) from 333 |
| 450–474 | light | 0.6 (6.32), 0.65 (7.07) from 455, 0.7 (7.88) from 461 | 0.55 (5.63), 0.6 (6.32) from 454, 0.7 (7.88) from 484 |
| 475–699 | light | 0.75 (8.75) | 0.7 (7.88) to 516, then 0.75 (8.75) |

Both drop at once when a frame is over budget (frames 0–2 and 300–302 are the only ones over, while their timings are in flight). The new defaults change the scale 5 times against 7, hold 0.8 and 0.6 (inside the budget, so no further step is taken), and climb back in 25 frames against 67. Over 120 synthetic loads (base 1 to 4 ms, 8 to 20 ms per unit area, ±3% and ±10% noise, three seeds), the controller changed scale 191 times after settling against 1,068, with no settled frame over the budget, and used 89% of the budget on average against 84%. `dynamic_resolution_holds_a_scale_inside_its_band_under_noise` keeps 40 of those loads within the budget with at most 20 changes after settling.

## Tests

`cargo test -p pfx-gpu --lib screens` covers the fit and bars of 16:10, 16:9, 21:9 and 32:9 windows against each policy, a width-anchored policy on each of them with its refusals, the shared letterbox, the windowed position kept, stored, restored on its monitor, clamped, and dropped when the monitor is gone, device detection from each source, mode switching on a stand-in window, the Deck's fixed mode, the native video mode, the TOML table and its refusals, `bars = "backdrop"` with the window and content in units, the pointer through the bars and dynamic resolution under a synthetic series. `crates/input/tests/screens.rs` maps the pointer through `Input`. `cargo test -p pfx-live --lib upscale` checks the shader and the CPU reference; the GPU tests run by name under pgpu:

```
pgpu run --class clip --as pfx -- cargo test -q -p pfx-live --lib upscale::tests:: -- --ignored --test-threads=1
pgpu run --class clip --as pfx -- cargo test -q -p pfx-editor --test colour -- --ignored --test-threads=1
pgpu run --class clip --as pfx -- cargo test -q -p pfx-game --lib gpu_tests::a_backdrop -- --ignored --test-threads=1
pgpu run --class clip --as pfx -- cargo test -q -p pfx-live --test dynamic_resolution -- --ignored --test-threads=1
pgpu run --class clip --as pfx -- cargo test -q -p pfx-live --test viewport -- --ignored --test-threads=1
pgpu run --class clip --as pfx -- cargo test -q -p pfx-live --lib taa::tests::gpu_ -- --ignored --test-threads=1
pgpu run --class clip --as pfx -- cargo test -q -p pfx-post --lib viewport -- --ignored --test-threads=1
```

`crates/live/tests/viewport.rs` holds the four renderer checks above: scale 1 against a plain renderer, a fixed viewport against a resized renderer, a scale change every 4 frames for 120 frames with no history restart and the same target textures (`Renderer::target_textures`) before and after, and the step measurement. `taa::tests::gpu_resamples_history_across_viewport_changes` matches the GPU resolve to the CPU reference through seven viewport sizes, and `pfx-post`'s `a_viewport_post_equals_a_chain_at_the_viewport_size` runs 37 chains (every style, bloom, halation, neon, the tape, depth and normal passes, regions) inside a larger allocation with a loud colour outside the rect and matches each chain built at the rect's size exactly.

`stats::tests::the_device_label_fills_every_line_once_set` covers the frame stats field, and `frame_stats_carry_a_positive_gpu_time_within_two_frames` checks the detected label in a real file.
