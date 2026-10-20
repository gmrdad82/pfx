# The start-up splash

Every game built on pfx-game opens with pfx's splash: pfx's Powered by lockup (the words "Powered by" beside the meteor mark and the wordmark, the same artwork the README and the web show), a hairline rule and PITOMD.COM on the game's ground, with a short chime as the rule grows. It is its own screen and it plays first; the game's own intro starts after it, untouched. It lives in the crate `pfx-splash` and is on by default through pfx-game's `splash` feature.

The lockup follows the ground. On a dark ground the splash draws `assets/marks/powered-by.svg`, its words lime; on a light one `powered-by-light.svg`, its words violet, and PITOMD.COM turns violet too. A ground counts as light when its relative luminance (sRGB, linearised, weighted 0.2126, 0.7152 and 0.0722) is above 0.179, the point where black and white text contrast with it alike. The lockup is centred, 12 % of the window's shorter side tall and at most two thirds of its width wide, and the rule under it grows to 1.15 times its width.

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

It is 48 kHz stereo at -16 LUFS short-term (BS.1770, the loudest 3 s window), with its true peak under -1 dBTP: a true-peak limiter holds the master at -1.5 dBTP. There is no sample on disk. `pfx_splash::chime()` synthesises it from code in about 20 ms, with fixed seeds and pfx's deterministic math, so every build makes the same samples. The room is ported from Freeverb (public domain; NOTICE.md).

pfx-game plays it through the game's audio output:

- it is made on its own thread while the splash composes;
- it starts with the splash's clock. If it is ready late, it starts at the matching point in the sound, so it never drifts and never delays the splash;
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
hide = false
key = "PITO1.…"
```

| Key | Default | Meaning |
|---|---|---|
| `background` | `#0c0a12` | The ground colour, `#rrggbb`. |
| `hide` | `false` | Hide the splash. It takes effect only with a valid pfx Pro key in `key`. |
| `key` | none | A pfx licence key, as issued or with its armour lines. |

## Hiding it

Only a pfx Pro key hides the splash. With `hide = true`, the key is checked offline at start-up, with no network and no clock:

- its Ed25519 signature must verify against pfx's public key, compiled into the engine;
- it must be a pfx key with the Pro tier; an Indie key shows the splash;
- every pfx key is timed: it covers a build made on or before its paid-through date plus its grace days, read from the key. The build date is the game's, set at compile time and never today's date, so a build made while the key was paid keeps working after the key lapses. pfx-game takes it from `Config::build_date` first (a `pfx new` game's `build.rs` sets it from `SOURCE_DATE_EPOCH`, else from the committer time of the game repo's `HEAD`, even with uncommitted changes: [Projects](projects.md), "The build date"), then from `SOURCE_DATE_EPOCH` as pfx-splash was compiled; a date that is not a real `YYYY-MM-DD` is ignored. A key with no paid-through date, a lifetime key, or a build with no date keeps the splash.

A game made with `pfx new` takes its key from the user's pfx settings, where `pfx settings unlock` or the editor's Unlock keeps it, or from `PFX_LICENCE_KEY` at build time, and sets `hide` with it ([settings.md](settings.md#the-licence)).

No key, an Indie key, an expired key or a key that does not verify shows the splash. A splash that fails to compose or draw is left out and the game starts; it never stops a game.

## Speed

The art is compiled in (the lockup's and the site's SVGs, `include_str!`), so nothing is read from disk. Composing it at the window's pixel size (reading the SVGs, painting the mark's glow and blur at that size and baking the shapes' distance fields) takes 140–240 ms on a Ryzen 9 9950X, on the worker thread. Each frame then rebuilds a few dozen draws and runs one flat pass: under 0.1 ms of CPU, and 3.5 ms median, 7.0 ms p99 for the whole frame at 3840x2160 on a Radeon RX 9060 XT (built, submitted and waited for, before present), well inside a 60 fps frame.

## Making the pictures

`cargo run --release -p pfx-splash --example options -- <dir> 60` draws the splash offscreen: a still and a 2 s frame sequence, each at 1920x1080 and 1080x1920, and the chime as `chime.wav`; a `#rrggbb` argument draws it on that ground instead of the default. `bench` times frames at 1080p and 4K instead. `cargo run --release -p pfx-splash --example chime -- <out.wav>` writes the chime and prints its short-term and integrated loudness and its true peak. `cargo run -p pfx-splash --example outline` remakes `crates/splash/art/site.svg` from JetBrains Mono Bold; the lockup comes from `assets/marks/` through `bin/marks`.
