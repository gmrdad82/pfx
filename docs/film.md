# Films

pfx-film renders scripted films offline from any app, at a fixed frame rate, frame by frame and sample by sample. A film is a TOML script of scenes. The app shows each scene through the `Show` trait; the film adds the camera, the looks, captions, marks and a synthesised score on top, then hands every frame and the mixed audio to an `Encoder`. A game uses it for its trailers and store clips, cut from the game's own frames.

It depends on `pfx-core`, `pfx-gpu`, `pfx-live`, `pfx-sound`, `pfx-text` and `pfx-input` (without default features), plus `serde`, `serde_json`, `toml`, `png`, `pollster` and `swash`; its `video` feature adds [pito-codecs](https://github.com/gmrdad82/pito-codecs) v1.9.0 for the footage films' clips. It runs no encoder or other tool itself: the default `Files` encoder writes PNG frames, PNG stills and a WAV under the folder the caller gives, and the caller turns them into a video with whatever it likes.

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

Every place in a script (a caption's `place`, a key's `focus`, an art's `place`) is a point in the film's layout, 1920x1080 unless `[film] layout = [width, height]` says otherwise (each side in [16, 16384]). The camera keeps its view inside that layout in any output aspect; a rig whose shape differs from the layout's (a portrait rig under a wide layout, or the other way) turns the layout for the overlay, so captions and bands fit the frame; a layout already the rig's shape stays as it is. `WIDE` is the default, and `Camera::wide` carries the layout to `picture_in`, `view`, `lens` and the looks.

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

### Sidechains inside the score

`[duck.<name>]` ducks score sounds under other score sounds, the bass under the kick, several at once:

```toml
[duck.bass]
under = ["kick"]
beds = ["bass", "sub"]
depth_db = 12
attack = 2
release = 40
lookahead = 2

[duck.pads]
under = ["kick", "braam"]
beds = ["pad"]
depth_db = 6
```

- `under` and `beds` each name score sounds or cue kinds (`kind = "kick"` on a recipe), each once and never in both; `under = ["score"]` keys on every score sound that isn't one of its beds. A name the score has neither as a sound nor a kind is refused when the film is shot.
- The law is the same six fields with the same defaults, one law for the whole film. The sidechains run in name order, each keyed by the stems as the ones before it left them, so a bed of one can be the key of the next.
- `[duck]`'s own keys still duck the app's audio under the score. A `[duck]` table holding only named sidechains leaves the app's audio alone; with its own keys, or empty, it ducks the app too, after the sidechains.
- **How:** `Score::stems` puts every score sound that a sidechain names on a stem of its own (one pfx-sound mixer for each set of sidechains a sound belongs to, made only for such a film); the mixdown keeps each stem apart and ducks the beds at the end, with the same follower and gain computer. The `Sounds` trait's `stems(selectors)` and `render_stems(frames)` are the hooks; their defaults refuse named sidechains, so an app's own `Sounds` wrapper overrides both to take part.

## The drop and the limiters

A drop hits when it is louder than the build. Two things set that:

- **The score's gain by scene:** `[scene.score] gain_db = -6` (in [-60, 12]) scales every score cue that starts in that scene, so a build written at the drop's levels sits 6 dB under it. It applies to cues by where they start: a sound cued in the build keeps the build's gain as it rings into the drop.
- **The limiters don't flatten it.** The score's mixer soft-clips each sample above the score table's `limiter` and clamps at full scale, and the film's master soft-clips each sample above `[film] limit` (0.72 by default) with a tanh, then scales the whole film down at once if its peak is over `[film] ceiling` (0.89 by default, in (0.1, 1]). The last is one gain for the whole film, so it keeps the build's and the drop's levels apart; the clips touch only peaks. In the test fixture, at 128 bpm, the build and the drop sit at -15.8 and -14.7 dB RMS as written, -21.8 and -14.7 with the build at `gain_db = -6`, and a drop pushed until it limits loses 0.1 dB of its lead to the clips. Raise `limit` and `ceiling` towards 1 for more headroom.

A film that sets neither `[scene.score]`, `limit` nor `ceiling` mixes exactly as before.

## Determinism

pfx-film never reads the wall clock or a global random source. The film's time is the frame index over the fps, the shake runs from a fixed seed, the score's noise from the seed the app gives `Score::new` (the script's `seed`), and the app gets its time only from `frame.seconds()`. So the same script, revision, seed and app give the same frames, stills and samples byte for byte; the crate's test renders a film twice and compares every byte.

## The audio contract

- The film's audio is 48 kHz (`RATE`) stereo (`CHANNELS`, interleaved left and right) in `f32` samples.
- Each film frame the app hands back what it heard in that frame in the same format; the film resamples it by the frame's speed, so a slowed scene sounds pitched down and muffled; in a scene `Kinds::shown` hides, the app's audio is silent and the score plays on.
- The score is mixed on top, then softly limited above `[film] limit` (0.72) and scaled under `[film] ceiling` (0.89). `Encoder::finish` gets the mixed samples with the rate and channel count; `Files` writes them as a 16-bit PCM WAV, `audio.wav`.
- A frame's audio is `frame * RATE / fps` samples in, so any fps that divides evenly gives frames of equal length and every fps keeps the audio in step with the frames.

## Footage films

A film needs no game: `pfx_film::reel` is a built-in `Show` and `Kinds` whose scenes are footage, and `pfx film` assembles one from the command line.

```
pfx film <script.film.toml> --out <dir> [--json] [--plan]
```

`--out` is a folder under the caller's `tmp/`. The command writes `frames/000000.png...`, `stills/<name>.png` for scenes that name a still, `audio.wav` and `manifest.json`, and prints the manifest's path (or, with `--json`, the manifest itself). It never runs an encoder; the manifest's `pcod` field is the exact command that makes the video, `pcod frames <out>/frames --audio <out>/audio.wav --fps <fps> <out>/film.mp4`. `--plan` checks the script and its files and prints each scene's kind, length, start and frame count without rendering (with `--json`, as JSON; with `--out`, with the `pcod` command too).

On a terminal, `pfx film` keeps a progress line on stderr (frame n/N, frames per second, ETA and a bar), cleared before it prints; piped, with `NO_COLOR` or with `--json` it shows none ([Progress and stderr](render-a-scene.md#progress-and-stderr)).

The manifest carries `film` (1), `script` and `script_sha256`, `frames`, `fps`, `size`, `seconds`, `scenes` (name, kind, seconds, from, frames, source), `out`, `frames_dir`, `stills_dir`, `stills`, `wav`, `samples`, `rate`, `format` (`png`), `pcod`, `pfx` (the version) and `wall_ms` (`total`, `per_frame`).

A footage script is a film script (everything above works: keys, camera moves, looks, captions, marks, flashes, shakes and the score) with these keys added:

| Key | Where | What |
|---|---|---|
| `size` | `[film]` | the output's width and height in pixels, even, in [16, 16384]; default `[1920, 1080]`; `[2160, 3840]` is a vertical 4K film |
| `fps` | `[film]` | the film's frame rate, in [1, 240]; default 60 |
| `score` | `[film]` | a score TOML (the recipes `[[scene.sound]]` cues), relative to the script; none means no score sounds |
| `font` | `[film]` | the font captions, labels and the check word are set in, relative to the script; needed when the film has text; pfx ships none |
| `background` | `[film]` | the `#rrggbb` behind every scene that sets no backdrop; default black |
| `check`, `check_colour` | `[film]` | the check badge's word and disc colour; default "Checked" on a mid green |
| `kind` | `[[scene]]` | `clip`, `frames` or `card` (required) |
| `file` | clip | the video, relative to the script |
| `dir` | frames | the frame folder, relative to the script |
| `fit` | clip, frames | `fit` (the whole picture, bars of the backdrop around it; default) or `fill` (the layout covered, the picture cropped) |
| `volume` | clip, frames | the footage's gain, in [0, 4]; default 1 |
| `track` | clip | which audio track plays, 0 first; a clip with none is silent |
| `colour` or `gradient` | any scene | the backdrop: a `#rrggbb`, or `["#top", "#bottom"]` falling from top to bottom |

A scene's `from` is where in its footage it starts, in seconds. Paths may also be absolute. Set the film's `layout` to the output's shape (`[1080, 1920]` for a vertical film) so captions and marks are placed in it.

- **clip** plays a video from `from` for the scene's length, scaled to the layout by `fit`, with its audio as the scene's stem, resampled to 48 kHz stereo. Decoding goes through [pito-codecs](https://github.com/gmrdad82/pito-codecs) v1.9.0 (MIT, pinned by tag, behind pfx-film's `video` feature, which the `pfx` tool turns on): MP4, MOV, MKV and WebM in H.264, HEVC or VP9, with AAC or Opus audio, on the GPU (VA-API, VideoToolbox, Media Foundation) when it decodes the stream and on the CPU otherwise, bit-exact either way; ffmpeg is never run. A phone recording's rotation is applied. A build without `video` refuses clip scenes when it opens the script.
- **frames** plays a folder of `000000.png`, `000001.png`... as `pfx shot` writes them (or a `frames/` folder inside it, as `pfx film` writes them), with its WAV. A `manifest.json` in the folder gives `fps`, `frames` and `wav` (absolute or relative to the folder); without one the frames count up from `000000.png` at the film's fps and `audio.wav` beside them is the sound. 8-bit and 16-bit RGB, RGBA and grey PNGs read; PCM 8, 16, 24 and 32-bit and float WAVs read, mono or stereo, at any rate (resampled linearly to 48 kHz).
- **card** is the backdrop alone: a colour or a gradient, with the scene's `[[scene.text]]` lines in the film's `[style.*]` and `[[scene.art]]` marks on it. A mark's `file` is an SVG the film's author supplies, read relative to the script; the film ships no wordmark.

When a clip's or a folder's rate differs from the film's, each film frame shows the source frame at or before its time and nothing is blended: a 30 fps clip in a 60 fps film shows every frame twice, a 120 fps folder every other frame. The audio is never retimed; it plays at its own rate. A speed ramp (`speed` in a key) moves through the footage at that speed, and the film pitches its audio as it does a game's.

Each frame is drawn on the GPU through pfx-live's flat renderer: the footage frame as a sprite under the camera's view and the game look (grade, desaturation, contrast, dim, vignette, grain, split), then the overlay (captions, marks, art) under the over look (glow, flashes, dips to black, letterbox), read back as RGBA8. Text is set with `pfx-text` in MSDF glyphs at the output's density, so captions stay crisp at 4K.

**Cost:** a 3-second vertical film at 2160x3840 and 60 fps (2 s of a 4K H.264 screen recording zoomed by the camera with a desaturation key, grain and a vignette, a caption with a 4-unit outline, then a 1 s card with a caption, a mark and a check badge, and a score cue), `pfx film` built in release under `pgpu run --class bench` with the card to itself, took 19.6 s for 180 frames: 109 ms a frame from decode to PNG on disk, the GPU 30% busy on average (an AMD Radeon RX 9060 XT on Mesa radeonsi). The work per frame is the decode and YUV to RGBA conversion on pito-codecs, the 33 MB sprite upload when the picture changes, two flat passes with their looks, one RGBA16 readback and the PNG writes on every core.

## Tests

`cargo test -p pfx-film` renders `crates/film/tests/fixtures/film.toml` twice through a neutral app and checks the frames, the still and the audio match byte for byte; checks the looks pfx-live accepts at every key's limits, every fixture sound, the marks' bake and the script's refusals, and pins the fixture film's hash on Linux and macOS (Windows' maths library gives other bits, so there a film repeats itself exactly but not across systems). `tests/beat.rs` converts beat times exactly at 100 and 128 bpm, through a signature and a tempo map, and names the scene and the beat of every off-grid time; the ducker's unit tests drop a bed by its depth under a hit, bring it back over the release and turn it off per scene, and the film test shoots the fixture ducked under the score, under a cue kind and off. `tests/drop.rs` mixes a build and a drop from `fixtures/drop.toml` and measures that `[scene.score] gain_db = -6` puts the build 7 dB under the drop and the limiters keep that lead, that a `[duck.bass]` sidechain drops the bass by its depth under each kick and brings it back before the next while the kick-less build stays bit for bit, and that sidechains naming what the score lacks are refused. The fixtures are neutral files written by hand, listed in `crates/film/tests/fixtures/README.md` and `tests/agnostic/fixtures.txt`. `cargo test -p pfx-film --test reel` opens a footage script and checks its plan, frame counts, `pcod` command and refusals; its GPU test (`--features video -- --ignored`, under pgpu) generates a 15-frame H.264 and AAC clip with pito-codecs' own encoder, a 4-frame 15 fps PNG folder with a 24 kHz WAV and a manifest, and a card with a caption, a mark and a check badge, shoots the film twice and checks every frame and sample match, that the folder's frames are held across the 30 fps film, and that the clip, the folder and the card show and sound. Its inputs are generated in the test, so no new file is tracked.
