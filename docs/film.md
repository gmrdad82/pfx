# Films

pfx-film renders scripted films offline from any app, at a fixed frame rate, frame by frame and sample by sample. A film is a TOML script of scenes. The app shows each scene through the `Show` trait; the film adds the camera, the looks, captions, marks and a synthesised score on top, then hands every frame and the mixed audio to an `Encoder`. A game uses it for its trailers and store clips, cut from the game's own frames.

It depends on `pfx-core`, `pfx-live`, `pfx-sound` and `pfx-input` (without default features), plus `serde`, `toml` and `png`. It runs no encoder or other tool itself: the default `Files` encoder writes PNG frames, PNG stills and a WAV under the folder the caller gives, and the caller turns them into a video with whatever it likes.

## What it does

- **The timeline:** scenes with lengths, keys eased between (any pfx ease by name, or `step` to hold), speed ramps and freeze frames (`speed`), and a shake through pfx-core's `Shake`.
- **The camera:** focus, zoom, roll, follow and shake as a flat `View` matrix, kept inside the film's layout in any output aspect, with `Camera::lens` for an app's 3D camera and `safe` margins for a platform's interface.
- **The looks:** grade, desaturation and tint, contrast, dim, vignette, grain, split, bloom, letterbox, flashes and dips to black, as pfx-live look passes: one look for the app's picture (`game_look`), one over the overlay (`over_look`).
- **Captions:** text with named styles, outlines, slams, slides, growth and letter spacing that spreads, set letter by letter on the app's sheet.
- **Art:** rules, bands, glows, shades, spots and SVG marks with a label and a check badge; the app adds its own kinds.
- **The score:** recipes of tones and noise with envelopes, bends, filter sweeps and drive on pfx-sound's mixer, cued from the script and mixed under the app's audio, which follows the speed ramps (pitched and muffled when slowed).
- **The writer and the plan:** `shoot` runs the film; `plan` gives each scene's frame count and the moment the app starts it from.

## The traits an app implements

```rust
pub trait Kinds {
    type Scene;
    type Film;
    type Art;
    type Drawn: Drawn;
    const ART: &'static [&'static str];
    fn check_film(film: &Self::Film) -> Result<(), String>;
    fn check_scene(scene: &Scene<Self>) -> Result<(), String>;
    fn check_art(art: &Art<Self::Art>) -> Result<(), String>;
    fn check_script(script: &Script<Self>) -> Result<(), String>;
    fn shown(scene: &Scene<Self>) -> bool;
}

pub trait Show {
    type Kinds: Kinds;
    type Cue: Cue;
    fn plan(&mut self, script: &Script<Self::Kinds>) -> Result<Vec<Self::Cue>, String>;
    fn set_theme(&mut self, scheme: Option<Scheme>);
    fn seek(&mut self, scene: &Scene<Self::Kinds>, take: &Take<Self::Cue>) -> Result<(), String>;
    fn aim(&self, scene: &Scene<Self::Kinds>) -> Option<[f32; 2]>;
    fn tick(&mut self, frame: u64, input: &[InputEvent]);
    fn draw(&mut self, frame: &FilmFrame<'_, Drawing<Self>>) -> Result<(), String>;
    fn audio(&mut self, out: &mut Vec<f32>);
    fn readback(&mut self) -> Result<Vec<u8>, String>;
    fn ticks(&self) -> u64;
    fn finish(&mut self) -> Result<(), String>;
}

pub trait Sheet {
    type Drawn: Drawn;
    fn size(&self) -> [f32; 2];
    fn draws(&mut self) -> &mut Vec<Draw>;
    fn group(&mut self, opacity: f32) -> Option<u16>;
    fn letter(&mut self, letter: Letter);
    fn width(&mut self, text: &str, size: f32, weight: u16) -> f32;
    fn check(&self, centre: [f32; 2], height: f32) -> Option<Draw>;
    fn art(&mut self, art: &Self::Drawn);
}
```

- `Kinds` names the app's own script keys: its `[[scene]]` keys, its `[film]` keys and its art kinds (listed in `ART`), with checks for each. Every `check_*` and `shown` has a default, as do `Show::ticks` and `Show::finish`.
- `Show` is the app. Each film frame the film asks `aim` (where `follow` should look), passes that frame's input to `tick`, cuts the frame and calls `draw`. The app advances by `frame.seconds()` of its own time, draws its picture under `frame.view(out)` and `frame.game_look(out)`, paints `frame.cut.pieces` with an `Overlay` on its sheet, composites the sheet under `frame.over_look(out)`, and gives back what it heard in `audio`. `readback` returns the frame as RGBA8 at the rig's size.
- `Sheet` is the app's overlay surface: flat draws plus letters in its own type. `check` draws the tick inside the check badge's disc.
- `Cast` fills a caption's `{slots}` and poses the app's art; `Sounds` is the score (`Score`, or an app's wrapper with sounds of its own); `Encoder` takes the frames, the stills and the audio (`Files` writes them to disk).

## A minimal film

```toml
[film]
bars = 2.39
glow = 0.4
seed = 3

[style.title]
size = 96
weight = 800
caps = true
outline = 4
smash = 0.5

[[scene]]
name = "open"
seconds = 2.0

[[scene.key]]
at = 0.0
zoom = 1.0
black = 1.0

[[scene.key]]
at = 1.5
ease = "cubic_out"
zoom = 1.4
black = 0.0
speed = 0.25

[[scene.text]]
text = "A {thing}"
style = "title"
at = 0.5

[[scene.sound]]
at = 0.0
sound = "pad"
```

Shooting it:

```rust
let score = Score::new(&score_toml, &[], 1, RATE)?;
let sounds = |name: &str| score.knows(name);
let known = Known { sounds: &sounds, marks: &["mark.svg"] };
let script: Script<MyKinds> = Script::parse(&film_toml, &known)?;

let mut score = Score::new(&score_toml, &[], script.film.seed, RATE)?;
let mut files = Files::open(&out, script.film.frames, script.stills() > 0)?;
let rig = Rig { size: [1920, 1080], fps: 60 };
let crew = Crew { show: &mut app, cast: &my_cast, score: &mut score, encoder: Some(&mut files) };
let reeled = shoot(&script, rig, crew)?;
```

`reeled` counts the frames, the app's ticks, the stills and the audio frames written. `plan(&mut app, &script, fps)` gives the takes without shooting.

## The layout

Every place in a script (a caption's `place`, a key's `focus`, an art's `place`) is a point in the film's layout, 1920x1080 unless `[film] layout = [width, height]` says otherwise (each side in [16, 16384]). The camera keeps its view inside that layout in any output aspect; a portrait rig (taller than wide) turns the layout for the overlay, so captions and bands fit the tall frame. `WIDE` is the default, and `Camera::wide` carries the layout to `picture_in`, `view`, `lens` and the looks.

## Marks and the check badge

`Marks::load(base, &[(name, svg)])` turns each SVG's paths into flat icons, one per outer contour with its holes, from icon id `base` on; the app bakes `marks.shapes` into its icon atlas. A mark art draws one with an optional `label` and, with `check = true`, a check badge: a disc with the sheet's tick and a word beside it. The word and the disc's colour are the app's, set once on the marks:

```rust
let marks = Marks::load(0, &files)?.with_check(Check { word: "Approved".into(), colour: Srgba::hex(0x2f8f4f) });
```

`Check::default()` is the word "Checked" on a mid green.

## Input

A scene's `[[scene.input]]` touches drive the app at given times: `pointer = [x, y]` in the layout, `click = true` for a left click, and `key` for a press and release. A key is any pfx-input `Key`, named by its variant without regard to case or underscores (`enter`, `space`, `a`, `f5`, `page_up`, `numpad_1`), or a digit `0` to `9`.

## Determinism

pfx-film never reads the wall clock or a global random source. The film's time is the frame index over the fps, the shake runs from a fixed seed, the score's noise from the seed the app gives `Score::new` (the script's `seed`), and the app gets its time only from `frame.seconds()`. So the same script, revision, seed and app give the same frames, stills and samples byte for byte; the crate's test renders a film twice and compares every byte.

## The audio contract

- The film's audio is 48 kHz (`RATE`) stereo (`CHANNELS`, interleaved left and right) in `f32` samples.
- Each film frame the app hands back what it heard in that frame in the same format; the film resamples it by the frame's speed, so a slowed scene sounds pitched down and muffled; in a scene `Kinds::shown` hides, the app's audio is silent and the score plays on.
- The score is mixed on top, then softly limited under a 0.89 ceiling. `Encoder::finish` gets the mixed samples with the rate and channel count; `Files` writes them as a 16-bit PCM WAV, `audio.wav`.
- A frame's audio is `frame * RATE / fps` samples in, so any fps that divides evenly gives frames of equal length and every fps keeps the audio in step with the frames.

## Tests

`cargo test -p pfx-film` renders `crates/film/tests/fixtures/film.toml` twice through a neutral app and checks the frames, the still and the audio match byte for byte; checks the looks pfx-live accepts at every key's limits, every fixture sound, the marks' bake and the script's refusals. The fixtures are neutral files written by hand, listed in `crates/film/tests/fixtures/README.md` and `tests/agnostic/fixtures.txt`.
