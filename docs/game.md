# The game runtime

A new game starts from `pfx new <game>`, which scaffolds its repo: `docs/projects.md`.

`pfx-game` (`crates/game`) is the one call a game's `main` makes. A game is its own crate in its own repo: it implements `pfx_play::Game` and hands it to `pfx_game::run`, and pfx opens the window, paces the frames, reads the input, plays the sound, keeps the fixed tick and draws the scene and the game's UI. Without it every game writes its own winit app, pacing, display settings and frame loop.

The crate carries nothing of any game: no names, values, assets or fonts. Its tests and its example use the engine's neutral fixtures (`crates/game/examples/empty/world`, a floor and a block, and the engine's OFL test font).

## The call

```rust
use pfx_game::{Config, Game, Input, Tick, World};

fn main() -> std::process::ExitCode {
    let config = Config::new("my-game", env!("CARGO_PKG_VERSION"), "content/first.scene.toml")
        .font(include_bytes!("../content/fonts/ui.ttf"));
    pfx_game::launch(MyGame::new, config, |commands| {
        commands.add("run", "plays a seed headless and prints the result", run);
    })
}
```

`launch` is the launcher below; `pfx_game::run(game, config)` is the same without it, for a game with no commands. Either returns once the window closes or the game quits, after `Game::stop`: `run` gives `Ok(code)`, the game's exit code (0 when the window closed), or `Err(error)` once it has reported the error (below, "Quitting and failing"); `launch` turns that into the process's `ExitCode`.

`Config::new(name, version, scene)` names the game (what `help` and `version` print, and the window title unless `title` sets one), its version and the scene the world is built from. Before the scene loads, the window and `Headless` call `config.bind_text()`, so a scene's text can show `{name}` and `{version}` (`docs/scenes.md`, "Bound text"). `Config::flat(name, version)` is the same with no scene (`scene: None`), for a flat game (below). The builder methods set the rest:

| Method | What it sets | Default |
|---|---|---|
| `title(text)` | the window's title, apart from the launcher's name (`config.window_title()`) | the name |
| `crash(hook)` | `Fn(&str)`, called with the error when the game fails or the runtime stops on one, after it is printed to stderr | none |
| `settings(Settings)` | the settings used when the game keeps none (`Game::settings` returns `None`) | `Settings::default()` |
| `screen(ScreenPolicy)` | the aspect policy, the game's `[screen]` table (`docs/screens.md`) | 16:10 on a Deck, 16:9 elsewhere |
| `device(Device)` | the device, instead of detecting it | `screens::detect` (`PFX_DEVICE` overrides it) |
| `render_scale(f32)` | the 3D render size as a factor of the content rect | 1.0 |
| `budget(Budget)` | dynamic resolution against a GPU budget, through the renderer's own controller | off |
| `filter(Filter)` | the upscale filter, `Sharp` or `Nearest` (pixel art) | `Sharp` |
| `exposure(Exposure)` | the 3D frame's exposure: `Exposure::Auto { bias }` meters every frame, `Exposure::Fixed(f)` holds it; `Fixed(1.0)` is the exposure pfx edit's viewport draws with | `Auto { bias: 0.0 }` |
| `font(bytes)` | a font the UI's labels use; the first is the default, more can follow | none |
| `pads(Pads)` | `Pads::Gilrs`, `Pads::None` or `Pads::Backend(backend)`, Steam Input's included | `Gilrs` |
| `seed(u64)` | the session's seed and the frame seed | 0 |
| `layout(source)` | the `LayoutSource` keycap labels come from | `WindowsLayout` on Windows; elsewhere learned from key presses |
| `icons(Icons)` | the flat icons the UI's `Shape::Icon` draws use, baked by the game (`flat::Icons::bake`) and uploaded once | none |
| `sprites(SpriteImage)` | the sprite atlas the UI's image fills (`Draw::sprite`, `Fill::Image`) sample, uploaded once ([animation.md](animation.md)) | none |
| `environment(EnvironmentTexels)` | the linear equirect (up to 4096 a side) and intensity the UI's reflection dial samples, uploaded once | none |
| `sound(bool)` | whether to open the default audio device | on |
| `warm_budget(seconds)` | how long the warm-up may hold the first frame back (below) | 2 s (`WARM_BUDGET`) |

A scene that is missing or does not load is refused before any window opens.

**A flat game.** With no scene the world is built from `pfx_load::scene::Scene::empty()` (also `Scene::default()`, so a game or test that needs an empty scene never writes a struct literal and a new field never breaks it), with no tunables or rigs read from disk. No 3D pass runs and nothing is staged: the framed view is the UI's clear colour (`Ui::clear`, black without one), then the game's paint, then its UI. The fixed tick, input, sound, pacing and settings run as in any game. `Headless` takes the same config.

## What it owns

**The window.** It creates one winit window in the game's own process, titled `config.window_title()`, plays the display settings' mode on it (`screens::Display`, `apply_display`: windowed, borderless or fullscreen, the stored size, monitor and windowed position), detects the Steam Deck, where the mode is always borderless fullscreen at the panel's size, and fits the window to the aspect policy every resize: `ScreenPolicy::fit` gives the layout, the content rect, the bars, the safe area and the UI scale. The pointer reaches the game in layout units through the bars (`Input::set_viewport(report.viewport())`).

**Pacing.** A `FramePacer` with the settings' focused and background caps, 30, 60, 90, 120 or uncapped each, and pausing while minimized. Focus and occlusion come from winit's `Focused` and `Occluded`; minimized is read from the window after every event. With `pacing.vrr` on, the cap is held to the monitor's refresh less 3 fps and the surface asks for `PresentPreference::vrr(host)`; otherwise it asks for `PresentPreference::game()` (`pfx_game::present_preference(vrr, host)` is that choice). Toggling `vrr` in the settings takes effect live, with no restart: the pacer gets the new limit at once, and before the frame is drawn the runtime reconfigures the surface with the other preference (`WindowTarget::set_present`, which keeps only a mode the surface lists and falls back to Fifo). `Driver::take_present` hands that switch over once; a headless run records it as `WindowCall::Present(preference)` for its `host` (Wayland by default). The loop never sleeps on its own: it hands winit `WaitUntil` the pacer's deadline. A frame after a pause starts the game clock again from that frame, so a minimized game resumes with no burst of catch-up ticks.

**Input.** The game's action map (`Game::actions`), the winit keyboard and mouse events (`pfx_input::winit::event`), and the pads: gilrs with Steam's precedence read from the environment (`SteamPrecedence::from_env`), or any `Backend` the config passes. The glyph style of the pad last used reaches the UI each frame (`UiFrame::glyph_style`).

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

**Frame stats and the dev HUD.** The renderer writes `PFX_FRAME_STATS` as `docs/frame-stats.md` says, and the session reports its `sim_ms` into the same handle. With the `dev-hud` feature, F3 toggles the HUD over the presented frame. Turn the feature on only in a dev build. The HUD draws into the window's own surface at its size (`PlaySession::draw_hud_on`, `Hud::draw_on`), top right of the window whatever the render scale.

**The fixed tick.** Each frame advances the `PlaySession` by the real seconds since the last frame, which runs the ticks that fell due at the scene's `[physics] rate` (at most 8 a frame) and calls `Game::frame` with the interpolation alpha, exactly as `docs/play.md` describes. `Game::try_frame` gets a `FrameTime { alpha, seconds }`, and `UiFrame::seconds` carries the same real seconds beside `alpha`: the time since the last frame, 0 on the first frame and on the first frame after a pause, for animation that runs on the wall clock rather than the tick (a cursor blink, a UI tween). The ticks stay the only clock the simulation reads. A game's camera can follow a body or look through a character's eyes with the rigs in [camera-rigs.md](camera-rigs.md), driven each tick from its input.

**The frame.** In order: the bars or the backdrop, the framed view (the 3D frame, or a flat game's clear colour), the game's own paint (`Game::paint`, below), the game's UI and the dev HUD. The scene is drawn by the live renderer at the render size into a reused texture as large as the renderer's targets; `pfx_live::upscale` draws that rect into the window inside the content rect with the policy's bars, and the game's UI is drawn by the upscale's flat pass at the window's native size over it, so text and UI stay crisp at any 3D scale. The render size is the content rect times the render scale, or, with a budget, what dynamic resolution chose: the renderer's viewport inside targets allocated at the content size, so a scale step reallocates nothing and keeps the TAA history (`docs/screens.md`, "Viewport dynamic resolution").

**The colour path.** The renderer's `Rgba16Float` frame holds sRGB-encoded values (its finish chain ends with `Encode`), and the window's surface is sRGB, which encodes in hardware. The upscale decodes the frame once before writing it (`Compose::source_format`, `docs/screens.md`, "The colour path"), the bars are decoded the same way, and the UI's flat pass writes linear values, so everything reaches the window encoded once. At render scale 1 and a fixed exposure a game's window, the renderer's own `Rgba8UnormSrgb` output and pfx edit's viewport agree within 1/255 (`crates/editor/tests/colour.rs`, which renders one flat-backed neutral scene all three ways and also reads the editor's offscreen shot).

## The trait

`pfx_play::Game` grew hooks with defaults (`ui`, `warm`, `paint`, `settings` and `modules`), so `PlaySession`, `SceneGame` and `pfx edit` are unchanged:

```rust
pub trait Game {
    fn actions(&self) -> Vec<ActionSpec> { Vec::new() }
    fn start(&mut self, world: &mut World);
    fn tick(&mut self, world: &mut World, tick: &Tick, input: &Input) {}
    fn try_tick(&mut self, world: &mut World, tick: &Tick, input: &Input) -> Result<(), GameError> { self.tick(world, tick, input); Ok(()) }
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
- `Ui::clear`: the framed view's colour in a flat game (`ui.clear(colour)`), an opaque sRGB colour; a game with a scene draws its 3D frame there instead;
- `Ui::look`: a flat look for the frame's UI (`ui.look(&stack.frame(time))`, or `UiLook::still(look, tokens)`): the draws, labels and prompts go through `FlatLooks` as `render_flat_look` draws a board (`docs/flat-looks.md`), clipped to the frame like any UI. It is per frame, so a game runs its own `LookStack` and hands the frame it wants;
- `Ui::effects(&mut effects)`: composes the UI's draws and light through the game's `flat::effects::Effects` (`effects.compose`, `docs/effects.md`): shake moves every draw and turns the light with it, and bursts, rings and the flash join the draws. Call it after the draws it should carry; draws added later stay steady;
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

The renderer is the runtime's own: `renderer.gpu()` gives the device and queue for the game's passes, and its `warm`, `warm_flat` and flat passes are the game's to use. Its output format is the 3D frame's `Rgba16Float`, so a game drawing a flat scene into the target uses its own `FlatPass`, or its own `Upscale` and `encode_overlay` (`docs/screens.md`), which is how the runtime draws its UI. Anything the game paints stays inside `clip` by its own scissor. An error stops the run as a failing `try_` hook does: `Game::stop`, the error on stderr and to the crash hook, and a non-zero exit.

### Warm-up

`Game::warm(warm)` runs before the first visible frame (pfx-game's `Warmup`, one `pass` a turn of the loop, which the window and pfx edit's play share), with nothing presented until it is done. It returns `Warming::Done`, or `Warming::More` to be called again on the next turn of the loop (the window keeps answering its events between calls). The runtime stops calling it at `Done` or once `Config::warm_budget` has passed since the first call, and only then draws the first frame. The `Warm` handle:

- `glyphs(family, weight, sizes, chars)` prefills the UI's glyph atlas for those characters at each size and the screen's pixels per unit (`TextEngine::prefill`, `docs/text.md`) and uploads the pages, returning the cells it made, so labels in those faces draw on their first frame without rasterizing;
- `looks(&[look, …])` builds the UI's flat pass and every given look's pipelines at the window's format and size (`Upscale::warm_overlay`); pass every look the UI switches between;
- `renderer()` is the runtime's `Renderer`, for its own `warm(scenes)` and `warm_flat(format, size, looks)`; `None` in a headless run without the GPU;
- `pass()`, `elapsed()`, `budget()` and `left()`: which call this is (from 0), the seconds since the first, the budget, and what is left of it.

Pacing ignores the warm-up: the game clock starts at the first frame, so the first frame advances by nothing and no ticks pile up for the time spent warming. Headless, each call counts `frame_cost_ns` toward the budget without moving the fake clock.

### One frame for the window and the editor

`Painter::draw(driver, output, format)` is the whole frame, and the window, `Headless` and pfx edit's play (`docs/editor.md`, "Play mode") all call it, so a game draws the same wherever it runs. The editor plays through the same `Driver` too, built with `Driver::new` around its own `PlaySession` (hooked for pgpu and recording its input), and two calls exist for it: `Driver::step()` runs one tick of a paused session (`PlaySession::step`) and lays out the UI again, as `update` does after its ticks; and `Driver::start_clock(now_ns)` sets the time the next `update` measures from, so the editor's first frame after the warm-up advances by its own pass instead of nothing. `Painter::wait_for_frames(true)` makes the painter wait for each 3D frame (`Renderer::render`) instead of pipelining it (`Renderer::submit`), so auto exposure's meter is read every frame; the editor turns it on, a game's window leaves it off.

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
focused = "120"
background = "30"
vrr = true
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
| `pacing` | `focused` and `background` caps (`"30"`, `"60"`, `"90"`, `"120"`, `"uncapped"`), `vrr`, `pause_when_minimized` | 60, 30, off, on |
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
| `edit [args]` | opens the game in pfx's editor, once block B sets the hook |
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

**Refusals.** Input the launcher cannot parse gets the answer every PITO command line gives, clap's, on stderr with exit 2: an unknown command is `error: unrecognized subcommand 'tui'`, an unknown flag `error: unexpected argument '--x' found`, then a blank line, `Usage: <game> [COMMAND]`, a blank line and `For more information, try '--help'.`; never the whole help, which prints only for `help`, `--help` and `-h`. `edit` with no hook set is an unrecognized subcommand. A registered command's usage error (`CommandError::usage("unexpected argument 'x' found")`, in clap's words) prints the same way, under `Usage: <game> <command> <shape>`, where `commands.usage(command, shape)` gives the shape (`commands.usage("run", "<SEED> [--frames <N>]")`) and `[ARGS]...` stands in until one is given. An empty shape says the command (or `edit`) takes no arguments, and the launcher refuses any before it runs: `my-game edit extra` answers `error: unexpected argument 'extra' found` under `Usage: my-game edit`. A shape for a name that is neither registered nor `edit` is refused with the other registration mistakes. In a terminal the refusal is coloured as clap colours it (`error:` bold red, the offending token yellow, `Usage:` bold and underlined, the command path and `--help` bold), and plain when stderr is not a terminal, `NO_COLOR` is set, `TERM` is `dumb` or `CLICOLOR` is 0 (`CLICOLOR_FORCE` forces it; Windows prints it plain). `pfx_core::cli` writes it, for the launcher and for `pfx` alike: `Refusal::new(message).under(usage)`, `render(color)`, `print()`, `color()`, and clap's wordings (`unexpected`, `unrecognized`, `no_command`, `repeated`, `conflict`, `missing_value`, `required`, `invalid`, `invalid_choice`, `with_value`). After a missing or invalid value clap prints no `Usage:` line, and neither does `Refusal`. A refusal after a successful parse stays one line, `<game> <command>: <message>`, exit 1.

**Arguments that are not a command.** `commands.unmatched(hook)` takes what an unknown first argument would otherwise refuse: global flags, or an error in the game's own format. `hook(args)` gets every argument and returns `Ok(Unmatched::Run)` to run the game (after the hook has noted a flag, for example in state the factory reads), `Ok(Unmatched::Command(rest))` to read `rest` as the command line instead (a flag stripped before a command; the hook is not asked again, and an empty `rest` runs the game), `Ok(Unmatched::Exit(code))` once it has answered itself (a JSON error on stdout), or `Err(CommandError)`, printed as `<game>: <message>` with its code, in whatever format the game chose. Without a hook, an unknown first argument is refused as above. Built-in and registered commands never reach it, and with no arguments the game runs without registering anything. A name is one word, not starting with `-`, and `edit`, `help` and `version` are taken; a refused or duplicate name exits with 2 before anything runs. The game is built (`factory()`) only when it runs or is edited, never for `help` or a command.

**The edit hook.** `commands.edit(about, hook)` sets what `edit` does: `hook(factory, args)` gets a `Factory` (`Rc<dyn Fn() -> Box<dyn Game>>`) over the constructor `launch` was given, and the arguments after `edit`. The launcher builds no game for it: the editor calls the factory once for each play, so every F5 starts a fresh game and a game needs no placeholder. The hook builds the config it hands the editor itself (a game made by `pfx new` calls its own `config(&project)` again). That game sets the hook inside the `tools` feature to `pfx_editor::run_project(project.root(), factory, config)`, which opens the project in pfx's editor where F5 plays the game's own logic (`docs/projects.md`), with `commands.usage("edit", "")` since it takes no arguments; until a hook is set, `edit` is refused as an unrecognized subcommand and `help` does not list it. `pfx_game::factory(make)` makes a `Factory` from any constructor, and `launcher::edit(name, hook, factory, args)` runs a hook as `launch` does (an `Err` is printed as `<game> edit: <error>` and exits with 1). Because the factory may be called more than once, `launch` takes an `Fn() -> G + 'static` (a constructor such as `MyGame::new`, or a closure that clones what it needs), no longer an `FnOnce`.

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
- `exited()` says the game asked to quit or failed; no frame runs after it. `finish()` stops the run as the window would: `Game::stop`, then `Ok(code)`, or `Err(error)` reported to stderr and the crash hook.
- `warm()` runs the game's warm-up now and returns the calls it made (`warm_passes`); the first frame runs it otherwise. `painter()` reaches the GPU painter: its text engine, glyph pages, last prompt quads and upscale.
- A `Pads::Backend` is polled each frame; `Pads::Gilrs` is never opened headless.

## Tests

- `crates/game/src/tests.rs` (headless, in the gate): frames per second at each cap with the tick held at 60 per second, the background cap on focus loss and occlusion, the pause while minimized with no catch-up burst, the interpolation alpha reaching `frame` and `ui` and advancing by the frame's share of a tick, a volume change in the settings changing the mix (per channel, and muted), a bind change remapping an action and the defaults coming back, display and pacing changes reaching the stand-in window and the pacer, the Deck's fixed mode, the UI frame following the aspect fit and the safe area, the pointer through the bars, the settings' TOML and JSON, and a missing scene refused.
- `crates/game/src/tests_flat.rs` (headless, in the gate): a config with no scene (`Config::flat` and `scene: None`) running at its rate with its clear colour and an empty world, the warm-up called before the first UI with its passes and elapsed seconds and the first frames paced as a game with none, and the budget stopping a warm-up that never finishes. `Scene::empty()` is checked in `pfx-load`, `KeycapLabel`'s floor and the widened cap's pieces in `crates/input/tests/layout.rs`, and the new `Label`, `Ui` and `UiLook` builders and `Ui::effects` in `pfx-play`.
- The gates (`dev/gate.sh`, fast and full) also run `cargo test -q -p pfx-game --lib --features tools,steam`.
- The launcher (in the gate, in both builds): no arguments run the game without registering, and without `tools` every argument is ignored; with `tools`, help's exact text, `version`, a command with its arguments, its error, an unknown command or flag refused in clap's exact words, a usage shape and an `edit` that takes no arguments, refused names and the edit hook, which gets its arguments and a factory: deciding builds no game, and each call of the factory builds a fresh one (three calls, three games, each stopped on its own).
- `crates/game/src/tests_host.rs` (headless, in the gate): a quit at tick 30 stopping the ticks and the frame hook, running `Game::stop` and exiting with its code; an error from `try_tick`, `try_frame`, `try_ui` and `world.fail` each stopping at its tick, running `Game::stop`, reaching the crash hook and exiting non-zero; the real seconds of a frame at 60 and 30 fps in `try_frame` and `UiFrame`, 0 after a pause; the window title; the dragged size, monitor and position reaching `world.display()`, stored by the game with no window call and a real change still applied; the windowed position restored on its monitor at the next start and left to the desktop when the monitor is gone; VRR toggled live asking the surface for `vrr(host)` and back for `game()` once each, and the present choices on Wayland and X11 surfaces; a width-anchored policy giving a 1920×1200 layout at 16:10 and 1920×1080 at 16:9; a game's sine pushed into the music channel at 800 frames a tick, identical across two runs, halved by the music volume, and bounded by its buffer.
- The launcher's codes (in the gate, with `tools`): a command returning an `ExitCode`, a `u8`, `Ok(ExitCode)`, a usage error (2), an `io::Error` (1) and a `CommandError` with its own code; the unmatched hook running the game, rewriting the command line once, answering with its own code and printing its own error, never asked for a built-in or registered command.
- There is no GPU test of the live present-mode switch: wgpu makes no surface without a window, and an offscreen target has no present mode. The decision (which preference, when, and what each surface's list resolves to) is the CPU test above; `WindowTarget::set_present` is the call it feeds.
- `crates/game/src/tests_input.rs` (headless, in the gate): a rumble the game queues reaching the pad backend with its two motors and stopping after its time, the setting halving it and 0 silencing it, nothing sent while the keyboard is active; prompts resolving to the active device's controls in its glyph style and back to the keycap, labels following a layout switch and a rename; with `steam`, Steam Input's pads rumbling through `TriggerVibration`.
- `crates/game/src/gpu_paint_tests.rs` (GPU, ignored): a flat game with no scene painting a card through `Game::paint` with its own `Upscale`, the UI's card drawn over it and the clear colour around, with the paint frame's sizes, in a 16:9 and a pillarboxed window; a wrapped, centred, two-face label in a box pixel-identical to the same label placed by hand at the measured offset, with its ink centred, on two lines, the fallback face drawn and a turned label turning about its corner; a warm-up of two passes filling the glyphs and a look's pipelines, after which the first frame uploads no glyph and makes no pipeline; and the prompt quads a game lays out equal to the runtime's, with a floored keycap widened past 160 and drawn to its right edge.
- `crates/game/src/gpu_tests.rs` (GPU, ignored): the loop drawing the neutral world and a UI card and label into an offscreen target at 30, 60 and 120 fps, with the frames and ticks counted and the background cap on focus loss; the card at native size over the 3D world and the label's ink; a wide window's bars with the 3D at half scale and the UI still native; a 32:9 window with a striped backdrop showing it in both side regions, the frame pixel-identical to a 16:9 window's, a UI bar laid out past both sides of the layout clipped to the frame, the pointer over the sides outside, a click there firing no mouse-bound action while a click inside does, and black bars with the frame unchanged when the backdrop is empty or the policy does not ask for it; dynamic resolution dropping the render size under a tight budget; a keycap labelled in the player's layout, a PlayStation glyph, an icon, a half-opacity group with no darker overlap and the environment's colour on a bevel; with `dev-hud`, F3 showing the HUD top right of the window and hiding it again.

pfx edit's play runs the same painter and driver; its tests are `crates/editor/tests/play.rs` (`docs/editor.md`, "Tests").

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
