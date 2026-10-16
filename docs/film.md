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
- **The beat grid:** a tempo, a time signature and a tempo map, so every time in the script may be written in beats and bars, with a check that cuts and cues land on the grid.
- **Ducking:** the app's audio and any named bed sounds duck under the score, a sidechain compressor keyed by the score's envelope, set per film and per scene.
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

## Beat grid

A film cut to music sets its tempo in `[film]`:

```toml
[film]
bpm = 128
signature = [4, 4]
offset = 0.5
snap = "1/4"
strict = false

[[film.tempo]]
at = "16bar"
bpm = 140
```

- `bpm` is the tempo from beat 1, in [10, 1000]. `offset` is the seconds from the film's start to beat 1 (0 by default); the time before it is the lead-in.
- `signature = [beats, note]` sets the bar (`[4, 4]` by default); `beats_per_bar = n` says the same in one number, and a script uses one or the other. A beat is what `bpm` counts.
- `[[film.tempo]]` changes the tempo from a place: `at` is film seconds (a number) or a place counted from beat 1 (`"16bar"` is where the seventeenth bar starts). Each change comes after the one before it, at most 256.
- With `bpm` set, every time in the script may be a string of beats and bars instead of seconds: a scene's `seconds` and `from`, a key's, text's, art's, sound's, flash's, shake's and touch's `at` and `until`, a text's and art's `fade` and `out`, a sound's `every` and `seconds`, a flash's `seconds`, and a style's `fade` and `out`. Write `"8b"`, `"1.5b"`, `"2bar"` or `"2bar+1b"` (`b`, `beat`, `beats`, `bar`, `bars`, joined by `+`). A number is still seconds.
- Each is turned into seconds once, at parse, through the tempo map: a scene's length from where the scene starts, a cue's `at` and `until` from the scene's start, a length on a cue (`fade`, `every`, …) from the cue's `at`, and a style's from the first tempo. Within one tempo it is `beats × 60 / bpm` in a single division, so `"8b"` is exactly 4.8 s at 100 bpm and 3.75 s at 128, and `frames()` gives 288 and 225 frames at 60 fps. Without `bpm` a string time is refused.

### The snap check

`Script::off_grid(fps)` lists what misses the grid by more than one frame at that fps: a scene whose length isn't a whole number of beats, a cut (counted in whole frames, as the film shoots it) off the `snap` subdivision, and a key, flash, shake, text or sound cue's `at` off it. `snap` is `"1"`, `"1/2"`, `"1/4"` (the default) up to `"1/64"` of a beat, or `"off"` to check lengths only. Anything in the lead-in isn't checked. Each line names the scene and the time in beats, from the scene's start and from beat 1:

```text
scene 'drop': a key at 1.4b is off the 1/4-beat grid by 3.0 frames (1.4b from beat 1)
```

It only warns: `off_grid` returns the lines for the app to print. With `strict = true` it refuses with the first line, and `plan` (so `shoot`) refuses with it too.

## Ducking

A `[duck]` table ducks the bed under the score:

```toml
[duck]
under = ["score"]
beds = ["crowd"]
depth_db = 12
attack = 10
release = 250
threshold_db = -30
knee = 6
lookahead = 5

[[scene]]
name = "quiet"
seconds = "4bar"

[scene.duck]
depth_db = 0
```

- **The bed** is what ducks: the app's own audio (what `Show::audio` returns, after the speed ramp) and every sound named in `beds`, the app's samples and stems included.
- **The key** is what it ducks under: `under` (a name or a list) is `"score"` for every score sound not in `beds`, or names score sounds and cue kinds. A recipe in the score names its kind with `kind = "hit"` (braams, hits and the like); a score sound neither keyed nor a bed plays dry, beside both.
- **The law:** `depth_db` is the most the bed drops, in [0, 60]; `threshold_db` (in [-90, 0]) and `knee` (dB in [0, 24]) set where it starts; `attack` and `release` are milliseconds; `lookahead` (ms, in [0, 100]) reads the key that much ahead, so the bed starts down before a hit lands, with nothing delayed. The defaults are 12 dB, 10 ms, 250 ms, -30 dB, 6 dB and 5 ms.
- **Per scene:** `[scene.duck]` overrides any of the law's six fields from that scene's start; `depth_db = 0` turns ducking off there (the bed comes back over the release). A scene override without a `[duck]` table ducks only in those scenes, under the whole score.
- **How:** the key's level is the louder of its two channels, in dB; a soft-knee gain computer gives how far it is over the threshold, capped at `depth_db`; an envelope follower smooths that with the attack and release; the bed is multiplied by the resulting gain. The mixdown keeps the bed, the key and the dry sounds apart while the film is shot and ducks the bed once at the end, before the soft limiter, so it is offline, sample exact and deterministic. It costs about 27 ms per minute of film audio on one core.
- **Routing:** the `Sounds` trait's `route(under, beds)` and `render_apart(frames)` (key, bed, dry) split the score; their defaults put everything on the key. `Score` routes by name and kind, refuses an `under` that is no sound or kind, and plays the bed and the dry sounds on mixers of their own, made only when a script asks for them. An app's own `Sounds` wrapper routes its samples the same way.
- A film without `[duck]` or `[scene.duck]` mixes exactly as before, byte for byte.

## Determinism

pfx-film never reads the wall clock or a global random source. The film's time is the frame index over the fps, the shake runs from a fixed seed, the score's noise from the seed the app gives `Score::new` (the script's `seed`), and the app gets its time only from `frame.seconds()`. So the same script, revision, seed and app give the same frames, stills and samples byte for byte; the crate's test renders a film twice and compares every byte.

## The audio contract

- The film's audio is 48 kHz (`RATE`) stereo (`CHANNELS`, interleaved left and right) in `f32` samples.
- Each film frame the app hands back what it heard in that frame in the same format; the film resamples it by the frame's speed, so a slowed scene sounds pitched down and muffled; in a scene `Kinds::shown` hides, the app's audio is silent and the score plays on.
- The score is mixed on top, then softly limited under a 0.89 ceiling. `Encoder::finish` gets the mixed samples with the rate and channel count; `Files` writes them as a 16-bit PCM WAV, `audio.wav`.
- A frame's audio is `frame * RATE / fps` samples in, so any fps that divides evenly gives frames of equal length and every fps keeps the audio in step with the frames.

## Tests

`cargo test -p pfx-film` renders `crates/film/tests/fixtures/film.toml` twice through a neutral app and checks the frames, the still and the audio match byte for byte; checks the looks pfx-live accepts at every key's limits, every fixture sound, the marks' bake and the script's refusals, and pins the fixture film's hash on Linux and macOS (Windows' maths library gives other bits, so there a film repeats itself exactly but not across systems). `tests/beat.rs` converts beat times exactly at 100 and 128 bpm, through a signature and a tempo map, and names the scene and the beat of every off-grid time; the ducker's unit tests drop a bed by its depth under a hit, bring it back over the release and turn it off per scene, and the film test shoots the fixture ducked under the score, under a cue kind and off. The fixtures are neutral files written by hand, listed in `crates/film/tests/fixtures/README.md` and `tests/agnostic/fixtures.txt`.
