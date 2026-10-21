# The game runtime

A new game starts from `pfx new <game>`, which scaffolds its repo: `docs/projects.md`.

`pfx-game` (`crates/game`) is the one call a game's `main` makes. A game is its own crate in its own repo: it implements `pfx_play::Game` and hands it to `pfx_game::run`, and pfx opens the window, paces the frames, reads the input, plays the sound, keeps the fixed tick and draws the scene and the game's UI. Without it every game writes its own winit app, pacing, display settings and frame loop.

The crate carries nothing of any game: no names, values, assets or fonts. Its tests and its example use the engine's neutral fixtures (`crates/game/examples/empty/world`, a floor and a block, and the engine's OFL test font).

## The call

```rust
use pfx_game::{Config, Game, Input, Tick, World};

fn main() -> std::process::ExitCode {
    let config = Config::new("my-game", env!("CARGO_PKG_VERSION"), "content/first.pitofx")
        .font(include_bytes!("../content/fonts/ui.ttf"));
    pfx_game::launch(MyGame::new, config, |commands| {
        commands.add("run", "plays a seed headless and prints the result", run);
    })
}
```

`launch` is the launcher below; `pfx_game::run(game, config)` is the same without it, for a game with no commands. Either returns once the window closes or the game quits, after `Game::stop`: `run` gives `Ok(code)`, the game's exit code (0 when the window closed), or `Err(error)` once it has reported the error (below, "Quitting and failing"); `launch` turns that into the process's `ExitCode`.

`Config::new(name, version, scene)` names the game (what `help` and `version` print, and the window title unless `title` sets one), its version and the scene the world is built from. Before the scene loads, the window and `Headless` call `config.bind_text()`, so a scene's text can show `{name}` and `{version}` (`docs/scenes.md`, "Bound text"). `Config::flat(name, version)` is the same with no scene (`scene: None`), for a flat game (below). `Config::packed(name, version, bytes)` builds the world from a `.pitofx` the game holds in memory, usually `include_bytes!` of its packed scene, so the game ships as one binary: the scene's meshes, images and fonts resolve from those bytes and nothing is read from disk, a file the scene names and the bytes do not hold is refused, and with no project folder beside it the tunables and rigs start empty ([migrate-3.md](migrate-3.md), "Shipping one binary"). The builder methods set the rest:

| Method | What it sets | Default |
|---|---|---|
| `title(text)` | the window's title, apart from the launcher's name (`config.window_title()`) | the name |
| `mark(path)` | the game's mark, a PNG, shown before its name in the top-left header of `<game> edit`; a relative path is read from the project folder, and the picture is trimmed to its ink (its alpha, or the colour of its corner when it is opaque) and resampled to the header's height in device pixels (a game made by `pfx new --mark <png>` sets it to `content/marks/mark.png`, the picture its first scene shows on the title's plate) | no mark: the header shows the name alone |
| `wordmark(path)` | the game's wordmark, a PNG, shown after the mark in the same header in place of the typed name, trimmed and resampled to the header's height as the mark is; a relative path is read from the project folder | none: the header types the name |
| `crash(hook)` | `Fn(&str)`, called with the error when the game fails or the runtime stops on one, after it is printed to stderr | none |
| `settings(Settings)` | the settings used when the game keeps none (`Game::settings` returns `None`) | `Settings::default()` |
| `screen(ScreenPolicy)` | the aspect policy, the game's `[screen]` table (`docs/screens.md`) | 16:10 on a Deck, 16:9 elsewhere |
| `device(Device)` | the device, instead of detecting it | `screens::detect` (`PFX_DEVICE` overrides it) |
| `render_scale(f32)` | the 3D render size as a factor of the content rect | 1.0 |
| `budget(Budget)` | dynamic resolution against a GPU budget, through the renderer's own controller | off |
| `filter(Filter)` | the upscale filter, `Sharp` or `Nearest` (pixel art) | `Sharp` |
| `exposure(Exposure)` | the 3D frame's exposure: `Exposure::Auto { bias }` meters every frame, `Exposure::Fixed(f)` holds it; `Fixed(1.0)` is the exposure pfx gui's viewport draws with | `Auto { bias: 0.0 }` |
| `font(bytes)` | a font the UI's labels use; the first is the default, more can follow | none |
| `pads(Pads)` | `Pads::Gilrs`, `Pads::None` or `Pads::Backend(backend)`, Steam Input's included | `Gilrs` |
| `seed(u64)` | the session's seed and the frame seed | 0 |
| `layout(source)` | the `LayoutSource` keycap labels come from | `WindowsLayout` on Windows; elsewhere learned from key presses |
| `icons(Icons)` | the flat icons the UI's `Shape::Icon` draws use, baked by the game (`flat::Icons::bake`) and uploaded once | none |
| `sprites(SpriteImage)` | the sprite atlas the UI's image fills (`Draw::sprite`, `Fill::Image`) sample, uploaded once ([animation.md](animation.md)) | none |
| `environment(EnvironmentTexels)` | the linear equirect (up to 4096 a side) and intensity the UI's reflection dial samples, uploaded once | none |
| `sound(bool)` | whether to open the default audio device | on |
| `warm_budget(seconds)` | how long the warm-up may hold the first frame back (below) | 2 s (`WARM_BUDGET`) |
| `clear(Srgba)` | the window's ground from its first present until the game's first frame (below) | black |
| `build_date(Option<&str>)` | the game's build date, `YYYY-MM-DD`, which a timed pfx Pro key must cover to hide the splash ([splash.md](splash.md)); a `pfx new` game passes `option_env!("PFX_BUILD_DATE")` from its `build.rs` ([projects.md](projects.md), "The build date") | none: `SOURCE_DATE_EPOCH` as pfx-splash was compiled, if set |

A scene that is missing or does not load is refused before any window opens.

**A flat game.** With no scene the world is built from `pfx_load::scene::Scene::empty()` (also `Scene::default()`, so a game or test that needs an empty scene never writes a struct literal and a new field never breaks it), with no tunables or rigs read from disk. No 3D pass runs and nothing is staged: the framed view is the UI's clear colour (`Ui::clear`, black without one), then the game's paint, then its UI. The fixed tick, input, sound, pacing and settings run as in any game. `Headless` takes the same config.

## What it owns

**The window.** It creates one winit window in the game's own process, titled `config.window_title()`, plays the display settings' mode on it (`screens::Display`, `apply_display`: windowed, borderless or fullscreen, the stored size, monitor and windowed position), detects the Steam Deck, where the mode is always borderless fullscreen at the panel's size, and fits the window to the aspect policy every resize: `ScreenPolicy::fit` gives the layout, the content rect, the bars, the safe area and the UI scale. The pointer reaches the game in layout units through the bars (`Input::set_viewport(report.viewport())`).

**The first present.** The window is created hidden. As soon as its surface is configured, before the renderer, the painter, the driver or the warm-up are built, the runtime clears the surface to `Config::clear` (black without one) and presents it, then shows the window and presents the ground once more, so X11, Windows and macOS never show an empty black window while the game builds and warms; Wayland maps the window on that first present. The ground is presented again after any resize during the warm-up. A game that picks its ground from the system's light or dark preference reads `pfx_game::system_scheme()` before it builds its `Config` (below). Built but untested on X11, Windows and macOS (no window opens on this machine's desktop).

**The surface's configure.** Every configure of the window's surface (the first one, a resize, a lost or outdated surface, a VRR or present-mode change) runs only while the GPU is idle, so the frame never blocks on a GPU drain: the runtime first asks wgpu, without waiting, whether its queue is empty (`Device::poll(PollType::Poll)`), and only then configures, through `WindowTarget`'s validation error scope. While the GPU is still busy, or when wgpu refuses the configure anyway (`ConfigureSurfaceError::GpuWaitTimeout`, its answer when another thread submitted during its own wait, as a game's render worker does), the runtime skips that frame's draw (nothing is acquired or drawn on a surface that is not configured) and tries again on the next frame; the ticks, the sound and the pads keep running meanwhile. A render worker that leaves the GPU idle now and then needs nothing. One that keeps it busy without a break takes `world.configuring()`, a `Configuring` handle it clones into its thread, and holds its submits while `pending()` is true, so the configure finds the queue idle within a frame. A configure that keeps failing ends the run with a clear error once it has failed 8 times in a row over at least 2 seconds (`the window's surface: not configured after …`, saying whether the GPU never went idle or wgpu refused, with wgpu's error printed before it); one success resets the count. The scope catches validation refusals only: an out-of-memory or internal device error still reaches the device's uncaptured-error handler.

**Light or dark.** `pfx_game::system_scheme()` reads the system's preference, `Some(Scheme::Dark)`, `Some(Scheme::Light)` or `None` when the system states none, before any window exists: on Linux the XDG desktop portal's `org.freedesktop.appearance` `color-scheme` over the session bus (a small D-Bus client in pfx-game, about 1 to 5 ms, giving up after 500 ms), on Windows the `AppsUseLightTheme` registry value, on macOS `defaults read -g AppleInterfaceStyle`. In the game, `world.scheme()` holds it and follows its changes: on Linux a thread listens for the portal's `SettingChanged` signal and wakes the loop, on Windows and macOS the window's `theme()` and winit's `ThemeChanged` feed it. It starts `None` until the first answer arrives, normally before the first frame, so a game reads it each tick or in `ui` to pick its ground and palette. A headless run reports `None` until the test sets one: `run.set_scheme(Some(Scheme::Dark))` (or `run.feed(Event::Scheme(..))`). Windows and macOS are built but untested there.

**Pacing.** The settings' two switches, `pacing.vrr` and `pacing.frame_rate`, are pfx's window pacing (`docs/window.md`, "Pacing"): uncapped presents each frame as soon as it is ready, `cap60` and `cap120` hold 60 or 120 frames a second on the event loop's absolute deadline grid (locked to whole refreshes when VRR is off), and VRR picks the surface's present mode (`Present::preference(host, fullscreen)`; `pfx_game::present_preference(present, host, fullscreen)` is the same choice). Focus and occlusion never slow a frame; the loop pauses only while minimized, if `pause_when_minimized`. Both switches take effect live, with no restart: the pacer gets a new frame rate at once, and a VRR change reconfigures the surface before the next frame is drawn (`WindowTarget::follow`, which keeps only a mode the surface lists and falls back to Fifo, and on Wayland flips the surface's content-type hint). `Driver::take_present` hands a VRR switch over once; a headless run records it as `WindowCall::Present(preference)` for its `host` (Wayland by default). The loop never sleeps on its own: it hands winit `WaitUntil` the pacer's deadline, and an uncapped frame is paced by the swapchain alone. With `PFX_PACING=<file>` the window writes its pacing report there (`docs/window.md`, "Pacing report"). A frame after a pause starts the game clock again from that frame, so a minimized game resumes with no burst of catch-up ticks.

**Input.** The game's action map (`Game::actions`), the winit keyboard and mouse events (`pfx_input::winit::event`), and the pads: gilrs with Steam's precedence read from the environment (`SteamPrecedence::from_env`), or any `Backend` the config passes. The glyph style of the pad last used reaches the UI each frame (`UiFrame::glyph_style`).

**The pointer.** The game asks for the OS cursor from its world, in `start`, `tick` or `events`: `world.set_cursor(Cursor::Captured)` hides it and keeps it inside the window (a shooter's play), `Cursor::Hidden` hides it and leaves it free (a game that draws its own cursor), and `Cursor::Shown`, the default, gives it back (menus, a pause screen). The request stays until the game changes it. The window applies it after each frame's ticks, and gives the cursor back on its own while the window has lost focus or is minimized, taking the game's request again when focus returns, so a player who alt-tabs out of a shift is never left without a cursor. Captured asks the OS to confine the cursor (`CursorGrabMode::Confined`: Windows, X11, Wayland), and where it can't (macOS) it locks the cursor in place and moves the game's pointer itself from the mouse's raw motion (winit's `DeviceEvent::MouseMotion`, points scaled to pixels), clamped to the window. Either way the game reads the same thing: `input.pointer()` in window-mapped layout units, and `PointerMoved` events in its recording, so a replay needs no OS cursor at all. If the OS refuses both, the cursor stays free and the window prints why on stderr. The macOS lock path is built but untested on a Mac. A headless run has no cursor and ignores the request.

**Putting the pointer back.** A game that pauses into a menu shows the cursor, and the player's clicks move it; when play resumes, `world.warp_pointer([x, y])` puts it back where it was when play froze, so an avatar that chases the pointer does not lunge at the menu card. The point is in layout units, the same units `input.pointer()` reads, mapped through the bars to window pixels and clamped into the framed view. The next frame's first tick reads it: after the frame's ticks, the runtime sets Input's pointer to the point as a pointer that left and came back, so the jump is no motion, no anchor travel and no switch to the mouse for a player on a pad, and the window moves the OS cursor there once (`Window::set_cursor_position`; on the locked macOS path it moves only its own pointer). Where the OS refuses (Wayland, unless the pointer is locked), Input's pointer still moves and the window prints one line on stderr. A headless run updates Input's pointer and moves no cursor.

**Focus.** `world.focused()` says whether the player is looking at the game: true while the window has focus and is neither occluded nor minimized, false otherwise. The window feeds it from winit's `WindowEvent::Focused` and `Occluded` and its own minimized check, the same `Attention` events the frame pacer reads, so a game reads it every tick to pause its attract loop, duck its music or open its pause menu when the player alt-tabs away, and carry on when focus returns. It starts true. A headless run reports focused until the test says otherwise: `run.set_focused(false)` (or `run.feed(Event::Attention(..))` for occlusion and minimizing) reaches `world.focused()` (and a minimize the pacer's pause), so a game's pause path can be tested without a window. The editor's play-in-viewport reports the editor window's focus while the viewport plays.

**Rumble.** The game queues rumble from `tick` on its world: `world.rumble(Rumble::motors(low, high, ms))` plays on the active pad (only while a gamepad is the active device), and `world.rumble_pad(pad, rumble)` on one pad. After the tick the session hands the queue to its `Input`'s rumbler, and each step's motor commands (`Step::motors`) go to the pad backend's `set_motors` after the frame's ticks: gilrs force feedback, or through Steam Input's `TriggerVibration` for a pad Steam owns. The settings' `rumble` (0 to 100) scales every level. A replayed session sends no motor commands.

**Keycap labels.** The runtime keeps a `KeyLabels`. On Windows it refreshes it every frame from `WindowsLayout`, which catches a layout switch within a frame. On Linux and macOS it learns each label from the key presses (`pfx_input::winit::learn`), as `docs/input.md` says. `Config::layout` passes any other source. Prompts in the UI draw keycaps with these labels, so a French player sees A where the binding is the physical Q. `Driver::labels_mut` reaches it for `rename`.

**Sound.** The world's deterministic mixer (`world.sounds`) renders the audio of every tick at the device's rate and channel count; the runtime streams it to the default device through a PCM stream of 0.25 s. The settings' volumes are applied to that mixer, so what the player hears and what a headless run collects are the same mix. Without a device the game runs silent.

**The game's own sound.** A game that makes its own samples (a synthesiser, a tracker, a decoder of its own) opens a PCM stream on any of the world's mixer channels and pushes into it from its hooks:

```rust
fn start(&mut self, world: &mut World) {
    self.music = Some(world.sounds.stream(ChannelId::MUSIC, 2, 0.1, Level::default()));
}

fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
    let frames = world.audio_due();
    let samples = self.synth.render(frames, world.sounds.rate());
    self.music.as_ref().unwrap().push(&samples);
}
```

- `Sounds::stream(channel, channels, buffer_seconds, level)` returns a `PcmStream` on that channel (`MUSIC`, `EFFECTS`, `DIALOG` or a custom one), so the settings' volume for the channel, its mute, ducking and filters apply to it like any clip. The buffer is bounded: `push` takes at most `room()` frames of interleaved samples at `world.sounds.rate()` and returns how many it took; `queued()` and `capacity()` say the rest.
- `world.audio_due()` is the number of frames the mixer renders at the end of the current tick (800 at 48 kHz and 60 ticks a second; 0 in `start`). Pushing that many each tick keeps the stream fed with one tick of latency; pushing ahead fills the buffer.
- It is deterministic: the mixer pulls from the stream on the tick clock, never on the device's, so the same pushes give the same samples in a headless run (`Headless::audio`) and in the device's stream. A stream that runs dry fades out over 2 ms and fades back in when samples return.
- `Sounds::end_stream(stream, fade_seconds)` ends it; dropping the `PcmStream` ends it once what it queued has played.

**Frame stats and the dev HUD.** The renderer writes `PFX_FRAME_STATS` as `docs/frame-stats.md` says, and the session reports its `sim_ms` into the same handle. The window (and `Headless` with the GPU) writes one line for every frame it presents, the splash's included, with a `source`: `splash`, `ui` for a frame the renderer drew nothing in (a flat game, or a UI over the last 3D frame), `game` for one it drew in ("Presented frames" there). With the `dev-hud` feature, F3 toggles the HUD over the presented frame. Turn the feature on only in a dev build. The HUD draws into the window's own surface at its size (`PlaySession::draw_hud_on`, `Hud::draw_on`), top right of the window whatever the render scale.

**The fixed tick.** Each frame advances the `PlaySession` by the real seconds since the last frame, which runs the ticks that fell due at the scene's `[physics] rate` (at most 8 a frame) and calls `Game::frame` with the interpolation alpha, exactly as `docs/play.md` describes. `Game::try_frame` gets a `FrameTime { alpha, seconds, tick }`, and `UiFrame::seconds` carries the same real seconds beside `alpha`: the time since the last frame, 0 on the first frame and on the first frame after a pause, for animation that runs on the wall clock rather than the tick (a cursor blink, a UI tween). The ticks stay the only clock the simulation reads. `FrameTime::tick` is the whole `Tick` the session's clock gave this frame (`dt` scaled by the game speed and any hit-stop, `paced` without the dip, `ticks` run): a game steps its presentation with it, such as `effects.step(&time.tick)` for the flat pass's bursts, so effects slow and freeze with the session's own clock. A hit-stop comes from that clock too: `world.hit_stop(HitStop::new(floor, hold, recover))` from `tick` or `events` (reduced motion applied first, from `world.fx.comfort`), and `world.set_speed(speed)` for the game speed; `world.clock()` reads them back. Neither changes the ticks or what each tick sees, so recordings and replays are unaffected (effects.md, One game clock). A game's camera can follow a body or look through a character's eyes with the rigs in [camera-rigs.md](camera-rigs.md), driven each tick from its input.

**The frame.** In order: the bars or the backdrop, the framed view (the 3D frame, or a flat game's clear colour), the game's own paint (`Game::paint`, below), the game's UI and the dev HUD. The scene is drawn by the live renderer at the render size into a reused texture as large as the renderer's targets; `pfx_live::upscale` draws that rect into the window inside the content rect with the policy's bars, and the game's UI is drawn by the upscale's flat pass at the window's native size over it, so text and UI stay crisp at any 3D scale. The render size is the content rect times the render scale, or, with a budget, what dynamic resolution chose: the renderer's viewport inside targets allocated at the content size, so a scale step reallocates nothing and keeps the TAA history (`docs/screens.md`, "Viewport dynamic resolution").

**The colour path.** The renderer's `Rgba16Float` frame holds sRGB-encoded values (its finish chain ends with `Encode`), and the window's surface is sRGB, which encodes in hardware. The upscale decodes the frame once before writing it (`Compose::source_format`, `docs/screens.md`, "The colour path"), the bars are decoded the same way, and the UI's flat pass writes linear values, so everything reaches the window encoded once. At render scale 1 and a fixed exposure a game's window, the renderer's own `Rgba8UnormSrgb` output and pfx gui's viewport agree within 1/255 (`crates/editor/tests/colour.rs`, which renders one flat-backed neutral scene all three ways and also reads the editor's offscreen shot).

## Clipboard

A game copies and pastes text through its world, from `tick` or `events` (and reads it from `frame` or `ui`):

```rust
if input.action("copy").pressed {
    world.set_clipboard(&self.seed_code);
}
if input.action("paste").pressed && let Some(text) = world.clipboard_text() {
    self.name_field.insert(&text);
}
```

- `world.set_clipboard(text) -> bool` hands the text to the system clipboard. It returns at once: true when the clipboard took the request, false when there is none (a headless run, a build without the feature, a system that refused it), and it never panics.
- `world.clipboard_text() -> Option<String>` returns at once with the last text fetched from the system, or what the game set last, and asks for a fresh copy. It is `None` until the first answer lands, when the clipboard holds no text, and wherever there is no clipboard. The window fetches once when the game starts and again each time the window gains focus, so text the player copied in another app is normally there before the paste key is read. Call it when the game needs it (a paste key, a field gaining focus), not every frame: each call asks the system again.
- The clipboard's I/O never runs on the frame thread. A thread of its own (`pfx-clipboard`) owns the system clipboard and answers requests in order; queued sets collapse to the last one, and a fetch that was overtaken by a set is dropped, so a get after a set reads what was set. A system that blocks (a Wayland data-device roundtrip, a slow X11 selection owner) delays only that thread, and the window waits at most 250 ms for it on exit.
- The backend is the one the editor's shell uses through egui-winit: on Wayland, smithay-clipboard on winit's own display connection; on X11, Windows and macOS, arboard; under gamescope, whichever of the two matches the connection winit opened. Neither opens a window of its own. Steam Deck's gaming mode (gamescope) may offer no clipboard or refuse a set; a get then stays `None` and a set does nothing, quietly. Windows is built (and checked by the full gate) but untested; macOS is written but not built here; gamescope is untested.
- pfx-game's `clipboard` feature, on by default, brings arboard (MIT or Apache-2.0) and smithay-clipboard (MIT); with it off, or under `PFX_HEADLESS=1`, the world has no clipboard and both calls return false and `None`.
- The world reaches the clipboard through `pfx_play::Clipboard` (`set_text`, `get_text`, `refresh`); a host of its own attaches one with `world.attach_clipboard(Box::new(..))`. The clipboard is attached after `Game::start` returns, so it answers from the first tick on. The editor keeps its own clipboard in its egui shell, unchanged; a game playing in its viewport has no clipboard of its own.

## Links

A game opens a web page in the player's browser through its world, from `tick`, `events` or `ui`:

```rust
if input.action("store_page").pressed {
    self.opening = world.open_url("https://example.com/game").ok();
}
if let Some(Err(error)) = self.opening.as_mut().and_then(Opening::poll) {
    self.notice = Some(error.to_string());
}
```

- `world.open_url(url) -> Result<Opening, UrlError>` returns at once and never blocks the frame or panics. The link is checked first (`Url::parse`): only `http://` and `https://` links with a host open; every other scheme (`file:`, `javascript:`, `data:`, `mailto:`, `steam:`, custom ones) is `UrlError::Scheme`, and a link with a space, a control character, one of `\` `"` `<` `>` `` ` `` `{` `}` `|` `^`, a character outside ASCII, a user name before the host, no host or over 4096 bytes is `UrlError::Malformed`, so a game that opens a link from a mod, a save or the network can't open a file or run a script with it. The scheme and the host are lower-cased, so every opener finds the player's default browser. Percent-encode anything else. With no browser attached (a headless run, the editor's play) it is `UrlError::NoBrowser`.
- The `Opening` answers later: `poll()` is `None` until the opener answers, then `Some(&Ok(()))` or `Some(&Err(UrlError::Failed(why)))`, and keeps that answer. A game that doesn't care drops it. One link opens at a time: a link asked for while another is still opening answers `UrlError::Busy`, so a game that asks every frame by mistake starts one opener, not hundreds.
- The window attaches `SystemBrowser`, which starts the system's opener on a thread of its own: `xdg-open`, then `gio open`, `gnome-open` and `kde-open` on Linux and the BSDs, `/usr/bin/open --` on macOS, and on Windows `ShellExecuteExW` with the `open` verb, COM initialised on that thread and no dialog of its own (`SEE_MASK_FLAG_NO_UI`), so a missing handler is an error, never a prompt. It answers when the opener exits, or `Ok` when the opener is still running after 2 seconds (`xdg-open` with no desktop handler runs the browser itself, in the foreground), and the thread then waits for the opener to exit. The link is one argument, never passed through a shell. The link's checks and its routing are tested here; no test starts a real opener, so the launch itself is built but untested on Linux, macOS and Windows alike.
- Under Steam, with pfx-game's `steam` feature, `pfx_game::steam::SteamInput::browser()` is a `SteamBrowser`; pass it as `Config::browser(input.browser())` before the input goes to `Pads::Backend`. Its links open in the Steam overlay's web page (`SteamApi::web_page`, `ISteamFriends::ActivateGameOverlayToWebPage`), which is the only browser a player sees on a Steam Deck in gaming mode; the request waits for the next frame's pad poll, and when the overlay is off or Steam is down the link goes to the fallback browser instead, the system's unless `SteamInput::fallback(browser)` gives another (a test passes one that records). The link opens only when the input is polled: while the `SteamInput` is not the run's pads, or the loop is paused while minimized, it waits. One link waits at a time; another asked for before the poll answers `UrlError::Busy`.
- `Config::browser(browser)` replaces the system browser with any `pfx_play::Browser` (`open(&Url) -> Opening`; `Opening::pending()` gives the sender its answer goes to, `Opening::done(result)` an answer at once). A headless run attaches one only when its config has one, so a test records links without opening anything.

## The trait

`pfx_play::Game` gives every hook but `start` a default, so a game implements only the hooks it uses:

```rust
pub trait Game {
    fn actions(&self) -> Vec<ActionSpec> { Vec::new() }
    fn start(&mut self, world: &mut World);
    fn tick(&mut self, world: &mut World, tick: &Tick, input: &Input) {}
    fn try_tick(&mut self, world: &mut World, tick: &Tick, input: &Input) -> Result<(), GameError> { self.tick(world, tick, input); Ok(()) }
    fn events(&mut self, world: &mut World, events: &[WorldEvent]) {}
    fn animated(&mut self, world: &mut World, events: &[AnimationEvent]) {}
    fn frame(&mut self, world: &World, alpha: f32) {}
    fn try_frame(&mut self, world: &World, time: FrameTime) -> Result<(), GameError> { self.frame(world, time.alpha); Ok(()) }
    fn ui(&mut self, world: &World, frame: &UiFrame) -> Ui { Ui::default() }
    fn try_ui(&mut self, world: &World, frame: &UiFrame) -> Result<Ui, GameError> { Ok(self.ui(world, frame)) }
    fn warm(&mut self, warm: &mut Warm<'_>) -> Result<Warming, GameError> { Ok(Warming::Done) }
    fn paint(&mut self, renderer: &mut Renderer, target: &wgpu::TextureView, frame: &PaintFrame) -> Result<(), GameError> { Ok(()) }
    fn settings(&self) -> Option<&Settings> { None }
    fn stop(&mut self) {}
    fn modules(&mut self) -> Option<&mut dyn Reload> { None }
}
```

A game implements either the plain hook or its `try_` variant; the session calls only the `try_` ones, whose defaults call the plain ones. `GameError` is `Box<dyn Error + Send + Sync>`, so `?` works on any error and `Err("text".into())` on a string. `warm` and `paint` return it too, so a game uses one error type in every hook.

`modules` hands the game's gameplay modules to the runtime and the editor (see "Mods" below).

### Quitting and failing

- **Quit:** `world.quit(code)` from `tick` (a menu's Quit, a finished run). No tick runs after the one that asked, `frame` and `ui` are not called again, the window closes, `Game::stop` runs and the process exits with `code`.
- **Fail:** an `Err` from `try_tick`, `try_frame` or `try_ui`, or `world.fail(error)` from `tick`. The game stops the same way: nothing more runs, `Game::stop` runs, the error is printed to stderr as `<game>: <error>` and handed to the config's `crash` hook, and the process exits with 1. A failure after a quit replaces it; a quit after a failure does not.
- `world.exit()` (and `PlaySession::exit`, `Driver::exit`) is the request, an `Exit::Quit(code)` or `Exit::Failed(message)`; `Exit::code` is its exit code. A runtime error (the GPU, the display mode) ends the run the same way, reported to stderr and the crash hook.

### The UI

`ui` runs once a frame, after the ticks and `frame`, and returns the game's 2D UI for that frame:

- `Ui::draws`: flat-pass `Draw`s (`docs/flat.md`) in layout units;
- `Ui::labels`: `Label { text, family, size, weight, at, anchor, colour, elevation, id, line, wrap, height, vertical, transform, fallbacks }`, laid out by the runtime through the engine's shared glyph atlas at the screen's pixels per unit, so they are crisp at native size. Without a wrap width, `at` is the label's top left (or its top centre or top right, by `anchor`, measured on the widest line). The builders:
  - `line(height)`: the line height, `size × 1.25` by default (`line_height()`);
  - `wrap(width)`: lines wrap at `width` and `at` is the left edge of a box that wide; `anchor` aligns each line inside it;
  - `boxed([width, height], horizontal, vertical)`: a box with its top left at `at`; lines wrap at its width and align by `horizontal`, and the block sits at the box's top, middle or bottom by `vertical` (the block's height from the text engine, the offset added to the origin before placing, so the glyphs land exactly where a label placed there by hand would);
  - `transform(matrix)`: a flat `Matrix` about the label's `at`, applied to the placed glyphs as their draw's transform (`flat::rotation(angle)` turns the label about its own corner);
  - `fallbacks(["Noto Sans SC", …])`: the faces tried in order for characters the label's family lacks (`TextEngine::set_fallbacks` for the family at every weight, set when the list changes; a label naming none leaves its family's list as it is). Each face must be a config font;
- `Ui::prompts`: input prompts drawn from the input crate's glyph atlas, crisp at any size. `Prompt::action(name, at, height)` shows the controls bound to an action on the device the player last used (`Input::prompt`), side by side with `gap` between them. `Prompt::control(control, …)` shows one control, and `Prompt::glyph(glyph, …)` one exact glyph. Pads are drawn in the active glyph style (PlayStation or standard). Keys are keycaps with their label in the player's layout, laid out in the prompt's font (`.font(family, weight)`; a key prompt needs a config font). `.floor(size)` is the smallest label in layout units: a label that would shrink under it widens its keycap instead (`KeycapLabel::fit_with_floor`, `docs/input.md`). `at` is the top left of the first glyph's 48-unit box, and `height` is that box's height in layout units;
- `Ui::groups`: flat `Group`s (`ui.group(opacity)` returns the index a draw joins with `.group(index)`), so a stack fades as one;
- icon draws: a `Draw::new(Shape::Icon(icon))` in `Ui::draws` draws the config's icons;
- `Ui::curve` and `Ui::light`: the shadow curve and light of the UI's draws; no curve casts no shadow. The config's environment is what the reflection dial samples;
- `Ui::clear`: the framed view's colour in a flat game (`ui.clear(colour)`), an opaque sRGB colour; a game with a scene draws its 3D frame there instead. In a flat game the UI's draws are finished against it, so glows, soft edges and additive draws come out exactly as a flat pass cleared to it draws them (within 1/255; `docs/screens.md`, "The HUD's ground");
- `Ui::look`: a flat look for the frame's UI (`ui.look(&stack.frame(time))`, or `UiLook::still(look, tokens)`): the draws, labels and prompts go through `FlatLooks` as `render_flat_look` draws a board (`docs/flat-looks.md`), clipped to the frame like any UI. It is per frame, so a game runs its own `LookStack` and hands the frame it wants;
- `Ui::effects(&mut effects)`: composes the UI's draws and light through the game's `flat::effects::Effects` (`effects.compose`, `docs/effects.md`): shake moves every draw and turns the light with it, and bursts, rings and the flash join the draws. Call it after the draws it should carry; draws added later stay steady;
- `Ui::view`: a view of the whole UI board (`ui.view(View::about(centre).zoom(1.5).roll(0.1))`, or any 2D `Matrix`): zoom, pan and roll applied to the draws, labels and prompts with their shadows and effects, as if the finished board were transformed (`docs/flat.md`, "The view"). `None` by default, which draws exactly as before. The pointer stays in layout units; `flat::unview_point(&matrix, pointer)` maps it onto the viewed board. The UI is still clipped to the frame, and the backdrop does not move;
- `Ui::backdrop`: flat `Draw`s in window units (below), the game's background continued past the framed view (`ui.backdrop(draw)`). The runtime draws them only when the screen policy says `bars = "backdrop"`.

`UiFrame` says where to lay it out: `layout` (the layout's size in units), `safe` (the safe rect in layout units, `[x, y, width, height]`), `ui_scale` (1.4 on a Deck by default), `pixels_per_unit`, `window` (the window's size in pixels, the size to remember for a windowed mode), `deck`, `glyph_style`, `alpha` and `seconds` (the frame's real seconds). A label needs a font in the config; a frame with labels and no font is an error.

For the backdrop, `UiFrame` also carries `window_units` (the whole window in units at the layout's scale, so 3840×1080 for a 32:9 window on a 1080-unit layout), `content` (the framed view in those units, `[x, y, width, height]`; on a pillarboxed 32:9 window `[960, 0, 1920, 1080]`) and `backdrop` (whether the policy asked for one). A unit is the same size in both spaces, so a pattern drawn at `content`'s offset lines up with the same pattern drawn in layout units inside the frame.

**The backdrop.** On a window wider or taller than the game's aspects, the framed view keeps its aspect (16:9 on a desktop, 16:10 on a Deck by default) and everything playable, readable and clickable stays inside it, so a wide screen shows no more of the game. With `[screen] bars = "backdrop"` the runtime draws `Ui::backdrop` first, across the whole window, beneath the framed 3D view and the UI; the framed view then covers it inside the content rect, so the frame is pixel-identical to the same game's in a window of its own aspect. While the backdrop has nothing to draw, or without the policy key, the bars take the policy's colour as before. The backdrop's draws write no picking ids, the pointer over it reports `inside: false` with layout coordinates outside the layout, and the safe area stays inside the frame. Two rules hold whatever the policy: the game's UI (`Ui::draws`, labels and prompts) is clipped to the frame, so a draw laid out past the layout never spills into the backdrop or the bars; and a mouse button pressed with the pointer outside the frame fires no action bound to it until it is released, while `input.mouse_held(button)` and `input.mouse_outside(button)` still show the press to a game that wants it. Keys and pads fire wherever the pointer is.

### Painting

`Game::paint(renderer, target, frame)` runs every frame after the framed view (the 3D frame or the clear colour) and before the UI, on the same target the runtime draws into: the window's surface, or `Headless`'s offscreen target. A game with its own flat painter draws whatever it likes there, and the runtime's UI goes over it. `PaintFrame` says where:

| Field | Meaning |
|---|---|
| `format`, `output` | the target's format (sRGB: write linear values) and its size in pixels |
| `content`, `clip` | the framed view in pixels, `[x, y, width, height]`, and the scissor that keeps a draw inside it (`upscale::clip`) |
| `render` | the 3D render size, or `None` when no 3D pass ran |
| `layout`, `content_units`, `window_units`, `pixels_per_unit` | the layout's size, the frame and the window in its units, and the screen's pixels per unit |
| `frame`, `seed` | the runtime's frame number and seed |

The renderer is the runtime's own: `renderer.gpu()` gives the device and queue for the game's passes, and its `warm`, `warm_flat` and flat passes are the game's to use. Its output format is the 3D frame's `Rgba16Float`, so a game drawing a flat scene into the target uses its own `FlatPass`, or its own `Upscale` and `encode_overlay` (`docs/screens.md`), which is how the runtime draws its UI. Anything the game paints stays inside `clip` by its own scissor. A pass that reads the 3D frame's depth reads it reversed (near 1, far and sky 0) whatever the scene's `[live] reversed_depth`, and takes its distances from `pfx_live::depth::DEPTH_WGSL` (`docs/live-renderer.md`, "Reading the depth buffer from your own shader"). An error stops the run as a failing `try_` hook does: `Game::stop`, the error on stderr and to the crash hook, and a non-zero exit.

### Warm-up

`Game::warm(warm)` runs before the first visible frame (pfx-game's `Warmup`, one `pass` a turn of the loop, which the window and pfx gui's play share), with nothing presented until it is done. It returns `Warming::Done`, or `Warming::More` to be called again on the next turn of the loop (the window keeps answering its events between calls). The runtime stops calling it at `Done` or once `Config::warm_budget` has passed since the first call, and only then draws the first frame. The `Warm` handle:

- `glyphs(family, weight, sizes, chars)` prefills the UI's glyph atlas for those characters at each size and the screen's pixels per unit (`TextEngine::prefill`, `docs/text.md`) and uploads the pages, returning the cells it made, so labels in those faces draw on their first frame without rasterizing;
- `looks(&[look, …])` builds the UI's flat pass and every given look's pipelines at the window's format and size (`Upscale::warm_overlay`); pass every look the UI switches between;
- `renderer()` is the runtime's `Renderer`, for its own `warm(scenes)` and `warm_flat(format, size, looks)`; `None` in a headless run without the GPU;
- `format()` is the texture format the frame is presented in, the window surface's own format once it is chosen (or the offscreen target's in a headless run with the GPU; `None` without the GPU), so a game's canvas, pipelines or flat passes warmed here are built for the format the first frame draws into and never rebuilt at the first paint;
- `size()` is the window's inner size in pixels, `[width, height]`, the size the first frame's output has (in a headless run with the GPU the stand-in window's, as built or as the last `Event::Resized` set it; `None` without the GPU), so a game sizes its canvas, targets and glyphs here once instead of guessing and redoing them at the first paint. When the window's size changes while the splash still plays after the warm-up finished (a fullscreen window settling to the monitor's size), the runtime runs the warm-up again from pass 0 at the new size, still under the splash, so a game that warms by `size()` has warmed at the size its first frame draws at;
- `pass()`, `elapsed()`, `budget()` and `left()`: which call this is (from 0), the seconds since the first, the budget, and what is left of it.

Pacing ignores the warm-up: the game clock starts at the first frame, so the first frame advances by nothing and no ticks pile up for the time spent warming. Headless, each call counts `frame_cost_ns` toward the budget without moving the fake clock.

### One frame for the window and the editor

`Painter::draw(driver, output, format)` is the whole frame, and `Headless` and pfx gui's play (`docs/editor.md`, "Play mode") call it; the window calls its two halves, `prepare` and `compose` (below), so a game draws the same wherever it runs. The editor plays through the same `Driver` too, built with `Driver::new` around its own `PlaySession` (hooked for pgpu and recording its input), and two calls exist for it: `Driver::step()` runs one tick of a paused session (`PlaySession::step`) and lays out the UI again, as `update` does after its ticks; and `Driver::start_clock(now_ns)` sets the time the next `update` measures from, so the editor's first frame after the warm-up advances by its own pass instead of nothing. `Painter::wait_for_frames(true)` makes the painter wait for each 3D frame instead of pipelining it, so auto exposure's meter is read every frame; the editor turns it on, a game's window leaves it off. Otherwise the runtime's renderer keeps up to `FRAMES_IN_FLIGHT` (2) frames on the GPU (`Renderer::set_frames_in_flight`, `docs/flat.md`, "Frames in flight"), waiting only when the oldest is still running, so the CPU builds the next frame while the GPU draws this one, in a window and in `Headless` alike.

### Frames on demand

The window draws a frame only when something changed. The loop still wakes for every tick: the session advances, the game's `frame` and `ui` run and the 3D frame is built, so the simulation, its ticks and its recording are the same as before; only the drawing and the present are skipped. `Painter::prepare(driver, forced)` decides, each tick:

- **The 3D frame** is drawn when the renderer's fingerprint of it differs from the last drawn one (`Renderer::needs_redraw`, `docs/live-renderer.md`, "Frames on demand"): a moved or posed instance, a material, a light, the sun, the camera, surface text, a resize, any look setter, and the work that changes on its own: particles, steam, creatures, liquid, trees, wind, a running tape glitch, a sky still settling and auto exposure while it moves. After a change it draws `pfx_live::demand::SETTLE` (8, one TAA jitter cycle) more frames, so TAA and the temporal reflections converge before it rests. Time alone changes nothing. A skipped 3D frame submits nothing; the last one stays in its texture.
- **The window's frame** (the bars, the 3D frame, `Game::paint`, the UI and the dev HUD) is composed and presented when the 3D frame was drawn, when the UI differs from the last one presented (`Ui`, the prompts' parts, the keycap labels a prompt shows, the screen report or the window's size), while the dev HUD is shown and once after it is hidden, when the game asked for it, and when the window needs it: the first frame, a reconfigured surface (a resize, a display mode, a VRR switch), a window the system asks to redraw, uncovered or restored, and the frame after one whose surface could not be acquired.

A game whose `paint` draws something the runtime can't see (its own animated pass) calls `world.request_frame()` from `start`, `tick`, `events`, `frame` or `ui` (it takes `&World`) for each frame it wants presented. A game that wants every frame drawn, as before, sets `[authoring.pfx] redraw = "always"` in its `project.toml` (the scaffold's `config()` reads it, `Project::redraws_always`) or `Config::redraw(Redraw::Always)`; the renderer's watch is then off and every frame the pacer starts is drawn and presented.

A skipped frame presents nothing, so the compositor keeps showing the last image, and VRR and the frame rate hold for the frames that are drawn. An uncapped frame is paced by the swapchain, which a skipped frame never reaches, so after one the loop rests until the game's next tick is due (`WaitUntil` the time `Driver::next_tick_ns` reads from the session's clock: the part of a tick still to run, at the game speed, never more than one tick of the scene's `[physics] rate`) instead of spinning, so the frame that tick changes starts when it falls due, not a whole tick after the skipped frame; resting a whole tick from the skipped frame used to overshoot the next tick by up to a refresh, so a tick that fell due just after a present was drawn and presented at once, the next refresh skipped, and the frame after it rested past its tick: two presents within a millisecond, then one of three refreshes (measured with the `boot` example at 2560x1440 and 120 Hz over 9 s: 28 to 39 such pairs and a UI p99 of 25.3 to 26.3 ms before, 0 to 2 and 18.1 to 20.8 ms after, with the p50 on the 60 Hz tick at 16.67 ms); any window event (a key, the pointer, a resize, focus, a scheme change) ends the rest at once, so input is answered on the next turn of the loop, and pads are read at every tick as before. The pacing report counts only presented frames: a skipped frame marks the gap as idle (`Pacing::idle`), as a pause does, so on-demand gaps never read as late frames. Sound is streamed every tick, drawn or not. An empty game made by `pfx new` idles at the numbers in [measured.md](measured.md#start-up-idle-and-memory).

### Prompts for a game's own painter

`pfx_game::prompt` lays prompts out exactly as the runtime does, so a game that paints its own prompts draws the same quads:

- `prompt::parts(input, &PromptOf)` resolves a prompt to its `Part`s on the active device (`Part::Glyph(glyph)` or `Part::Key(key)`);
- `prompt::layout(parts, at, size, &Style, &mut Sources) -> PromptQuads`: `Style { family, weight, gap, floor, pixels_per_unit }` (`Style::of(&prompt, pixels_per_unit)`), and `Sources { atlas, text, labels }`, the game's own `GlyphAtlas`, `TextEngine` (needed for keys) and `KeyLabels`;
- `PromptQuads { glyphs, labels, width }`: the glyph quads on the input glyph atlas, the keycap labels' quads by text page, and the prompt's width in layout units. The game draws `glyphs` from its `GpuAtlas` of `atlas.atlas` and `labels` from its text pages, in one colour.

The runtime's painter calls the same function each frame (`Painter::prompt_quads()` holds the last frame's), and `prompt_quads_laid_out_by_a_game_equal_the_runtimes_own` compares them.

### Settings

`Settings` is a serde value the game keeps and stores wherever it keeps its settings (a TOML file, Steam Cloud, its save). The engine writes no file.

```toml
rumble = 100

[display]
mode = "windowed"
size = { width = 1600, height = 900 }

[pacing]
vrr = true
frame_rate = "uncapped"
pause_when_minimized = true

[audio]
master = 1.0
muted = false
music = 0.8
effects = 1.0
dialog = 1.0
custom = { 3 = 0.5 }

[binds.keyboard]
jump = [{ Press = { Key = "Space" } }]
```

| Part | What it holds | Default |
|---|---|---|
| `display` | `screens::DisplaySettings`: the mode, the windowed size, the monitor's name, the windowed position on that monitor | borderless, 1600×900, no position |
| `pacing` | `vrr` (adaptive sync on or off) and `frame_rate` (`"cap60"`, `"cap120"` or `"uncapped"`), the two switches every pfx window takes (`docs/window.md`), and `pause_when_minimized`; the `focused` and `background` keys of older files are ignored | on, uncapped, on |
| `audio` | the master volume and mute, `music`, `effects` and `dialog`, and `custom` channels by index (`ChannelId::custom`) | all 1, unmuted |
| `binds` | a `pfx_input::BindingMap`, fitted to the game's actions | the actions' defaults |
| `rumble` | the rumble intensity, 0 to 100 | 100 |

Every key may be left out; a missing one takes its default. The runtime reads the game's settings once at start (or the config's, when `settings` returns `None`) and compares them after every frame's ticks. A change applies at once: new caps to the pacer, VRR to the pacer and the surface's present mode, the rumble intensity to the rumbler, volumes to the mixer (after the mixer's 10 ms ramp), binds to the action map, and a display change to the window. A game changes its settings in `tick` or `frame`, from its own menu, and saves them itself.

**What the driver learned.** The player changes the display without the game: dragging the window's size, moving it, taking it to another monitor. The driver keeps all of it in its `Display`, and every frame hands it to the world before the ticks: `world.display()` is a `DisplaySettings` with the mode, the windowed size the player dragged to, the monitor's name and the windowed position on that monitor (`docs/screens.md`). To keep it for the next start, a game copies it into its own settings:

```rust
if *world.display() != self.settings.display {
    self.settings.display = world.display().clone();
    self.save();
}
```

Settings whose `display` equals what the driver learned are taken as they are: the runtime re-applies the window mode only when the game's display differs from both what it applied last and what it learned, so storing the learned state never resizes, moves or re-fullscreens the window. The display is not part of the simulation: a recording does not carry it, so a replay reads whatever the replaying window reports.

## The launcher

`pfx_game::launch(factory, config, |commands| …)` runs the game with no arguments. With the `tools` feature it is also the game's command line:

| Command | What it does |
|---|---|
| (none) | runs the game |
| `help`, `--help`, `-h` | lists the commands, each with its one-line description |
| `version`, `--version`, `-V` | prints `<name> <version>` |
| `edit [args]` | opens the game in pfx's editor, when the game set the edit hook (below) |
| `<command> [args]` | a command the game registered |

A game registers its own commands by name with a description: `commands.add("run", "plays a seed headless", run)`, where `run` is `Fn(&[String]) -> R` and gets the arguments after its name. `R` is any `Outcome`:

| The command returns | The process exits with |
|---|---|
| `()` | 0 |
| `ExitCode` or `u8` | that code |
| `Ok(value)` of a `Result<T, E>` where `T` is one of the above | `value`'s code |
| `Err(error)` where `E: Display` (`String`, `&str`, `io::Error`, `Box<dyn Error>` …) | 1, after `<game> <command>: <error>` on stderr |
| `Err(CommandError::usage(message))` | 2, a usage error, printed as a refusal (below) |
| `Err(CommandError::new(code, message))` | `code`, after `<game> <command>: <message>` |

A closure that returns `Ok(..)` names its error type (`Ok::<_, String>(())`) or returns `()` or an `ExitCode` instead, since Rust cannot guess an error it never sees.

**Refusals.** Input the launcher cannot parse gets the answer every PITO MD command line gives, clap's, on stderr with exit 2: an unknown command is `error: unrecognized subcommand 'tui'`, an unknown flag `error: unexpected argument '--x' found`, then a blank line, `Usage: <game> [COMMAND]`, a blank line and `For more information, try '--help'.`; never the whole help, which prints only for `help`, `--help` and `-h`. `edit` with no hook set is an unrecognized subcommand. A registered command's usage error (`CommandError::usage("unexpected argument 'x' found")`, in clap's words) prints the same way, under `Usage: <game> <command> <shape>`, where `commands.usage(command, shape)` gives the shape (`commands.usage("run", "<SEED> [--frames <N>]")`) and `[ARGS]...` stands in until one is given. An empty shape says the command (or `edit`) takes no arguments, and the launcher refuses any before it runs: `my-game edit extra` answers `error: unexpected argument 'extra' found` under `Usage: my-game edit`. A shape for a name that is neither registered nor `edit` is refused with the other registration mistakes. In a terminal the refusal is coloured as clap colours it (`error:` bold red, the offending token yellow, `Usage:` bold and underlined, the command path and `--help` bold), and plain when stderr is not a terminal, `NO_COLOR` is set, `TERM` is `dumb` or `CLICOLOR` is 0 (`CLICOLOR_FORCE` forces it; Windows prints it plain). `pfx_core::cli` writes it, for the launcher and for `pfx` alike: `Refusal::new(message).under(usage)`, `render(color)`, `print()`, `color()`, and clap's wordings (`unexpected`, `unrecognized`, `no_command`, `repeated`, `conflict`, `missing_value`, `required`, `invalid`, `invalid_choice`, `with_value`). After a missing or invalid value clap prints no `Usage:` line, and neither does `Refusal`. A refusal after a successful parse stays one line, `<game> <command>: <message>`, exit 1.

**Arguments that are not a command.** `commands.unmatched(hook)` takes what an unknown first argument would otherwise refuse: global flags, or an error in the game's own format. `hook(args)` gets every argument and returns `Ok(Unmatched::Run)` to run the game (after the hook has noted a flag, for example in state the factory reads), `Ok(Unmatched::Command(rest))` to read `rest` as the command line instead (a flag stripped before a command; the hook is not asked again, and an empty `rest` runs the game), `Ok(Unmatched::Exit(code))` once it has answered itself (a JSON error on stdout), or `Err(CommandError)`, printed as `<game>: <message>` with its code, in whatever format the game chose. Without a hook, an unknown first argument is refused as above. Built-in and registered commands never reach it, and with no arguments the game runs without registering anything. A name is one word, not starting with `-`, and `edit`, `help` and `version` are taken; a refused or duplicate name exits with 2 before anything runs. The game is built (`factory()`) only when it runs or is edited, never for `help` or a command.

**A game's editor is the game's.** In a window `<game> edit` opens (`edit_project` or `run_project`) everything that names the program names the game, for as long as the window is open, even after another file is opened in it: the window's title (`<name> · <file>`), the GPU queue's label and the slow-frame lines on stderr use the config's `name`; the status bar shows its version line (`<name> v<version>`); the header shows the config's `mark` and `wordmark` or none, never pfx's; and the window keeps the game's theme. `crates/workbench` tests that such a window paints no pfx colour, no `pfx` text and no pfx mark.

**The edit hook.** `commands.edit(about, hook)` sets what `edit` does: `hook(factory, args)` gets a `Factory` (`Rc<dyn Fn() -> Box<dyn Game>>`) over the constructor `launch` was given, and the arguments after `edit`. The launcher builds no game for it: the editor calls the factory once for each play, so every F5 starts a fresh game and a game needs no placeholder. The hook builds the config it hands the editor itself (a game made by `pfx new` calls its own `config(&project)` again). The hook returns `Result<(), E>` for any `E` that becomes a `CommandError` (a `String` does, with code 1): a usage error (`CommandError::usage`) is refused in clap's words under `Usage: <game> edit <shape>` with exit 2, as a registered command's is, and any other error prints `<game> edit: <error>` and exits with its code. That game sets the hook inside the `tools` feature to `pfx_workbench::edit_project(project.root(), factory, config, args)`, with `commands.usage("edit", pfx_workbench::EDIT_USAGE)`. With no arguments it opens the project in pfx's window where F5 plays the game's own logic (`docs/projects.md`), as `pfx_workbench::run_project` does. With `--shot <png>` or `--clip <dir> --tape <file>` it draws the same window offscreen, never opening one: `--size WxH` (1586x992 unless given), `--scale S` (the pixels per point, 1 to 4: `--size 2560x1440 --scale 1.6` lays the window out at 1600x900) and, for a clip, `--fps 30|60|120`. A shot draws one frame, or the last frame of a tape when `--tape` names one; a clip writes the tape's frames as `frame-00000.png`, `frame-00001.png` and on into its folder (an encoder such as pcod turns them into a video). Either works on a copy of the project, made fresh in the project's `tmp/pfx-edit/` from everything but `target/`, `tmp/` and hidden folders, with the config's scene and mark taken from the copy, so a filmed edit never saves into the game's files. The tape's directing steps act on the game's scene: `select <object>` selects it (one object), `orbit <degrees> <seconds>` and `dolly <metres> <seconds>` move the editor's view, and `frame <object> [seconds]` frames it without changing the selection; `view`, `camera`, `stage` and `reveal` act on a recipe and are refused there. Until a hook is set, `edit` is refused as an unrecognized subcommand and `help` does not list it. `pfx_game::factory(make)` makes a `Factory` from any constructor, and `launcher::edit(name, hook, factory, args)` runs a hook as `launch` does (an `Err` is printed as `<game> edit: <error>` and exits with 1). Because the factory may be called more than once, `launch` takes an `Fn() -> G + 'static` (a constructor such as `MyGame::new`, or a closure that clones what it needs), no longer an `FnOnce`.

## The tools feature and a Steam build

`tools` carries every command and whatever only they pull in. A game forwards it and turns it on in its defaults:

```toml
[features]
default = ["tools"]
tools = ["pfx-game/tools"]
```

- **Without `tools`**, `launch` ignores its arguments, never calls the registering closure and runs the game. A command's code is never compiled into that build: `launcher::command_code_links_only_with_the_tools_feature` checks the test binary for a registered command's function and its text, absent without the feature and present with it (skipped on Windows).
- **The Steam build** is the same binary built with `cargo build --release --no-default-features`: the game alone, arguments ignored. The tools build is a release build too; tools is not debug.
- **`pfx steam stage` refuses a tools build.** With `tools`, the launcher carries `launcher::TOOLS_MARKER` (`"pfx-game tools build: the launcher's commands and pfx's editor are linked in"`), kept in the binary by a `black_box` in `decide`; without it, the text is never compiled in. `pfx steam stage` reads every staged file that starts as an executable (ELF, PE, Mach-O) and refuses, before it writes anything, one that carries the marker, naming the file and saying to build with `--no-default-features` (`docs/steam.md`). The link check above asserts the marker is in the tools build and absent without it.
- **On Windows** the Steam build must be a GUI-subsystem exe, with no console window. The game's `main.rs` starts with:

  ```rust
  #![cfg_attr(not(feature = "tools"), windows_subsystem = "windows")]
  ```

  The tools build keeps its console, so `help` prints.

**Steam Input.** With pfx-game's `steam` feature, `pfx_game::steam::SteamInput::new(steam, pads)` is a pad backend over any `pfx_steam::SteamApi` (the game's own `pfx_steam::Steam`, or `Fake` in tests) and `SteamPads`, beside another backend (`.beside(Box::new(gilrs))`). Pass it as `Pads::Backend(Box::new(input))`. It runs Steam's callbacks each frame before reading the pads; the game takes the callbacks' events from the `SteamEvents` handle (`input.events()`, taken before the backend is handed over). Steam's precedence over gilrs follows `docs/input.md`.

## Mods

A game that takes mods or scripts hosts them with `pfx-mod`: sandboxed WASM components with no WASI, metered by fuel and capped in memory, tables and stack, behind a WIT world the game declares in its own repository, loaded from `mods/<id>/` and fingerprinted so a run's record says which mods ran. `docs/mods.md` has the security model, the limits, the world, loading, fingerprints and determinism.

A game's own logic can live in gameplay modules the same way, built from its own repo: pfx-game's `modules` feature re-exports pfx-mod as `pfx_game::modules`, whose `Modules` loads the modules `project.toml` names, calls them each tick through the game's world, and swaps a rebuilt one at a tick boundary (`docs/mods.md`, "Gameplay modules"). A game hands them to the runtime and the editor through `Game::modules()`; the runtime prints their notices (a module started, swapped, refused or stopped) to stderr each frame, and the editor logs them and reloads rebuilt modules while the game plays (`docs/projects.md`, "Live reload").

## Headless runs

`pfx_game::Headless` runs the same loop with no window: a `FakeClock` the run advances, a stand-in window (`StandIn`, a `WindowOps` that records the calls the display mode makes), and optionally the GPU, drawing into an offscreen `Rgba8UnormSrgb` target.

```rust
let mut run = Headless::new(MyGame::new(), config, Size { width: 1920, height: 1080 })?;
run.feed(Event::Attention(Attention::Focused(false)));
let count = run.run_for(1.0)?;
```

- `Headless::new` runs the CPU half alone (the session, the pacer, the settings, the UI); `with_gpu(game, config, gpu, size)` draws every frame too, and keeps pgpu's manners through `pfx_gpu::pace::Pace` (a turn between frames, 60 fps of wall time at most).
- `frame()` waits on the fake clock to the pacer's next deadline and runs one frame; `run_for(seconds)` runs every frame due in that span and returns the frames and ticks it ran. Each frame costs `frame_cost_ns` of fake time (1 ms by default), which an uncapped run is paced by.
- `audio` holds every sample the world rendered, `drawn` the last frame's render and output sizes (`render` is `[0, 0]` when no 3D pass ran) and the glyph cells it uploaded (`uploads`), `readback()` the target's pixels, and `window.calls` what the display mode asked of the window (and `WindowCall::Present` for a live VRR switch). `monitors` is the stand-in desk the display mode is planned against: one monitor called `stand-in` at the window's size, which a test replaces to place a window elsewhere.
- `on_demand = true` makes each frame `prepare` and `compose` as the window does, instead of `draw`: `composed` counts the frames composed (with `on_demand` off, every frame drawn), and `drawn` stays the last composed one.
- `exited()` says the game asked to quit or failed; no frame runs after it. `finish()` stops the run as the window would: `Game::stop`, then `Ok(code)`, or `Err(error)` reported to stderr and the crash hook.
- `warm()` runs the game's warm-up now and returns the calls it made (`warm_passes`); the first frame runs it otherwise. `painter()` reaches the GPU painter: its text engine, glyph pages, last prompt quads and upscale.
- A `Pads::Backend` is polled each frame; `Pads::Gilrs` is never opened headless.
- There is no clipboard: `world.set_clipboard` returns false and `world.clipboard_text()` `None`.

## Shot adapters

`pfx render` (stills and clips) and `pfx bake` start their adapter and every child render process headless: they remove `WAYLAND_DISPLAY`, `WAYLAND_SOCKET`, `DISPLAY` and `XAUTHORITY` from its environment and set `PFX_HEADLESS=1` (`pfx_run::queue::headless`, `pfx_gpu::HEADLESS_VAR`). While `PFX_HEADLESS` is set to anything but empty or `0`, the engine's window paths refuse with an error naming it: `pfx_game::run` and `pfx_game::launch` playing the game, `Gpu::window*` making a surface, and the editor's window. `pfx_gpu::refuse_window(what)` is the same check for an app's own window code.

So a game's adapter is its own binary that never calls `pfx_game::launch` or `pfx_game::run`. It implements `pfx::adapter::Adapter` and serves it, and draws through `Headless` on a surface-less GPU (`Gpu::headless` makes the instance without a display handle and asks for an adapter with no compatible surface):

```rust
fn open(&mut self, shot: &str, fixtures: &Path, scale: f32, app: &Value) -> Result<Opened, String> {
    let gpu = pollster::block_on(pfx_gpu::Gpu::headless())?;
    self.run = Some(pfx_game::Headless::with_gpu(MyGame::for_shot(shot, fixtures), config, gpu, size)?);
}
fn advance(&mut self, to: f64) -> Result<(), String> { run.run_for(to - self.at)?; self.at = to; Ok(()) }
fn frame(&mut self, _: Option<[f32; 3]>, _: bool) -> Result<Image, String> {
    Ok(Image::from_rgba8(size.width, size.height, &run.readback()?))
}
fn main() { pfx::adapter::serve(&mut MyAdapter::default()).unwrap() }
```

## Tests

- `crates/game/src/tests.rs` (headless, in the gate): frames per second at cap60, cap120 and uncapped with the tick held at 60 per second, focus loss and occlusion keeping the rate, `world.focused()` true in a headless run until `set_focused(false)` or a minimize, the pause while minimized with no catch-up burst, the rest after a skipped frame ending when the next tick is due at any game speed (`Driver::next_tick_ns`), the interpolation alpha reaching `frame` and `ui` and advancing by the frame's share of a tick, a volume change in the settings changing the mix (per channel, and muted), a bind change remapping an action and the defaults coming back, display and pacing changes reaching the stand-in window and the pacer, the Deck's fixed mode, the UI frame following the aspect fit and the safe area, the pointer through the bars, the settings' TOML and JSON, and a missing scene refused.
- `crates/game/src/tests_host.rs` (in the gate, continued): a window frame opened as `splash` while the splash plays and as `ui` after it; the warm-up running again under the splash only when the window changed size after it finished. `crates/play/src/paint.rs` (unit): `Warm::size()` giving the warmer's size, and `None` from a warmer without one and from `NoWarmer`.
- `crates/game/src/tests_flat.rs` (headless, in the gate): a config with no scene (`Config::flat` and `scene: None`) running at its rate with its clear colour and an empty world, the warm-up called before the first UI with its passes and elapsed seconds and the first frames paced as a game with none, `Warm::format()` and `Warm::size()` `None` without the GPU, and the budget stopping a warm-up that never finishes. `Scene::empty()` is checked in `pfx-load`, `KeycapLabel`'s floor and the widened cap's pieces in `crates/input/tests/layout.rs`, and the new `Label`, `Ui` and `UiLook` builders and `Ui::effects` in `pfx-play`.
- The gate (`bin/gate`, fast and full) also runs `cargo test -p pfx-game --lib --features tools,steam`.
- `crates/play/src/tests/url.rs` (in the gate): `Url::parse` taking http and https links with a host and refusing every other scheme (`file:`, `javascript:`, `data:`, `mailto:`, `steam:` and the rest), a space, a control character, a bidi override, a character outside ASCII, a backslash, a quote and the other characters a URI forbids unencoded, a user name before the host, no host and an over-long link; the scheme and host lower-cased; a refused link never reaching the browser; an `Opening` keeping its answer, and failing when its browser goes away unanswered. `crates/steam/src/tests_invite.rs` checks `web_page` on `Fake`: other schemes and a quote rejected, the overlay off answering `Ok(false)`.
- The launcher (in the gate, in both builds): no arguments run the game without registering, and without `tools` every argument is ignored; with `tools`, help's exact text, `version`, a command with its arguments, its error, an unknown command or flag refused in clap's exact words, a usage shape and an `edit` that takes no arguments, refused names and the edit hook, which gets its arguments and a factory: deciding builds no game, and each call of the factory builds a fresh one (three calls, three games, each stopped on its own).
- `crates/game/src/tests_host.rs` (headless, in the gate): a quit at tick 30 stopping the ticks and the frame hook, running `Game::stop` and exiting with its code; an error from `try_tick`, `try_frame`, `try_ui` and `world.fail` each stopping at its tick, running `Game::stop`, reaching the crash hook and exiting non-zero; the real seconds of a frame at cap60 and an uncapped 30 fps in `try_frame` and `UiFrame`, 0 after a pause; the window title; the dragged size, monitor and position reaching `world.display()`, stored by the game with no window call and a real change still applied; the windowed position restored on its monitor at the next start and left to the desktop when the monitor is gone; VRR and the frame rate switched live, only a VRR switch asking the surface for a new preference, cap60 locked to two refreshes of 144 Hz and cap120 to one with VRR off, and cap60 to 60 with it on, and the present choices on a Wayland surface; a width-anchored policy giving a 1920×1200 layout at 16:10 and 1920×1080 at 16:9; a game's sine pushed into the music channel at 800 frames a tick, identical across two runs, halved by the music volume, and bounded by its buffer; and a surface configure that keeps failing retried each frame without an error until it has failed 8 times in a row over 2 seconds, then a clear error that says whether the GPU stayed busy or wgpu refused, with one success resetting the count.
- The launcher's codes (in the gate, with `tools`): a command returning an `ExitCode`, a `u8`, `Ok(ExitCode)`, a usage error (2), an `io::Error` (1) and a `CommandError` with its own code; the unmatched hook running the game, rewriting the command line once, answering with its own code and printing its own error, never asked for a built-in or registered command.
- There is no GPU test of the live present-mode switch: wgpu makes no surface without a window, and an offscreen target has no present mode. The decision (which preference, when, and what each surface's list resolves to) is the CPU test above; `WindowTarget::set_present` is the call it feeds.
- `crates/game/src/tests_input.rs` (headless, in the gate): a rumble the game queues reaching the pad backend with its two motors and stopping after its time, the setting halving it and 0 silencing it, nothing sent while the keyboard is active; prompts resolving to the active device's controls in its glyph style and back to the keycap, labels following a layout switch and a rename; with `steam`, Steam Input's pads rumbling through `TriggerVibration`, a link opening in the overlay on the next poll, and with the overlay off going to the fallback browser, a second link meanwhile answering `Busy`.
- `crates/game/src/gpu_paint_tests.rs` (GPU, ignored): a flat game with no scene painting a card through `Game::paint` with its own `Upscale`, the UI's card drawn over it and the clear colour around, with the paint frame's sizes, in a 16:9 and a pillarboxed window; a wrapped, centred, two-face label in a box pixel-identical to the same label placed by hand at the measured offset, with its ink centred, on two lines, the fallback face drawn and a turned label turning about its corner; a warm-up of two passes filling the glyphs and a look's pipelines at the target's format and the window's size (`Warm::format()`, `Warm::size()`), after which the first frame uploads no glyph and makes no pipeline; and the prompt quads a game lays out equal to the runtime's, with a floored keycap widened past 160 and drawn to its right edge; and a glow, a radial additive spark over its tail and an additive disc in the UI over a dark `Ui::clear`, within 1/255 of the same draws through a flat pass cleared to that ground.
- `crates/game/src/gpu_tests.rs` (GPU, ignored): the loop drawing the neutral world and a UI card and label into an offscreen target at cap60 and an uncapped 120 fps, with the frames and ticks counted; the card at native size over the 3D world and the label's ink; a wide window's bars with the 3D at half scale and the UI still native; a 32:9 window with a striped backdrop showing it in both side regions, the frame pixel-identical to a 16:9 window's, a UI bar laid out past both sides of the layout clipped to the frame, the pointer over the sides outside, a click there firing no mouse-bound action while a click inside does, and black bars with the frame unchanged when the backdrop is empty or the policy does not ask for it; dynamic resolution dropping the render size under a tight budget; a keycap labelled in the player's layout, a PlayStation glyph, an icon, a half-opacity group with no darker overlap and the environment's colour on a bevel; with `dev-hud`, F3 showing the HUD top right of the window and hiding it again; one frame-stats line for every frame drawn, numbered without gaps, `game` for the frames the renderer drew the scene in and `ui` for a UI change over the unchanged scene and for every frame of a flat game; and `Warm::size()` giving the window's size after an `Event::Resized`.

pfx gui's play runs the same painter and driver; its tests are `crates/editor/tests/play.rs` (`docs/editor.md`, "Tests").

```
cargo test -q -p pfx-game --lib
cargo test -q -p pfx-game --lib --features tools,steam
cargo test -q -p pfx-game --lib --no-run
cargo test -q -p pfx-game --lib --features dev-hud --no-run
pgpu run --class clip --as pfx -- cargo test -q -p pfx-game --lib --features dev-hud gpu_ -- --ignored --test-threads=1
pgpu run --class clip --as pfx -- cargo test -q -p pfx-editor --test colour -- --ignored --test-threads=1
```

## The example

`crates/game/examples/empty` is an empty world with a neutral block and a flat UI label, in a few lines of `main`, with one registered command:

```
cargo build --release -p pfx-game --example empty --features tools
target/release/examples/empty help
target/release/examples/empty hello
cargo build --release -p pfx-game --example empty
```

Running it with no arguments opens a window, so it is for a developer at the desk; agents build it and test the same loop headless.

`crates/game/examples/boot` is a generated boot to measure a game's start: the splash (shown whatever the licence), then a flat ident for 1.5 s, then an animated title, for `BOOT_SECONDS` seconds (6 by default), with `Warm::size()` printed for each warm-up pass on exit. `BOOT_RENDERER=1` makes it build a second renderer on another thread from its first warm-up step, as a game that loads in the background does. Run it with `PFX_FRAME_STATS=<path>` and split the lines by `source` (`docs/frame-stats.md`); like `empty`, it opens a window, so agents run it only inside a private headless compositor.
