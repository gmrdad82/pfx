# Window and display

The window feature of `pfx-gpu`. A game uses it for the display mode, the present mode, the render size, and the frame cap. `pfx` does not open a window, and nothing in this crate opens one either. The planning, the present-mode choice, the letterbox, and the frame pacer are pure functions. The tests drive them with fake monitors and a fake clock.

`Gpu::window`, `Gpu::window_with`, and `Gpu::window_checked` are unchanged. They still configure the surface as Fifo with a queue of 2 frames. `Gpu::window_present` takes a `PresentPreference` for the first configure, and `WindowTarget::set_present` changes it later. `WindowTarget::resize` still only changes the surface size.

The types live in `pfx_gpu::window`. The functions that take a winit `Window` (`apply_display_mode`, `monitors`, `placement`, `host`, `attention`, `minimized`, `refresh_millihertz`) need the `window` feature.

## Display modes

`plan_display_mode` turns a `DisplayMode`, a `PlatformCaps` (`exclusive: bool`), and a `Monitors` list into a `ModePlan`. `apply_plan` plays that plan on anything that implements `WindowOps`. `apply_display_mode` reads the window's monitors, fills the platform caps from `host`, plans, and calls winit 0.30.

| Mode | Fullscreen | Decorations | Size and position |
|---|---|---|---|
| `Windowed { size, position }` | cleared | on | `set_outer_position` when a position is set, then `request_inner_size` |
| `BorderlessWindowed { size, position }` | cleared | off | the same optional position and size, not a fullscreen mode |
| `BorderlessFullscreen { monitor }` | `Fullscreen::Borderless(Some(monitor))` | off | the compositor owns the size |
| `ExclusiveFullscreen { monitor, video_mode }` | `Fullscreen::Exclusive(video_mode)` | off | the video mode owns the size |

A zero width or height is `PlanError::EmptySize`. Fullscreen with an empty monitor list is `PlanError::NoMonitor`.

Leaving fullscreen happens before the decoration, position, and size calls, so a windowed mode can resize after the compositor has let go. Entering fullscreen sets decorations off and then the fullscreen mode, and does not also request a size or a position.

`VideoModeChoice::Exact` matches size, bit depth, and refresh. `VideoModeChoice::SizeRefresh` matches size and refresh and takes the highest bit depth, the later mode when two depths match. `VideoModeChoice::Native` takes the monitor's own size at its current refresh (within 1 Hz), then the highest bit depth; with no known refresh it takes the highest refresh.

`ModePlan::resizable` is true for `Windowed` and false otherwise. `apply_plan` calls `WindowOps::set_resizable` after the decorations in a windowed plan; the trait's default does nothing. The game-facing modes (windowed, borderless, fullscreen), the Steam Deck and the aspect policy are in `docs/screens.md`.

## Monitors

`monitors(&window)` lists the window's monitors as a `Monitors` (the same list `apply_display_mode` plans against), and `placement(&window)` is the window's current monitor's name with the window's outer top-left relative to that monitor's top-left, or `None` where the platform reports no position (Wayland). `Point` is serde, so a game's settings can keep a position (`DisplaySettings::position`, `docs/screens.md`).

`MonitorChoice` is `Current`, `Primary`, `Index`, or `Name`. A name matches `Monitor::name` exactly. `Current` and `Primary` come from the window. `apply_display_mode` numbers monitors in `Window::available_monitors` order and uses that same order for the winit handles.

When the choice misses, the plan uses the current monitor, then the primary, then the first, and sets `Fallback::MonitorMissing`. An exclusive mode can still use a video mode of that fallback monitor. The result says the requested monitor was missing.

## What each platform does

`Host::supports_exclusive` is the `PlatformCaps` value `apply_display_mode` uses. On Linux and the other unix desktops winit builds for, a window whose `xdg_toplevel` is present is `Host::Wayland`; otherwise it is `Host::X11`.

| Host | Exclusive fullscreen | VRR present preference |
|---|---|---|
| Windows | yes, `Fullscreen::Exclusive` | Mailbox, FifoRelaxed, Fifo |
| macOS | yes. Borderless fullscreen is the one that keeps spaces usable | Mailbox, Fifo |
| X11 | yes | Mailbox, Fifo, FifoRelaxed |
| Wayland | no. winit ignores `Fullscreen::Exclusive` | Mailbox, Fifo |
| Android, iOS, the web, anything else | no | Fifo |

Where exclusive is unsupported, or the video mode is not on the chosen monitor, the plan is borderless fullscreen on that monitor. The result's `fallback` is `ExclusiveUnsupported` or `VideoModeMissing`. A missing monitor is reported ahead of either of those. The winit call is `Fullscreen::Borderless`, not `Exclusive`, so Wayland is not asked for a mode it drops.

VRR present modes are what `PresentPreference::vrr` asks for, in that order. `choose_present` still keeps only a mode the surface lists, and falls back to Fifo. Mailbox is first on every desktop that can offer it: it keeps the latest frame and does not tear, which is the present mode variable refresh can follow. Immediate is left out. A tear drops some drivers out of the variable-refresh window, and on Wayland it steps around the compositor that actually owns the refresh.

X11 asks for Fifo before FifoRelaxed. FifoRelaxed may tear when a frame is late. Wayland asks for Mailbox and then Fifo. wgpu's Mailbox is supported on Wayland Vulkan when the surface lists it; many surfaces list only Fifo, and Fifo is the fallback. The compositor, not the present mode, varies the refresh. On Hyprland that is the display's variable-refresh setting, usually for a fullscreen surface.

`PresentPreference::vrr` sets `desired_maximum_frame_latency` to 1 so the swapchain does not hold a second frame the pacer has already spaced. `fifo` and `game` keep 2, which is what `Gpu::window` configures today.

## Present modes

`PresentPreference::default` and `fifo` request Fifo only, latency 2. `game` requests Mailbox, then FifoRelaxed, then Fifo, latency 2. That is the usual game order when variable refresh is not being asked for.

`choose_present` walks the request and takes the first mode the surface lists. If none of them is listed, the choice is Fifo and `fell_back` is true, including when the surface reported no modes. A latency below 1 is raised to 1. `WindowTarget::present` records the request, the mode that was configured, whether that was a fallback, and the latency. The same values are on `WindowTarget::config`.

`Gpu::window_checked` stays on Fifo. A game that also wants the live-renderer floor calls `set_present` after `window_checked`.

`set_present` can run at any time, between frames: it reads the surface's modes again, chooses, and reconfigures the surface at its current size. pfx-game calls it when a game's settings toggle `pacing.vrr`, before the next frame is drawn, so variable refresh turns on and off with no restart (`docs/game.md`, "Pacing").

## Render size and the 1920×1080 layout

The swapchain follows the window. `WindowTarget::resize` is that size. The picture the game draws can be a different size: `render_size` is `Native`, a `Factor` (rounded, and refused when it is not a positive finite scale), or an explicit `Size`. `viewport` builds the render size and the letterbox into it.

`LAYOUT_UNITS` is 1920×1080. `letterbox` fits that rectangle inside the render target, keeping the layout's aspect. A wider target gets bars on the left and right. A taller target gets bars above and below. Leftover pixels from an odd gap go to the right and the bottom. The visible rectangle is half-open: the near edges are inside, the far edges are the bars.

`Letterbox::matrix` is a column-major 4×4. It maps a layout point `(x, y, 0, 1)` to a render-target pixel, origin at the top left, y down, the same way window coordinates run. `inverse` maps a render-target pixel back to layout units. `Viewport::pointer_to_layout` does both steps for a pointer that arrived in window pixels: window pixel to render pixel, then the inverse. `pointer_inside` is false in the bars.

## Frame caps

`FrameCap` is `Fps30`, `Fps60`, `Fps90`, `Fps120`, or `Uncapped`. `FrameCaps::new(focused, background)` stores one cap for the focused window and one for the background, and pauses when minimized. The background cap is the game's. There is no engine default for it. Set `pause_when_minimized` to false to keep drawing into a minimized window.

A window that is focused and not occluded uses the focused cap. Unfocused or occluded (`Attention::Occluded`, from winit's `Occluded`) uses the background cap. Occlusion does not pause. `attention` maps `WindowEvent::Focused` and `WindowEvent::Occluded`. Minimized is not one of those events: the game reads `minimized` (`Window::is_minimized`, with an unknown answer treated as not minimized) and sends `Attention::Minimized`.

`FramePacer::poll` takes a `Clock` and returns `Start`, `Wait { until_ns }`, or `Paused`. It does not sleep. The caller waits until `until_ns` and polls again. The schedule is absolute on that clock: frame `n` is due at `origin + n * 1_000_000_000 / fps` nanoseconds, using integer division. Waiting until the deadline does not accumulate error. A poll that arrives late starts one frame and aims the next deadline at the next grid point after now, so a stall does not turn into a burst. Changing cap, focus, occlusion, or the VRR rate starts the new rate at that poll instead of catching up the old grid.

`FakeClock` is the clock the tests advance. `SystemClock` is a monotonic clock a game can pass in. Its zero is the instant it was built. The pacer never reads a clock of its own.

## Variable refresh

The monitor's refresh is `MonitorHandle::refresh_rate_millihertz`, also stored on `Monitor::refresh_millihertz` by `apply_display_mode`. `FramePacer::set_vrr(Some(refresh))` opts into a cap of refresh minus 3 fps: `(millihertz - 3000) / 1000`. The effective rate is the smaller of the chosen cap and that limit. Uncapped means the limit. A refresh so low that the limit would be 0 leaves the chosen cap alone.

| Refresh | Limit | 30 | 60 | 90 | 120 | Uncapped |
|---|---|---|---|---|---|---|
| 60 Hz | 57 | 30 | 57 | 57 | 57 | 57 |
| 120 Hz | 117 | 30 | 60 | 90 | 117 | 117 |
| 144 Hz | 141 | 30 | 60 | 90 | 120 | 141 |
| 165 Hz | 162 | 30 | 60 | 90 | 120 | 162 |

Whether variable refresh is enabled stays with the operating system or the compositor. The engine does not turn it on. Pair `set_vrr` with `PresentPreference::vrr(host)` when the game wants the present mode that preference describes. The cap still applies if the surface fell back to Fifo.

The window feature sets no class, application id, or protocol hint to mark a window as a game. Where the desktop notices a game, it notices the process that created the window. Create the window in the game's own process. A helper process outside that process tree is a different program. Windowed, borderless, and fullscreen count the same. A minimized or unfocused game stops counting, which is the same moment the background cap or the minimize pause takes over.
