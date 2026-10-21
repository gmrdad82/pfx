# Window and display

The window feature of `pfx-gpu`: the display mode, the present mode, the render size and the frame pacing every pfx window shares, the game runtime's (`docs/game.md`) and the editor shell's (`pfx gui` and any egui tool on `pfx-editor-shell`, `docs/editor-window.md`). Nothing in this crate opens a window: the caller creates it and hands it over. The planning, the present-mode choice, the letterbox, the frame pacer and the pacing report are pure functions. The tests drive them with fake monitors and a fake clock.

Every window takes the same two switches, `Present { vrr, frame_rate }`, and nothing else paces a frame. `Gpu::window`, `Gpu::window_with` and `Gpu::window_checked` configure the surface for `Present::default()` (VRR on). `Gpu::window_present` takes a `PresentPreference` for the first configure, `WindowTarget::follow(gpu, window, present)` switches a live window to another `Present` in place, and `WindowTarget::set_present` sets a preference directly. `WindowTarget::resize` only changes the surface size.

The types live in `pfx_gpu::window`. The functions that take a winit `Window` (`apply_display_mode`, `monitors`, `placement`, `host`, `fullscreen`, `present_preference`, `attention`, `minimized`, `refresh_millihertz`) need the `window` feature.

## Display modes

`plan_display_mode` turns a `DisplayMode`, a `PlatformCaps` (`exclusive: bool`), and a `Monitors` list into a `ModePlan`. `apply_plan` plays that plan on anything that implements `WindowOps`. `apply_display_mode` reads the window's monitors, fills the platform caps from `host`, plans, and calls winit 0.30.

| Mode | Fullscreen | Decorations | Size and position |
|---|---|---|---|
| `Windowed { size, position }` | cleared | on | `set_outer_position` when a position is set, then `request_inner_size` |
| `BorderlessWindowed { size, position }` | cleared | off | the same optional position and size, not a fullscreen mode |
| `BorderlessFullscreen { monitor }` | `Fullscreen::Borderless(Some(monitor))`, or `Fullscreen::Borderless(None)` for `Current` | off | the compositor owns the size |
| `ExclusiveFullscreen { monitor, video_mode }` | `Fullscreen::Exclusive(video_mode)` | off | the video mode owns the size |

A zero width or height is `PlanError::EmptySize`. Fullscreen with an empty monitor list is `PlanError::NoMonitor`.

Leaving fullscreen happens before the decoration, position, and size calls, so a windowed mode can resize after the compositor has let go. Entering fullscreen sets decorations off and then the fullscreen mode, and does not also request a size or a position.

`VideoModeChoice::Exact` matches size, bit depth, and refresh. `VideoModeChoice::SizeRefresh` matches size and refresh and takes the highest bit depth, the later mode when two depths match. `VideoModeChoice::Native` takes the monitor's own size at its current refresh (within 1 Hz), then the highest bit depth; with no known refresh it takes the highest refresh.

`ModePlan::resizable` is true for `Windowed` and false otherwise. `apply_plan` calls `WindowOps::set_resizable` after the decorations in a windowed plan; the trait's default does nothing. The game-facing modes (windowed, borderless, fullscreen), the Steam Deck and the aspect policy are in `docs/screens.md`.

## Monitors

`monitors(&window)` lists the window's monitors as a `Monitors` (the same list `apply_display_mode` plans against), and `placement(&window)` is the window's current monitor's name with the window's outer top-left relative to that monitor's top-left, or `None` where the platform reports no position (Wayland). `Point` is serde, so a game's settings can keep a position (`DisplaySettings::position`, `docs/screens.md`).

`MonitorChoice` is `Current`, `Primary`, `Index`, or `Name`. A name matches `Monitor::name` exactly. `Current` and `Primary` come from the window. `apply_display_mode` numbers monitors in `Window::available_monitors` order and uses that same order for the winit handles.

`Current` names no monitor: the plan is `FullscreenPlan::Borderless { monitor: None }` and winit's `Fullscreen::Borderless(None)`, so the window goes fullscreen on the output it is on. On Wayland the current monitor is unknown until the window maps, and the compositor puts a new window on the focused output (Hyprland, sway, GNOME), so the game opens on the display it was launched from rather than the first one; on Windows, macOS and X11 winit takes the window's current monitor. `ModePlan::monitor` still reports the current monitor where it is known.

When the choice misses, the plan does the same as `Current` and sets `Fallback::MonitorMissing`. An exclusive mode needs a video mode of a known monitor: it uses the chosen one, else the current one, and with neither it is borderless with `None` and `Fallback::MonitorMissing`. The result says the requested monitor was missing.

## What each platform does

`Host::supports_exclusive` is the `PlatformCaps` value `apply_display_mode` uses. On Linux and the other unix desktops winit builds for, a window whose `xdg_toplevel` is present is `Host::Wayland`; otherwise it is `Host::X11`.

| Host | Exclusive fullscreen | VRR on | VRR off |
|---|---|---|---|
| Windows | yes, `Fullscreen::Exclusive` | windowed: Fifo; fullscreen: Immediate, then Fifo | Fifo |
| macOS | yes. Borderless fullscreen is the one that keeps spaces usable | Fifo | Fifo |
| X11 | yes | Mailbox, then Fifo | Fifo |
| Wayland | no. winit ignores `Fullscreen::Exclusive` | Mailbox, then Fifo, and the surface's content type `game` | Fifo, content type `none` |
| Android, iOS, the web, anything else | no | Fifo | Fifo |

Where exclusive is unsupported, or the video mode is not on the chosen monitor, the plan is borderless fullscreen on that monitor. The result's `fallback` is `ExclusiveUnsupported` or `VideoModeMissing`. A missing monitor is reported ahead of either of those. The winit call is `Fullscreen::Borderless`, not `Exclusive`, so Wayland is not asked for a mode it drops.

Whether the display varies its refresh stays with the operating system, the driver and the compositor; the engine never turns it on. VRR on asks for the present mode each platform's variable refresh follows, and VRR off is steady vsync at the display's refresh: Fifo, no tearing.

- **Windows.** wgpu's DX12 swapchain is always the flip model (`DXGI_SWAP_EFFECT_FLIP_DISCARD`, created with `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING` where the system supports it). Fifo is `Present(1, 0)`: a windowed flip-model swapchain is the one Windows' variable refresh for windowed games follows. Immediate is `Present(0, DXGI_PRESENT_ALLOW_TEARING)`, the tearing flag, which pfx asks for only in fullscreen (`fullscreen(&window)`), where variable refresh needs it; Windows ignores it for a composed window. Mailbox (`Present(0, 0)`) is never asked for.
- **macOS.** Metal offers Fifo and Immediate (`displaySyncEnabled`). ProMotion and external adaptive-sync displays follow a Fifo layer's presents on their own, so both settings are Fifo. A `CAMetalDisplayLink` with `preferredFrameRateRange` is the way to steer it further; neither winit 0.30 nor wgpu 30 exposes one.
- **X11.** Mailbox where the driver lists it, else Fifo. The X server's variable refresh is a driver setting for a fullscreen window (`VariableRefresh` on amdgpu, `__GL_GSYNC_ALLOWED` on NVIDIA).
- **Wayland.** Mailbox where the surface lists it, else Fifo; Mesa lists Mailbox on RADV, ANV and NVK. The compositor owns the refresh: on Hyprland that is `misc:vrr` (1 always, 2 fullscreen, 3 fullscreen with a `game` or `video` content type), on KWin and GNOME their adaptive-sync settings. pfx sets the window's `wp_content_type_v1` to `game` with VRR on and `none` with it off, the hint `misc:vrr = 3` reads, and flips it in place. Tearing (`wp_tearing_control_v1`) is Mesa's: it marks a surface async only for the Immediate mode, which pfx never asks for on Wayland, so a frame never tears there.
- **Vulkan everywhere.** `PresentMode::AutoVsync` and `AutoNoVsync` are wgpu's own fallbacks (FifoRelaxed then Fifo; Immediate then Mailbox then Fifo). pfx lists its modes itself so the choice is the same on every driver. FifoRelaxed tears a late frame and Immediate tears always, so neither is asked for on Linux.

## Present modes

`PresentPreference::of(present, host, fullscreen)` (also `present.preference(host, fullscreen)`) is the table above: `fifo()` when VRR is off, `host.vrr_present_modes(fullscreen)` when it is on. `PresentPreference::default` and `fifo` request Fifo only. `PresentPreference::forced(mode)` requests one mode.

`choose_present` walks the request and takes the first mode the surface lists. If none of them is listed, the choice is Fifo and `fell_back` is true, including when the surface reported no modes. The swapchain's queue follows the mode chosen (`frame_latency`): 2 frames for Fifo and FifoRelaxed, so a frame is always ready at the vblank, and 1 for Mailbox and Immediate, which never wait. A latency of 3 was measured against these on every mode (below) and gained nothing, so they stay. `PFX_FRAME_LATENCY` (`LATENCY_VAR`, 1 to 3) overrides it for a diagnosis or a measurement. The `present` field records the request, the mode that was configured, whether that was a fallback, and the latency. The same values are on `WindowTarget::config`.

`present_preference(&window, present)` is what a window asks for: the table for the window's host and fullscreen state, unless `PFX_PRESENT` (`PRESENT_VAR`) is `fifo`, `fifo-relaxed`, `mailbox` or `immediate`, which forces that one mode for a diagnosis or a measurement. `follow` calls it, reconfigures the surface at its current size only when the request changed, and sets the Wayland hint; it runs between frames, so a switch lands on the next frame with no restart.

## VRR in effect

Asking for VRR does not make the display vary its refresh: the display, the driver and the compositor decide. When it does not, a frame paced on a 120 fps grid lands on a fixed refresh that is not a multiple of it, and now and then a frame waits a whole refresh more. So a window checks what it gets. `WindowTarget::present(queue, frame, tag)` presents and, while VRR is asked for or the pacing report is on, asks the compositor when the frame was shown; `WindowTarget::shown()` drains those times, and a `VrrProbe` reads them in windows of 120 intervals: intervals that land off the multiples of the refresh mean VRR is in effect, and 90% of intervals on a multiple (within 0.3 ms or 15% of a refresh) mean it is not. The refresh is the one the compositor reports with each frame; where it reports 0 (the protocol's "not a constant rate", which wlroots' headless output also sends) the monitor's refresh from winit stands in, and with neither a 0 counts as a free interval. A verdict changes only when two windows in a row read the same, so a timestamp source with jitter cannot flip it back and forth, and `set_present` reconfigures the surface only when the chosen mode or latency changes. `vrr_in_effect()` is `None` until the first verdict.

When VRR is asked for and not in effect, `follow` asks for Mailbox, then FifoRelaxed, then Fifo (unless `PFX_PRESENT` forces a mode), and the window's `FramePacer::set_vrr_in_effect(Some(false))` paces the caps and Auto as with VRR off: whole refreshes of the display, so every frame keeps the same phase. `WindowTarget::settled()` is true once after a verdict changes; the editor shell and the game runtime then call `follow`, hand the verdict to the pacer, start a new pacing line, and print `describe()` ("presents Mailbox at frame latency 1, VRR asked but not in effect: the refresh is fixed").

| Host | What pfx can know | Verdict |
|---|---|---|
| Wayland | `wp_presentation` feedback, through winit's raw handles: the time each frame was shown, the refresh, and `discarded` for a frame a later one replaced | measured, as above |
| X11 | nothing: the RandR output's `vrr_capable` and the driver's `VariableRefresh` need an X client library pfx does not carry, and Vulkan's `VK_KHR_present_wait` is not exposed by wgpu 30 | `None`: the request is trusted |
| Windows | DXGI frame statistics (`IDXGISwapChain::GetFrameStatistics`) are reachable through wgpu-hal's DX12 swapchain but are not read yet | `None`: the request is trusted |
| macOS | `CAMetalDrawable`'s `presentedTime` and `NSScreen`'s refresh-interval range need Objective-C bindings pfx does not carry | `None`: the request is trusted |

## Render size and the 1920×1080 layout

The swapchain follows the window. `WindowTarget::resize` is that size. The picture the game draws can be a different size: `render_size` is `Native`, a `Factor` (rounded, and refused when it is not a positive finite scale), or an explicit `Size`. `viewport` builds the render size and the letterbox into it.

`LAYOUT_UNITS` is 1920×1080. `letterbox` fits that rectangle inside the render target, keeping the layout's aspect. A wider target gets bars on the left and right. A taller target gets bars above and below. Leftover pixels from an odd gap go to the right and the bottom. The visible rectangle is half-open: the near edges are inside, the far edges are the bars.

`Letterbox::matrix` is a column-major 4×4. It maps a layout point `(x, y, 0, 1)` to a render-target pixel, origin at the top left, y down, the same way window coordinates run. `inverse` maps a render-target pixel back to layout units. `Viewport::pointer_to_layout` does both steps for a pointer that arrived in window pixels: window pixel to render pixel, then the inverse. `pointer_inside` is false in the bars.

## Pacing

Two switches, and nothing else, pace a pfx window. An app keeps them in its own settings (a game's `pacing`, `pfx gui`'s `[display]`, any app's own row) and hands them over; both change at run time, with no restart.

```rust
use pfx_gpu::window::{FrameRate, Present};

let present = Present { vrr: true, frame_rate: FrameRate::Uncapped };
```

| Switch | Values | Default |
|---|---|---|
| `vrr` | `true`: adaptive sync where the platform offers it (the table above); `false`: steady vsync at the display's refresh, Fifo, no tearing | `true` |
| `frame_rate` | `Uncapped` (`"uncapped"`): each frame presents as soon as it is ready, paced by the swapchain alone; `Cap60` (`"cap60"`): a steady 60 frames a second; `Cap120` (`"cap120"`): a steady 120; `Auto` (`"auto"`): 120 when the frame holds it, else the steadiest lower rate it does hold | `Uncapped` |

There is no other cap: no 30 or 90, no background or unfocused rate, and no sleep. Focus and occlusion never slow a frame. A window that has nothing to animate is event-driven and draws nothing until an event, a timer it asked for or a waker; anything that animates (playback, a drag, a camera ease, a progressive render, a game) asks for the next frame as it presents one.

`FramePacer::new(present)` does the pacing. `poll(&clock)` returns `Start { next_ns }`, `Wait { until_ns }` or `Paused`; it never sleeps, and the caller hands winit `ControlFlow::WaitUntil` the deadline. `set_present` switches the frame rate, `set_refresh(Some(millihertz))` gives it the monitor's refresh (`refresh_millihertz`), `on(Attention::Minimized(..))` pauses it while minimized and `set_pause_when_minimized(false)` turns that pause off.

- **Uncapped** is always `Start { next_ns: None }`. With Fifo the swapchain holds the window to the display's refresh, or, under variable refresh, to the frame time itself; with Mailbox the latest frame wins each refresh.
- **Cap60 and Cap120** keep an absolute grid on the clock: frame `n` is due at `origin + n × period`, in integer nanoseconds with no rounding carried, so waiting until each deadline never drifts. A poll that arrives late starts one frame and aims the next at the next grid point after now, so a stall never turns into a burst. A switch starts the new rate at that poll instead of catching up the old grid.
- **Auto** keeps the same grid at a rate it picks from a ladder: with VRR off and the refresh known, whole refreshes from the one that gives 120 fps down to 30 fps (`auto_period`: 120, 80, 60, 48, 40, 34.3 and 30 fps on 239.97 Hz; 60 and 30 on 60 Hz); otherwise 120, 100, 90, 80, 72, 60, 50, 45, 40 and 30. It times each frame on the UI thread, from the `Start` to the next poll (swapchain back-pressure included), in windows of 30 frames. When a window's 90th percentile overruns the period, it steps down at once to the first rung whose period is 10% longer than that frame time. It steps up one rung only after 8 windows in a row whose 90th percentile is 30% under the next rung's period, and a rise that falls back within 16 windows doubles that wait (up to 256 windows), so a scene whose cost only shows at the higher rate (the GPU, behind the swapchain) settles instead of flapping. A switch of the settings or the refresh starts it again at 120. `FramePacer::auto_rung` is the rung it is on.
- **The period** (`frame_period(present, refresh)`, a `Period` in exact nanoseconds per frame): with VRR on, 1/60 s or 1/120 s, which the display follows. With VRR off and the refresh known, a whole number of refreshes (`refresh_vblanks(refresh, fps)`: the refresh in hertz over the cap, rounded down after 0.6 Hz of slack), so Fifo latches every frame on the same phase of the display's own clock. Cap60: four refreshes of 239.97 Hz (59.99 fps), one of 60 Hz, two of 120 Hz. Cap120: two refreshes of 239.97 Hz (119.99 fps), three of 360 Hz, one of 120 Hz. A refresh that is no multiple of the cap gets the steadiest rate at or above the cap it can show without judder: under Cap60, 72 fps on 144 Hz and 82.5 on 165 Hz; under Cap120, one refresh of 144 Hz (144 fps) or of 165 Hz. A refresh under the cap is shown at its own rate (Cap120 on a 60 Hz display is 60 fps).

The grid takes its rate from the display's refresh, not its phase: neither winit 0.30 nor wgpu 30 hands an app its presentation timestamps (`VK_KHR_present_wait`, `VK_GOOGLE_display_timing`) or its frame-callback times. On Wayland pfx reads `wp_presentation` itself, for the report and the VRR verdict above, not to set the phase. Under VRR there is no phase to lock to. With VRR off, a deadline that lands within the event loop's wake jitter of a refresh boundary can slip one refresh.

`FakeClock` is the clock the tests advance. `SystemClock` is a monotonic clock a window passes in; its zero is the instant it was built, and `instant(ns)` turns a deadline into the `Instant` winit waits for. The pacer never reads a clock of its own.

The game runtime (`docs/game.md`, "Pacing") and the editor shell (`docs/editor-window.md`) run this pacer around winit; an app with its own winit loop does the same: request a redraw on `Start`, wait on `Wait`, and call `follow` when the switches change.

## Pacing report

`pfx_gpu::presents` measures what a window presents. `Presents::new(budget_ns)` takes each present's time (`presented(now_ns)`) and keeps the interval to the previous one, unless `idle()` came between: a window that stopped animating and drew again on the next event does not count its idle gap. `report()` gives a `PacingReport`: `frames`, `intervals`, `p50_ms`, `p99_ms` (10 µs buckets, never above the maximum), `max_ms`, `mean_ms`, `stddev_ms`, `late`, the intervals over the budget, and `missed`, the intervals of 1.5 budgets or more; `holds()` is no late frame and a p99 within the budget. Beside the intervals it keeps three more spreads: `acquired(wait_ns)`, the time the window waited for the swapchain's next image (`acquire_*_ms`); `present_call(ns)`, the time the present call itself took (`present_call_*_ms`); and `shown(Shown)`, the time the compositor showed each frame (`shown_intervals`, `shown_p50_ms`, `shown_p99_ms`, `shown_max_ms`, and `discarded`, the frames a later one replaced before they were shown). Shown times chain only between frames of one run: `Pacing::tag()` names the run, it moves on at every `idle()` and every setting, and the window hands it to `WindowTarget::present`. `GUI_BUDGET_NS` and `TUI_BUDGET_NS` are both 8.33 ms, 120 frames a second; under `cap60` the budget is 16.67 ms (`CAP60_BUDGET_NS`), the cap's own period.

`Pacing` is the file form both windows write. With `PFX_PACING=<path>` (`PACING_VAR`) set, the window records every present and rewrites the file whole (through a `.part` file, renamed) at most once a second and when it closes, so a killed window still leaves its last second. The file is written on its own thread (`pfx-pacing`, `WRITER_THREAD`): the window hands it a snapshot of the sections and never waits on the disk, an update that finds one still waiting to be written is dropped, and `finish` (or dropping the `Pacing`) hands over the last report and waits for it to land. One JSON line per setting the window ran under (`WindowTarget::setting`: the host, the configured present mode and frame latency, `vrr`, `vrr_in_effect` (`true`, `false`, or `null` before a verdict), `frame_rate`), so switching VRR or the frame rate in a run gives one line each:

```json
{"schema_version":2,"host":"wayland","present_mode":"Mailbox","frame_latency":1,"vrr":true,"vrr_in_effect":true,"frame_rate":"uncapped","budget_ms":8.330,"frames":3120,"intervals":3119,"p50_ms":9.450,"p99_ms":14.220,"max_ms":28.373,"mean_ms":9.514,"stddev_ms":1.512,"late":2978,"missed":73,"acquire_p50_ms":0.030,"acquire_p99_ms":0.060,"acquire_max_ms":0.148,"acquire_mean_ms":0.024,"present_call_p50_ms":0.040,"present_call_p99_ms":0.420,"present_call_max_ms":18.488,"viewport_p50_ms":8.620,"viewport_p99_ms":13.030,"viewport_max_ms":22.655,"paint_p50_ms":0.120,"paint_p99_ms":0.300,"paint_max_ms":0.975,"shown_intervals":3117,"shown_p50_ms":9.450,"shown_p99_ms":14.250,"shown_max_ms":28.473,"discarded":0}
```

Times are milliseconds with three decimals. The interval is present call to present call on the window's thread: with Fifo that is the swapchain's cadence once its queue is full, with Mailbox and Immediate it is the frame's own time. The shown intervals are the moments the display showed the frames, where the platform reports them (Wayland; `null` elsewhere). Unset, nothing is recorded and a frame pays a few branches.

## UI thread priority

A busy machine preempts the window's thread, and a frame that loses the CPU for a few milliseconds shows as a late interval however fast the frame is. `pfx_gpu::priority::raise_current_thread()` raises the calling thread, opt-in and off by default: an editor-shell app returns true from `App::raised_priority`, and any window takes `PFX_UI_PRIORITY=raised` (`PRIORITY_VAR`). It is never a realtime class.

| OS | What it does | Without privileges |
|---|---|---|
| Linux | the thread's nice to -10 (`RAISED_NICE`), `setpriority` on its thread id | as far as `RLIMIT_NICE` allows (a limit of 31 allows -11; the common default of 0 allows nothing, and the window says so on stderr). RealtimeKit's D-Bus call would need a D-Bus crate pfx does not carry. |
| Windows | `SetThreadPriority(THREAD_PRIORITY_ABOVE_NORMAL)`; MMCSS is not used, its game class runs in the realtime band | always allowed |
| macOS | `pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE)` | always allowed |

## Frame profile

`PFX_PROFILE=<path>` (`pfx_editor_shell::profile::PROFILE_VAR`) makes the editor window write one JSON line per presented frame, flushed once a second, so a killed window keeps all but its last second. Each line has `t`, the milliseconds since the window opened; `setting`, the present mode, `vrr` or `fixed` and the frame rate; `scale`, the viewport's render scale (`null` when it renders at full size with no budget); `cpu`, the frame's time on the window's thread, and that time by phase: `ui` (egui's layout), `viewport` (the 3D viewport's frame, its wait for the GPU included), `tessellate`, `acquire` (the swapchain image), `paint` (egui's upload, draw and the frame's one submission) and `present`. When the renderer's timestamps for a new frame have arrived, `gpu_frame`, `gpu` and `passes` give that frame's number, its GPU milliseconds and each pass's, and `ui_gpu` the GPU time of egui's pass. GPU times come from timestamp queries (`GpuProfiler`) and lag the CPU line by a frame or two; a device without timestamps leaves them out. Unset, the window records nothing and pays one branch per phase.

```json
{"t":93374.176,"setting":"Mailbox vrr uncapped","scale":0.750,"cpu":9.625,"ui":0.571,"viewport":7.977,"tessellate":0.484,"acquire":0.083,"paint":0.437,"present":0.073,"gpu_frame":7864,"gpu":5.061,"passes":{"opaque PBR":1.319,"post":0.693,"TAA resolve":0.441},"ui_gpu":0.330}
```

## Measured pacing

The real `pfx gui` window, release build, in a private headless compositor (labwc on the wlroots headless backend, its own runtime folder and `/tmp`, muted) under `pgpu run --class clip`, on an AMD Radeon RX 9060 XT under RADV, with other jobs on the machine (load average 15 to 28). The headless output runs at 240 Hz, set with `wlr-randr`, and has no adaptive sync, so these runs show pacing, not variable refresh. The scene is `samples/chase.pitofx`: 25 s after the window opens, loop, Chase cam and play are clicked, and its 10 s timeline moves the camera through the street, looped. Each setting runs 60 s, switched live in one session by clicking the `vrr` and frame-rate chips, the pointer parked away from the window after each click. `PFX_PACING` gives the numbers: present call to present call on the UI thread, a `late` interval over 16.67 ms, a `missed` one 25 ms or more. RADV lists Mailbox on the Wayland surface, so VRR on is Mailbox and VRR off Fifo.

A 1280×720 window on a 1920×1080 output, in two sessions in opposite orders. The first setting of a session also carries the window's start-up (the stage and its pipelines, the timeline starting), which is where its long frames come from:

| vrr | frame rate | mode | session | intervals | mean fps | p50 ms | p99 ms | max ms | late | missed |
|---|---|---|---|---|---|---|---|---|---|---|
| on | uncapped | Mailbox | 1st of A | 12089 | 197 | 4.60 | 18.21 | 43.1 | 174 | 24 |
| on | uncapped | Mailbox | 4th of B | 13944 | 230 | 4.35 | 9.49 | 56.1 | 93 | 29 |
| on | cap60 | Mailbox | 2nd of A | 3589 | 59.1 | 16.66 | 35.01 | 97.5 | 1768 | 88 |
| on | cap60 | Mailbox | 5th of B | 3642 | 60.0 | 16.66 | 18.97 | 23.6 | 1768 | 0 |
| on | cap120 | Mailbox | 3rd of A | 6746 | 111 | 8.99 | 24.16 | 156.4 | 295 | 48 |
| on | cap120 | Mailbox | 6th of B | 7345 | 120.0 | 8.25 | 12.32 | 23.6 | 8 | 0 |
| off | cap120 | Fifo | 4th of A | 7283 | 120.0 | 8.34 | 12.08 | 17.5 | 2 | 0 |
| off | cap120 | Fifo | 2nd of B | 7206 | 118.7 | 8.46 | 11.92 | 40.6 | 39 | 14 |
| off | uncapped | Fifo | 5th of A | 13311 | 219 | 4.43 | 12.16 | 52.9 | 75 | 37 |
| off | uncapped | Fifo | 3rd of B | 13793 | 227 | 4.33 | 11.21 | 54.1 | 95 | 33 |
| off | cap60 | Fifo | 6th of A | 3707 | 60.0 | 16.65 | 20.82 | 61.3 | 1812 | 14 |
| off | cap60 | Fifo | 1st of B | 3629 | 58.9 | 16.68 | 36.00 | 91.4 | 1824 | 60 |

- **Cap120** holds 120 frames a second (8.33 ms a frame on the mean, 119.99 fps from two refreshes of 239.97 Hz with VRR off) with a p99 of 12 ms, inside the GUI's 16.67 ms, and no missed frame once past the start-up. Its p99 sits about 3.7 ms over the 8.33 ms grid: a frame is presented at its deadline plus its own work, so the 2 to 5 ms the UI thread's frame takes shows in present-to-present intervals, as it does under Cap60 (p99 19 to 21 ms on a 16.67 ms grid).
- **Uncapped** runs at 200 to 230 frames a second at this size with a p99 of 9.5 to 12 ms.
- `late` counts about half of Cap60's frames because its grid sits exactly on 16.667 ms; `missed` is the count that shows a hitch there.

pfx gui's own presented rate at larger windows, the same scene and method, uncapped, the window maximized on an output of that size, 60 s with VRR on then 60 s with it off:

| window | vrr | mode | intervals | mean fps | p50 ms | p99 ms | max ms | late | missed |
|---|---|---|---|---|---|---|---|---|---|
| 1280×720 | on | Mailbox | 13944 | 230 | 4.35 | 9.49 | 56.1 | 93 | 29 |
| 2560×1440 | on | Mailbox | 9906 | 160 | 5.71 | 12.36 | 57.8 | 23 | 8 |
| 2560×1440 | off | Fifo | 9516 | 154 | 6.13 | 12.73 | 25.9 | 22 | 1 |
| 3840×2160 | on | Mailbox | 4538 | 72 | 11.00 | 46.14 | 333.6 | 930 | 338 |
| 3840×2160 | off | Fifo | 4029 | 64 | 10.62 | 80.02 | 619.3 | 678 | 340 |

At 2560×1440 the window keeps the GUI rule (p99 12.4 to 12.7 ms, about 155 to 160 fps). At 3840×2160 it does not: the median frame is 11 ms, but about one interval in five or six is late and one in twelve misses, so the mean falls to 64 to 72 fps and the p99 to 46 to 80 ms. The VRR-on lines are the first setting of their sessions, so they carry the start-up too.

### Where a frame's time goes

The same rig and scene with the release build of this layer, 30 s a setting after the timeline starts, each setting its own session; the report's spreads split the UI thread's frame: `viewport` is the app's viewport frame (the live renderer's work and its wait for frames in flight), `paint` the egui pass and its submit, `acquire` the wait for the swapchain image, `call` the present call. Inside `pgpu` every job's GPU queue runs at low priority (`AMD_PRIORITY=-512`) behind other jobs' work; "desktop" runs unset it, the priority a window has on a desktop. Load average 3 to 9, the GPU mostly free:

| window | vrr | mode | GPU queue | fps | p50 ms | p99 ms | sd ms | acquire p99 | call p99 | viewport p50 / p99 | paint p99 | shown p99 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2560×1440 | on | Mailbox | desktop | 224 | 4.09 | 20.41 | 2.49 | 0.05 | 0.07 | 1.06 / 18.74 | 0.24 | 19.66 |
| 2560×1440 | on | Mailbox | low | 228 | 4.09 | 19.49 | 2.30 | 0.07 | 0.18 | 1.10 / 17.67 | 0.41 | 19.34 |
| 2560×1440 | off | Fifo | desktop | 226 | 4.12 | 19.35 | 2.33 | 0.04 | 0.06 | 1.08 / 18.20 | 0.22 | 19.01 |
| 2560×1440 | off | Fifo | low | 244 | 4.05 | 5.78 | 0.64 | 0.05 | 0.07 | 1.01 / 3.82 | 0.25 | 4.61 |
| 3840×2160 | on | Mailbox | desktop | 105 | 9.45 | 14.22 | 1.51 | 0.06 | 0.42 | 8.62 / 13.03 | 0.30 | 14.25 |
| 3840×2160 | on | Mailbox | low | 103 | 9.68 | 12.39 | 1.43 | 0.07 | 0.09 | 8.70 / 10.00 | 0.37 | 13.82 |
| 3840×2160 | off | Fifo | desktop | 105 | 9.46 | 11.48 | 1.38 | 0.06 | 0.08 | 8.68 / 10.13 | 0.29 | 11.73 |
| 3840×2160 | off | Fifo | low | 102 | 9.65 | 19.37 | 1.73 | 0.06 | 0.08 | 8.85 / 17.84 | 0.29 | 19.44 |

- **The acquire never stalls here.** Its p99 is 0.04 to 0.07 ms in every run and its maximum under 2.3 ms; the present call and the egui paint stay under half a millisecond at the 99th percentile. Shown intervals follow the present intervals within a few tenths of a millisecond, so nothing queues behind the compositor either.
- **The viewport is the frame.** At 3840×2160 it takes 8.6 to 8.9 ms of the 9.5 ms median frame: the live renderer's GPU work, waited for when the next frame needs its slot. At 2560×1440 its median is 1.1 ms, and its 99th percentile (4 to 19 ms) is the tail of the whole interval. The present layer adds under 1 ms a frame at either size; 120 fps at 4K is the renderer's to reach.
- **The GPU queue's priority** made no consistent difference with the GPU mostly free (the 1440p Fifo low-priority run is the best of the eight). Earlier runs the same day at load average 15 to 35, all at low priority, had p99 20 to 42 ms at 2560×1440 against 11.8 ms for a desktop-priority run at load 7.

The swapchain's frame latency, 2560×1440 uncapped, low GPU priority, load average 15 to 35, two runs each; `PFX_FRAME_LATENCY` sets it:

| mode | latency | fps | p50 ms | p99 ms | acquire p99 | acquire max |
|---|---|---|---|---|---|---|
| Mailbox | 1 (the default) | 110, 126 | 9.54, 5.98 | 20.61, 32.21 | 0.12, 0.11 | 1.53, 2.57 |
| Mailbox | 3 | 85, 98 | 9.95, 5.87 | 40.23, 35.44 | 0.07, 0.10 | 1.73, 1.05 |
| Fifo | 2 (the default) | 118, 82 | 5.26, 7.27 | 33.18, 37.49 | 0.07, 0.19 | 0.12, 2.28 |
| Fifo | 3 | 106, 84 | 5.90, 7.58 | 32.41, 42.35 | 0.09, 0.18 | 2.76, 2.12 |

A deeper queue gains nothing where the acquire never waits, and adds a frame of input latency (8.3 ms at 120 fps), so the defaults stay. Acquiring the image later is moot for the same reason: the editor shell already acquires after the UI and the viewport are built, right before the paint that needs the image. wgpu 30 has no non-blocking acquire (its timeout is a fixed second).

**Auto against Uncapped** at 3840×2160, desktop GPU priority, load average 5 to 9:

| vrr | frame rate | fps | p50 ms | p99 ms | max ms | sd ms | viewport p50 / p99 |
|---|---|---|---|---|---|---|---|
| on | uncapped | 105 | 9.51 | 11.31 | 28.8 | 1.43 | 8.70 / 10.26 |
| on | auto | 94 | 10.69 | 12.18 | 20.2 | 1.00 | 1.44 / 8.82 |
| off | uncapped | 104 | 9.49 | 13.45 | 28.0 | 1.53 | 8.63 / 12.52 |
| off | auto | 92 | 10.76 | 18.42 | 34.1 | 1.63 | 1.51 / 9.12 |

With VRR on, Auto holds 90 fps with a third less spread and a lower maximum than Uncapped, at a tenth fewer frames and a p99 0.9 ms higher; with VRR off it is worse on every count, as its probes up to 120 fail on a frame the GPU holds at about 105. Uncapped stays the default.

**UI thread priority**, 2560×1440, VRR on, uncapped, desktop GPU priority, three runs each (nice 0 to -10, which `RLIMIT_NICE` 31 allows here): p99 11.83, 20.53, 21.46 ms with it off and 12.37, 20.69, 20.99 ms with it on, at load average 6 to 21. It does not move the tail on this machine, so it stays opt-in.

**VRR asked but not in effect.** The rig's headless output stamps a frame when it is committed and reports a refresh of 0, so it shows frames on arrival the way variable refresh does. It is not the fixed-refresh case. Cap120 with VRR on gave p99 11.38 and 9.65 ms before this check and 11.48 and 9.84 ms with it. The case it fixes, a 120.000 fps grid beating against a fixed 119.985 fps refresh, needs a real fixed-refresh display.
