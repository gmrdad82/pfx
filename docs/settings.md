# Settings

pfx keeps one set of settings for its own windows and one shared settings component for games. The settings are VRR, the frame rate, the frame stats overlay, the quality mode, one switch for each costly live effect, and the licence. `pfx gui` and a game's `<game> edit` show them in one panel. A game built with `pfx new` shows the same settings in its own menu, and `pfx settings` reads and sets them from a shell. Every change applies at once, with no restart, and is kept.

The panel follows the structure editor preferences usually have: Blender's Preferences, Godot's Editor Settings and DaVinci Resolve's Preferences all put a list of pages on the left, the selected page's grouped settings on the right, and apply most changes live. pfx's panel does the same with three pages: Display, Quality and Licence. As Resolve shows its edition in the main window, the licence badge stays in the window's top bar.

## The panel

Open it with `ctrl ,`, with the gear button in the top bar, or by clicking the licence badge beside it, which opens the Licence page. `esc` or Close shuts it. It fades in and rises into place over 0.18 s, eased out. The switches slide with the same easing.

- **Display:** VRR on or off (on by default), the frame rate (60 fps cap, 120 fps cap, Uncapped or Auto), and the frame stats overlay. The frame stats row, `F3` and the `stats` chip under the viewport are one switch, kept as `[display] stats`. The `vrr` and frame-rate chips under the viewport stay as shortcuts to the same two settings (`docs/editor-window.md`).
- **Quality:** Best appearance or Best performance, and the **Effects** group with one switch for each costly pass the live renderer runs. A pass that Best performance is holding off shows "held off" beside its switch.
- **Licence:** the badge (Free, Indie or Pro, with the licensee and the paid-through date; the top bar shows the covered-through date), the key's details, and Unlock.

"Reset this page" puts the page's settings back to their defaults. Until the first measurement has run, the frame rate and the quality stay unset (see "The first measurement").

## Frame rate

| Value | What it does |
|---|---|
| `60` | holds the window to 60 frames a second |
| `120` | holds the window to 120 frames a second |
| `uncapped` | presents each frame as soon as it is ready |
| `auto` | aims at 120 and settles to a steady lower rate when the scene can't hold it |

Auto is the window's own pacing (`docs/window.md`); a game's menu offers it too.

## Quality

- **Best appearance:** the viewport renders at native resolution with every effect that is switched on.
- **Best performance:** the renderer holds the 8.33 ms frame budget (120 fps). It lowers the render scale first, where the host has one: a game's dynamic resolution (`docs/screens.md`, at the game's own `Config::budget` or 8.33 ms). In the editor, Best appearance sets the recipe viewport's render scale to 100% and Best performance sets it to auto, which renders fewer pixels when the frame runs over its budget and upscales them with FSR 1. The render-scale chip under the viewport also offers 75% and 50%, which count as Best performance, and the Quality page shows the scale in use. The scene viewport renders at native scale. Once the scale is at its floor, or the host has none, Best performance switches the costliest effects off one at a time, in the order below. It needs 30 frames over budget in a row to drop one, and brings the last one back after 240 frames under 70% of the budget. A switch you turned off stays off, and Best appearance never drops anything.

The UI and text always render at native resolution.

## Effects

Each switch skips its pass everywhere: in the editor's viewport, in Play and in a game. A pass runs only where the scene asks for it and its switch is on. `Renderer::set_effects(pfx_settings::Effects)` applies a set of switches to a live renderer, and `Renderer::set_shedding(true)` lets Best performance hold passes off. `Renderer::effects()` returns what runs, `chosen_effects()` the switches as set, and `shed()` the effects held off.

| Key | Pass | On by default in a scene | Dropped by Best performance |
|---|---|---|---|
| `effects.taa` | temporal anti-aliasing (`[live] taa`); off, each frame draws unjittered and is shown as drawn | yes | never |
| `effects.ssr` | screen-space reflections (`[live] ssr`); off, surfaces reflect the scene cube, the probes and the sky | yes | 4th |
| `effects.scene_reflections` | the scene's own reflection cube, traced when a scene is staged (`[live] scene_reflections`) | yes | never: it costs at staging, not per frame |
| `effects.ambient_occlusion` | screen-space ambient occlusion and contact shadows (`[shadows] quality = "high"`) | where the scene asks | 5th |
| `effects.bloom` | the finish's bloom (`[finish] bloom`) | where the scene asks | 6th |
| `effects.flare` | the screen-space lens flare (`[flare]`) | where the scene asks | 3rd |
| `effects.motion_blur` | the camera's motion blur (`[motion_blur]`) | where the scene asks | 1st |
| `effects.depth_of_field` | the lens's depth of field | where the camera's lens asks | 2nd |
| `effects.fog` | room haze and height fog | where the scene asks | never |

A scene's own `[live] taa`, `ssr` and `scene_reflections` set to `false` still turn their pass off when the switch is on: a pass runs only when both say so (`Renderer::set_taa` and `set_ssr` hold the scene's choice, `set_effects` the user's). When the scene-cube switch is turned back on, the editor's scene viewport traces the cube again at once. The recipe viewer traces it again at its next edit.

## Where they are kept

pfx's own settings live in `settings.pitofx` in pfx's state folder (`$XDG_STATE_HOME/pfx`, else `~/.local/state/pfx` on Linux; `~/Library/Application Support/pfx` on macOS; `%LOCALAPPDATA%\pfx` on Windows), next to the editor's recent files and view switches, in the `.pitofx` format (`docs/pfx-file.md`):

```toml
[display]
vrr = true
stats = false
frame_rate = "auto"

[graphics]
quality = "performance"

[graphics.effects]
taa = true
ssr = false

[licence]
key = "PITO1.…"
```

The crate `pfx-settings` reads and writes the file (`Store::machine()`, `Store::preferences`, `Store::keep_preferences`, `Store::licence_key`). It keeps every other table in the file untouched.

A game keeps its own copy in its `pfx_game::Settings`, under `[pacing] vrr` and `frame_rate` and `[graphics]`. A project sets its game's defaults in `project.toml`:

```toml
[authoring.pfx.graphics]
quality = "performance"

[authoring.pfx.graphics.effects]
motion_blur = false
```

An unknown quality or effect is refused when the project opens.

## The first measurement

With the frame rate and the quality still unset, the editor times its first 60 viewport frames on the GPU. If the median native frame misses 8.33 ms, it picks Auto and Best performance; otherwise Uncapped and Best appearance. It keeps the choice and never measures again. `pfx settings set quality appearance` or the panel overrides it at any time.

## A game's settings menu

`pfx_game::SettingsMenu` is the shared runtime component. It shows VRR, the frame rate, the quality and each effect in a game's own UI, drawn with the game's font through `pfx_game::MenuLook`, and changes the game's `Settings`, which the runtime applies live. `pfx_game::menu::actions()` declares its actions: `Esc` or the pad's Menu button opens and closes it, the arrows or the D-pad move and change a row. A game made with `pfx new` uses it:

```rust
fn actions(&self) -> Vec<pfx_game::ActionSpec> {
    pfx_game::menu::actions()
}

fn tick(&mut self, _world: &mut pfx_game::World, _tick: &pfx_game::Tick, input: &pfx_game::Input) {
    self.menu.tick(input, &mut self.settings);
}

fn ui(&mut self, _world: &pfx_game::World, frame: &pfx_game::UiFrame) -> pfx_game::Ui {
    self.menu.ui(frame, &self.settings, &pfx_game::MenuLook::new(FONT))
}

fn settings(&self) -> Option<&pfx_game::Settings> {
    Some(&self.settings)
}
```

The menu fades in over 0.18 s, eased out.

## pfx settings

```
pfx settings get [<key>] [--json]
pfx settings set <key> <value> [--json]
pfx settings unlock <key-file|-> [--json]
```

The keys are `vrr`, `frame_rate`, `stats`, `quality`, `effects.<effect>` and `licence` (get only). Switches take `on`, `off`, `true` or `false`; the frame rate takes `60`, `120`, `uncapped` or `auto` (`cap60` and `cap120` also read); the quality takes `appearance` or `performance`.

```
$ pfx settings set effects.ssr off
effects.ssr = off
$ pfx settings get frame_rate --json
{"key":"frame_rate","value":"auto"}
$ pfx settings get --json
{"measured":true,"path":"…/settings.pitofx","settings":{"vrr":true,"frame_rate":"auto","stats":false,"quality":"performance","effects":{…},"licence":{"tier":"free"}}}
```

Input clap can't parse is refused in clap's shape, with exit 2. A key or value the settings don't take is one line on stderr, `pfx settings: …`, with exit 1. A running editor reads the file when it starts.

## The licence

pfx is free under the AGPL-3.0. A pfx key unlocks the Indie or Pro tier. Unlock in the panel, or `pfx settings unlock`, checks a key offline, with no network and no clock, through the same verifier the splash uses (`docs/splash.md`):

- the key's Ed25519 signature must verify against pfx's public key, compiled in;
- it must be a pfx key with the Indie or Pro tier;
- it must be timed: a key with no paid-through date is refused;
- it covers builds made on or before its paid-through date plus its grace days. If this pfx was built after that, the key is refused with both dates.

A key that passes shows its tier, its licensee (the key's `licensee`, else its email) and the date it covers builds through. It is kept in `settings.pitofx` as its bare token, without its armour lines. A key that fails is refused with a plain reason and nothing is kept.

A game made with `pfx new` carries the key into its splash settings. Its `build.rs` reads the key from `PFX_LICENCE_KEY` if that is set, else from the user's `settings.pitofx`, and compiles it in. The game's `config()` then sets the splash to hide with that key. The splash checks it against the game's own build date (`docs/splash.md`, "Hiding it"), so a Pro key hides the splash in builds made through its covered date, and an Indie or lapsed key shows it. The build script reruns when the settings file or `PFX_LICENCE_KEY` changes.

The panel's "Buy a licence" link to https://pitomd.com/pfx/license is hidden in this build.
