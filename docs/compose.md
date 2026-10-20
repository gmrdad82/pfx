# pfx compose

`pfx compose` writes original music in a style kept inside a `.pitofx`. A style is data: tempo and key ranges, a harmony table, melody steps, the instrument roles with their rhythms and synthesised patches, the form, one-shot cues and the master. From a style, a seed and a length, the composer writes a score, every note of it, into the same document, and renders the score to a 48 kHz stereo master at the style's loudness and under its true-peak ceiling. The same style, seed, length and hits give the same notes and the same audio bytes, on every run and every machine.

The library is the crate `pfx-compose`; the command is `pfx compose`.

## Try it

`samples/chase.pitofx` carries a public style, `[compose.style.build_drop]`: a through-composed build and drop for a film cue at 118 to 124 bpm, 18 roles on synthesised patches only, four sections (`rounds`, `tension`, `drop` and `card`), mastered to -16 LUFS short-term with a -1 dBTP ceiling. Compose 30 seconds with the drop and the end card on hits at 18 s and 26 s:

```
cp samples/chase.pitofx tmp/chase.pitofx
pfx compose tmp/chase.pitofx --style build_drop --seed 11 --seconds 30 --hits 18,26 --name title --wav tmp/title.wav --json
```

It prints one JSON line (A minor at 120 bpm, 437 notes in 19 stems, -16.09 LUFS short-term max, -19.89 LUFS integrated, -1.05 dBTP) and writes `tmp/title.wav`. Then:

```
pfx compose list tmp/chase.pitofx
pfx compose print tmp/chase.pitofx --score title
pfx compose render tmp/chase.pitofx --score title --wav tmp/title-again.wav
```

`title-again.wav` is byte for byte `title.wav`.

## A style: `[compose.style.<name>]`

A style sits under `[compose.style.<name>]` in a document's scene or recipe text. Every table refuses keys it does not know, and an error names the table, the key and the line.

```toml
[compose.style.small]
bpm = [100, 110]
tonics = [["C", 1], ["G", 1]]
modes = [["major", 2], ["dorian", 1]]

[compose.style.small.harmony]
beats = 4
start = ["I"]
cadence = ["V", "I"]
markov = { I = [["IV", 2], ["vi", 1]], IV = [["V", 2]], V = [["I", 2]], vi = [["IV", 1]] }

[[compose.style.small.role]]
name = "kick"
patch = "kick"
plays = "hit"
patterns = ["x...x...x...x..."]

[[compose.style.small.role]]
name = "pad"
patch = "pad"
plays = "chord"
octave = -1
gain = 0.05

[compose.style.small.patch.kick]
osc = [{ wave = "sine", hz = 150, to_hz = 55, glide_ms = 40 }]
env = [1, 300, 0.0, 0]

[compose.style.small.patch.pad]
osc = [{ wave = "saw", cents = -8, pan = -0.4 }, { wave = "triangle", cents = 8, pan = 0.4 }]
env = [30, 0, 0.8, 200]
lowpass = [900, 2400]
send = 0.2

[[compose.style.small.section]]
name = "calm"
bars = [2, 3]
roles = ["kick", "pad"]
intensity = [0.0, 0.4]

[compose.style.small.master]
lufs = -16
true_peak = -1.0
room = [0.6, 0.4, 0.15]
```

### The frame

| Key | Meaning |
|---|---|
| `bpm` | `[low, high]`; the composer picks one in range, the one that puts every `--hits` time nearest a bar line when hits are given |
| `beats_per_bar` | 4 by default |
| `tonics`, `modes` | weighted choices, `[["A", 1], ["E", 1]]` and `[["minor", 3], ["dorian", 1]]`; the modes are the scale table's names |
| `swing` | moves every off-eighth later by this fraction of an eighth |
| `humanize_ms` | moves every note by up to half this many milliseconds either way, seeded |

### `[harmony]` and `[melody]`

| Key | Meaning |
|---|---|
| `harmony.beats` | one chord every this many beats |
| `harmony.start`, `harmony.cadence` | Roman numerals to start from and to end on |
| `harmony.markov` | per numeral, the weighted numerals that may follow it |
| `harmony.sevenths` | chords with sevenths |
| `melody.range` | the lowest and highest midi note |
| `melody.steps`, `melody.rhythm` | weighted scale steps and weighted note lengths in beats |
| `melody.rest` | the chance of a rest in place of a note |
| `melody.motif_bars`, `melody.repeat` | the motif's length in bars, and the chance each bar repeats it |
| `melody.chord_tones` | steps move through the chord's tones rather than the scale |

A numeral without an accidental takes the key's own scale degree, and its case sets the third.

### `[[role]]`

Each role is one instrument line and becomes one stem.

| Key | Meaning |
|---|---|
| `name`, `patch` | the stem's name and the patch it plays |
| `plays` | `chord`, `bass`, `arp`, `melody`, `hit`, or one of the section events `riser`, `reverse`, `roll`, `impact` |
| `patterns` | `x` and `.` steps for a `bass`, `arp` or `hit` role, picked by the section's intensity |
| `euclid`, `rotate` | `[hits, steps]` Euclidean rhythms in place of patterns, and their rotations |
| `tones`, `octaves`, `semis` | cycles, one value per hit: the chord tone, the octave and a transposition |
| `voicing` | `lead` (the smallest movement from the last chord), `root` or `top` |
| `range` | the midi range a chord is voiced in |
| `octave`, `gain` | the role's octave and level |
| `hold`, `accent`, `ramp` | the note length in beats, the velocity drop off the accent, and a velocity swell across a section |

### `[patch.<name>]`

Patches are built from pfx-sound's synthesis ([sound.md](sound.md)): oscillators, an envelope, filters and drive.

| Key | Meaning |
|---|---|
| `osc` | a list of oscillators, below |
| `env` | `[attack_ms, decay_ms, sustain, release_ms]` |
| `linear` | linear envelope segments; exponential by default |
| `lowpass` | `[at intensity 0, at intensity 1]` in Hz, following the section's intensity |
| `sweep`, `sweep_ms` | per note, the lowpass starts at the first value and glides to the second over `sweep_ms`, or over the note |
| `highpass` | a fixed highpass in Hz |
| `drive` | `["soft" \| "tanh" \| "hard", drive_db, mix]` |
| `send` | the level into the room |
| `gain`, `pan` | the patch's level and pan |

`lowpass` and `sweep` drive the same filter, so a patch takes one of them.

An oscillator is a `wave` (`sine`, `square`, `saw`, `triangle`, `pulse`, `white`, `pink`) with `width` for a pulse, `cents`, `octave`, `gain`, `pan`, `at_ms` (a delay from the note's start) and its own `env`. Its pitch is the note's, or a fixed `hz`; `to_hz` with `glide_ms` glides it, and `bend` (semitones) bends it over `glide_ms` or the note. One oscillator takes one glide, so `to_hz` and a bend are refused together.

An oscillator may instead be `sample = "<chunk>"`: a WAV held as a FILE chunk in the same `.pitofx`, played once from the note's start to the clip's end, pitched from midi 60, with the oscillator's `cents`, `octave`, `gain`, `pan` and `at_ms`. A missing chunk is refused when the score renders.

### `[[section]]`

Sections are the form. The composer fits them to `--seconds`, each within its bar range.

| Key | Meaning |
|---|---|
| `name`, `bars` | the section's name and its `[low, high]` length in bars |
| `roles` | the roles that play, or `"*"` for all |
| `intensity` | `[from, to]` across the section; patterns, lowpass and the game's gain curves follow it |
| `round_bars`, `start_roles` | after the first `start_roles` roles, one more role comes in every `round_bars` bars |
| `hit` | the section starts on the bar nearest the n-th `--hits` time |
| `cue` | a cue fired where the section starts on its hit |
| `riser`, `roll`, `impact`, `tail` | a riser over the last `riser` beats, a roll over the last `roll` beats, an impact at the start, and the last bar's notes held `tail` beats |
| `lift` | transposes the section by semitones |
| `tempo`, `pitch` | ratios to the base tempo and pitch |
| `stutter` | drops the last eighth of every `stutter` eighths |
| `gain_db` | the section's level |

### `[[cue]]`

A cue is a one-shot stinger: its own clip, never a stem. In a render it is mixed in only where a section fires it; in a game it plays on its event.

| Key | Meaning |
|---|---|
| `name`, `event` | the cue's name, and the game event that fires it (the name by default) |
| `[[cue.voice]]` | `patch`, `plays` (`hit`, `glide`, `burst` or `chord`), `at` and `beats`, `gain`, `midi`, `hz = [from, to]` for a glide, `count`, `every` and `vels` for a burst, `numeral` and `octave` for a chord, and `fade` |

A cue renders at the score's base tempo and key wherever it fires.

### `[master]`

| Key | Meaning |
|---|---|
| `lufs` | the target maximum short-term loudness, in LUFS |
| `true_peak` | the true-peak ceiling, in dBTP |
| `room` | `[size, damp, wet]` of the room the patches send to |
| `sidechain` | `{ keys, beds, depth, release_ms }`: the key roles' peaks duck the bed roles |
| `fade` | the end fade, in seconds |

## A score: `[compose.score.<name>]`

`pfx compose` writes the score under `[compose.score.<name>]` (the style's name unless `--name` says otherwise): the style it came from, the seed, the length, the hits, the tempo, the key, the chords, the placed sections with their start seconds, every role's notes and lowpass automation, the cues and where they fire. Notes are compact rows, `[beat, midi, beats, velocity]`, with a bend and a fade added when a note has them.

```toml
[compose.score.title]
style = "build_drop"
seed = 11
seconds = 30.0
hits = [18.0, 26.0]
bpm = 120
key = "A minor"

[[compose.score.title.track]]
role = "lead"
notes = [[32.0, 81.0, 0.45, 0.543], [32.5, 81.0, 0.45, 0.543]]
```

The score is the source of truth after composing: edit a note by hand, and `pfx compose render` plays the edit. `style = { file = "music.pitofx", name = "..." }` takes the style from another document in the same folder, never from outside it. Composing again under an existing name replaces that score, and the output says so (`replaced` in `--json`).

## Stored as a recipe, rendered on demand

A document holds the score and no audio: a 30 second, 19 stem score adds a few KB to the `.pitofx`. The audio is rendered from the score whenever it is needed, and the same score always renders the same bytes.

The render runs each role's patch into its own stem on pfx-sound's offline note renderer, one thread per stem, then the master chain:

1. the sidechain ducks the bed stems under the key stems;
2. the room (Freeverb) runs on the patches' sends and becomes its own `room` stem;
3. the end fade, on every stem;
4. the stems summed in role order, with the cues mixed in where they fire;
5. one gain to the style's maximum short-term loudness, from one loudness measurement;
6. the true-peak limiter at the ceiling, its per-frame gain applied to every stem as well, so the stems sum to the limited master apart from the cues.

`pfx compose` and `pfx compose render` keep the master in pfx's cache, `${XDG_CACHE_HOME:-~/.cache}/pfx/compose/<key>/` (or `$PFX_CACHE_DIR/compose/<key>/`), as `mix.wav` and its measured levels, written to a temporary folder and renamed into place. The key is a SHA-256 of the recipe (the style, the score and any sample chunks the style reads), pfx's version and the cache's format version, so an edit renders again and an unchanged score is read back in milliseconds. `--wav` copies the cached master out.

## Loudness and true peak

The target is the maximum short-term loudness (ITU-R BS.1770, 3 second windows in 100 ms hops), the loudest moment, so a quiet build stays under its drop. A score lands within 0.5 LU of the style's `lufs`; when the limiter pulls it further, the gain is corrected and the limiter runs again, at most three times. Integrated loudness is measured and reported, not targeted. True peak is measured with BS.1770-4 Annex 2's 4x oversampling and stays at or under `true_peak`: the master, the sum of the stems, and each cue on its own.

## The command line

```
pfx compose <doc.pitofx> --style <name> --seed <N> --seconds <S> [--name <score>] [--hits <t,t,...>] [--bpm <B>] [--key <E:minor>] [--music] [--wav <out.wav>] [--json]
pfx compose render <doc.pitofx> --score <name> [--music] [--wav <out.wav>] [--json]
pfx compose list <doc.pitofx> [--json]
pfx compose print <doc.pitofx> [--style <name> | --score <name>] [--json]
```

| | |
|---|---|
| `pfx compose` | composes a score, writes it into the document, renders it and reports it |
| `render` | renders a score already in the document, after a hand edit or on another machine |
| `list` | the document's styles and scores, and whether each score's master is in the cache |
| `print` | the `[compose]` TOML, or one style or score, as TOML or JSON |
| `--hits` | rising seconds inside `--seconds`; a section with `hit = n` starts on the bar nearest the n-th |
| `--bpm`, `--key` | pin the tempo (20 to 400) or the key instead of drawing them |
| `--music` | writes the document's `[music]` table from the score, for a game |
| `--wav` | writes the master as a WAV |
| `--json` | one JSON line: the score, the style, the seed, bpm, key, chords, sections with their start seconds, the note count, the stems, the cached master's path and whether it was cached, the ms to compose, render and master, the limiter passes, the gain, the short-term maximum and integrated LUFS, and dBTP |

A command line pfx cannot parse is refused as clap refuses it: on stderr, exit 2, with the usage line. A refusal after parsing, such as a style the document lacks, is one line, exit 1, and the document is left as it was.

## In a game

`--music` writes a `[music]` table pfx-sound's adaptive music reads ([sound.md](sound.md#adaptive-music)): one stem per role and the room, named `compose/<score>/<role>.wav`, with gain curves from each role's presence per section; the sections by name with their bars (a name used twice becomes variants); and each cue as a `[[music.stinger]]` with its event. Sections start on the bar nearest their start, so a style with `tempo` ratios loops close to its bars, not exactly on them.

The stems are not stored in the document. A game loads the table with `pfx_compose::render::game::music(bytes, folder)`, which renders the score's stems and stingers on load and returns pfx-sound's `Music`, the adaptive track and its stingers. Call it on a worker thread before the music first plays; the 30 second sample takes about a third of a second (below). A stem file that is a FILE chunk, or a file beside the document, is read as it is.

## Measured

`build_drop`, seed 11, 30 seconds, 437 notes in 19 stems, release build on a Ryzen 9 9950X, medians of 5 with other jobs keeping the load average at 16 to 20:

| | |
|---|---|
| compose (the notes) | 0.09 ms |
| render (the stems, one thread each) | 79 ms |
| master (sidechain, room, loudness, limiter, true peak, stem gains) | 239 ms |
| the whole command, writing the document and the cached master | 357 ms |
| the same command with the master cached | 12 ms |

## Credits

The music theory is ported, not invented: the scale and chord tables and the Euclidean rhythms from Sonic Pi, and the Roman numerals and voicings from tonal, both MIT. The loudness meter and true-peak scanner come from ebur128 (MIT) and the room from Freeverb (public domain), through pfx-sound. Each is pinned by commit in [INSPIRATION.md](../INSPIRATION.md), with its licence in [NOTICE.md](../NOTICE.md).
