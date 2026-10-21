# The start-up splash

Every game built on pfx-game opens with pfx's splash: pfx's Powered by lockup (the words "Powered by" beside the meteor mark and the wordmark, the same artwork the README and the web show), a hairline rule and PITOMD.COM on the game's ground, with a short chime as the rule grows. It is its own screen and it plays first; the game's own intro starts after it, untouched. It lives in the crate `pfx-splash` and is on by default through pfx-game's `splash` feature.

The lockup follows the ground. On a dark ground the splash draws `assets/marks/powered-by.svg`, its words lime; on a light one `powered-by-light.svg`, its words violet, and PITOMD.COM turns violet too. A ground counts as light when its relative luminance (sRGB, linearised, weighted 0.2126, 0.7152 and 0.0722) is above 0.179, the point where black and white text contrast with it alike. The lockup is centred, 12 % of the window's shorter side tall and at most two thirds of its width wide, and the rule under it grows to 1.15 times its width.

A game that credits pfx on its own screen, such as a credits page, draws this same lockup, glows included, on its flat board with `pfx_splash::Lockup` ([lockup.md](lockup.md)).

## What it shows and when

The splash plays for 2 seconds: a 0.5 s fade in, the lockup holding while the whole group pushes in from 95% to 100% of its size, and a 0.5 s fade out. At 0.45 s the rule under the lockup grows from a point at the centre, both ends moving outward at the same eased speed, and reaches its full width at 0.85 s; then PITOMD.COM fades in under it over 0.25 s.

It overlaps the game's loading and never adds a wait:

- the window shows the splash's ground colour from its first frame;
- the splash is composed on a worker thread while the GPU and the renderer start, so drawing it costs no start-up time;
- while it plays, each frame runs one step of the game's own warm-up (its pipelines, glyphs and looks) and then draws the splash;
- when loading finishes first, the splash still completes its 2 s;
- when loading takes longer, the ground colour holds after the fade and the game's first frame comes the moment loading is done.

The splash cannot be skipped: a key or a click does nothing until it ends. Its timeline reads the clock, not a frame count, so a slow frame never stretches it.

## The chime

The splash has its own sound, a short sonic logo timed to the rule:

- it is silent until 0.45 s;
- as the rule grows from the centre, a detuned saw rises from D3 to D4 through a resonant low-pass that opens from 220 Hz to about 8 kHz, with a band of air over it, on the rule's own ease-out curve, widening in stereo as the line spreads;
- at 0.85 s, as the rule reaches full width, the sweep cuts and a clean D6 tone with a short FM glint and a soft D3 body resolves it;
- the tail rings in a small room and fades out with an ease to silence at 2.0 s.

It is 48 kHz stereo at -30 LUFS short-term (BS.1770, the loudest 3 s window), with its true peak under -15 dBTP: a true-peak limiter holds the master at -1.5 dBTP and a fixed cut after it sets the chime 14 dB below the limiter's level (`pfx_splash::LEVEL_DB`), its ceiling -15.5 dBTP (`pfx_splash::CEILING_DB`). The level is set in pfx and is the same in every app and game; nothing overrides it. There is no sample on disk. `pfx_splash::chime()` synthesises it from code in about 20 ms, with fixed seeds and pfx's deterministic math, so every build makes the same samples. The room is ported from Freeverb (public domain; NOTICE.md).

pfx-game plays it through the game's audio output:

- it is made on its own thread while the splash composes;
- it is handed to the mixer on the frame the rule appears, the first frame drawn after 0.45 s on the splash's clock, cut to that frame's time, so its first sound lands with that frame and its sweep follows the rule's growth. When that frame is late and the cut falls inside the sweep, the clip opens with a 4 ms ramp from silence, so it never clicks. If it is ready late, it starts at the matching point in the sound, so it never drifts and never delays the splash;
- it plays at the game's master volume, and not at all when the master is muted;
- it is silent with sound off (`Config::sound(false)`) or with no output device, and the splash plays the same;
- a Pro key that hides the splash hides the chime too.

## Settings

A game sets the splash in its pfx-game `Config`; there is no per-game code:

```rust
let config = pfx_game::Config::packed(NAME, env!("CARGO_PKG_VERSION"), WORLD)
    .splash(pfx_game::SplashSettings {
        background: pfx_game::SplashColour::parse("#0c0a12")?,
        ..Default::default()
    });
```

The same settings read from TOML, and an unknown key is refused:

```toml
background = "#0c0a12"
show = "auto"
hide = false
key = "PITO1.…"
```

| Key | Default | Meaning |
|---|---|---|
| `background` | `#0c0a12` | The ground colour, `#rrggbb`. |
| `show` | `"auto"` | `"auto"`: with `hide = true`, a verified Pro key that may hide it covering the build date hides the splash, as in [Hiding it](#hiding-it). `"always"`: the splash shows with any key or none. Anything else is refused. |
| `hide` | `false` | Hide the splash. It takes effect only with a valid pfx Pro key in `key` whose group may hide it. |
| `key` | none | A pfx licence key, as issued or with its armour lines. |

## Hiding it

Only a pfx Pro key whose group may hide it hides the splash. With `hide = true`, the key is checked offline at start-up, with no network and no clock:

- its Ed25519 signature must verify against pfx's public key, compiled into the engine;
- it must be a pfx key with the Pro tier; an Indie key shows the splash;
- its group must be one that may hide the splash, as in [Who may hide it](#who-may-hide-it);
- every pfx key is timed: it covers a build made on or before its paid-through date plus its grace days, read from the key. The build date is the game's, set at compile time and never today's date, so a build made while the key was paid keeps working after the key lapses. pfx-game takes it from `Config::build_date` first (a `pfx new` game's `build.rs` sets it from `SOURCE_DATE_EPOCH`, else from the committer time of the game repo's `HEAD`, even with uncommitted changes: [Projects](projects.md), "The build date"), then from `SOURCE_DATE_EPOCH` as pfx-splash was compiled; a date that is not a real `YYYY-MM-DD` is ignored. A key with no paid-through date, a lifetime key, or a build with no date keeps the splash.

A game made with `pfx new` takes its key from the user's pfx settings, where `pfx settings unlock` or the editor's Unlock keeps it, or from `PFX_LICENCE_KEY` at build time, and sets `hide` with it ([settings.md](settings.md#the-licence)).

With `show = "always"` the splash shows whatever the key: no key is read or checked, and `hide` and `key` are ignored. Products that always carry the splash set it so (`pfx_splash::Show::Always`, `pfx_game::SplashShow::Always` in a game's `Config::splash`).

No key, an Indie key, a friends-and-family, education or nonprofit key, a key of a group pfx does not know, an expired key or a key that does not verify shows the splash. A splash that fails to compose or draw is left out and the game starts; it never stops a game.

### Who may hide it

With `hide = true` and `show = "auto"`, by the key's tier and its signed `group` claim (a key without one, or with a `null` one, is a `customer` key):

| Key | Hides the splash |
|---|---|
| none (Free) | no |
| Indie | no |
| Pro, `customer` | yes, while it covers the build date |
| Pro, `owner` | yes, while it covers the build date (owner keys are issued through 9999-12-31) |
| Pro, `friends-family` | no |
| Pro, `education`, `nonprofit` or any other group | no |

The rule, a Pro key of an allowed group, sits in one place, `Group::may_hide_splash` in pfx-splash, which the splash and the settings panel's switch both call. `show = "always"` plays the splash for every key. Beyond this and the licence badge, the group changes nothing in pfx.

## Showing the splash in your own window

An app that opens its own window instead of running through pfx-game shows the same splash with `pfx_splash::Player`, the player pfx-game itself runs: the same composition on a worker thread, the same timeline, look and chime, and the same build-date and Pro-key rules. Add the crate `pfx-splash` and drive the player from the window's frame loop until it says it is done:

```rust
use pfx_splash::{Phase, Player};

let mut splash = Player::start(&settings, Some(BUILD_DATE), [width, height], sound_on);

if let Some(player) = &mut splash {
    let phase = player.phase(now_ns, loaded);
    if let Phase::Showing { at, .. } = phase
        && let Some(clip) = player.chime(at)
        && !muted
    {
        mixer.play(clip, master_volume);
    }
    if phase == Phase::Done {
        splash = None;
        start_intro();
    } else if player.settle(&gpu, [width, height]).is_err() {
        splash = None;
    } else {
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                let view = frame.texture.create_view(&Default::default());
                if player.draw(&gpu, &view, format, [width, height], phase).is_err() {
                    splash = None;
                }
                gpu.queue.present(frame);
            }
            _ => reconfigure_surface(),
        }
        window.request_redraw();
    }
}
```

- The splash runs on the app's clock, not on frames, so the app redraws every frame while it lives (`request_redraw` each frame, the event loop polling) and goes on loading on its own threads meanwhile. An app that draws only on input would show a still splash that then jumps to its end.
- A `Suboptimal` texture is drawn and presented like a `Success` one, and the surface reconfigured after; `Lost` and `Outdated` take the app's usual reconfigure.
- `gpu` is the app's `pfx_gpu::Gpu`, the one pfx's present layer gives it; an app that made its own device builds one from its adapter, device and queue, whose fields are public.
- `Player::start(settings, build_date, size, sound)` starts composing at the window's pixel size, and making the chime when `sound` is true, each on its own thread, and returns at once. It returns `None` when a verified Pro key hides the splash, which happens only under `show = "auto"`; then the app starts as if the splash had played. `build_date` is the app's own `YYYY-MM-DD` build date, as in [Hiding it](#hiding-it).
- `phase(now_ns, loaded)` reads the app's clock in nanoseconds; the splash's 2 s start at the first call after the composition is ready (`ready()` says when). `loaded` is whether the app has finished its own loading: `Phase::Waiting` holds the ground after the fade until it has, and `Phase::Done` is the app's cue to start its own intro.
- `settle(gpu, size)` takes the finished composition onto the GPU and recomposes on the worker thread when the window's size changes; call it each frame before `draw`.
- `draw(gpu, view, format, size, phase)` clears the view to the ground and draws the frame for that phase in one flat pass, and submits it. The ground is `settings.background.srgba()`, a `pfx_splash::Srgba`, for clearing the window's first frames before the composition is ready.
- `chime(at)` hands over the chime once, on the first frame the rule shows (`at` past 0.45 s) and when it is ready, cut to start at `at` seconds into the splash so it never drifts, with a 4 ms ramp from silence at the cut: a `pfx_sound::Clip` of 48 kHz stereo samples for the app's own mixer. The app plays it at its master volume, and not at all when muted.
- A resize during the splash shows the previous composition scaled to the new size until the worker thread has recomposed it, 140–240 ms later.
- A splash that fails to compose or draw is dropped and the app goes on; it never stops an app.

## Speed

The art is compiled in (the lockup's and the site's SVGs, `include_str!`), so nothing is read from disk. Composing it at the window's pixel size (reading the SVGs, painting the mark's glow and blur at that size and baking the shapes' distance fields) takes 140–240 ms on a Ryzen 9 9950X, on the worker thread. Each frame then rebuilds a few dozen draws and runs one flat pass: under 0.1 ms of CPU, and 3.5 ms median, 7.0 ms p99 for the whole frame at 3840x2160 on a Radeon RX 9060 XT (built, submitted and waited for, before present), inside the 8.33 ms frame of the 120 fps budget every GUI holds, with 1.3 ms to spare at p99.

## Making the pictures

`cargo run --release -p pfx-splash --example options -- <dir> 60` draws the splash offscreen: a still and a 2 s frame sequence, each at 1920x1080 and 1080x1920, and the chime as `chime.wav`; a `#rrggbb` argument draws it on that ground instead of the default. `bench` times frames at 1080p and 4K instead. `cargo run --release -p pfx-splash --example chime -- <out.wav>` writes the chime and prints its short-term and integrated loudness and its true peak. `cargo run -p pfx-splash --example outline` remakes `crates/splash/art/site.svg` from JetBrains Mono Bold; the lockup comes from `assets/marks/` through `bin/marks`, which copies it into `crates/splash/assets/`.
