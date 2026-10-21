# pfx compose

`pfx compose` writes original music in a style kept inside a `.pitofx`. A style is data: tempo and key ranges, a harmony table, melody steps, the instrument roles with their rhythms and synthesised patches, the form, one-shot cues and the master. From a style, a seed and a length, the composer writes a score, every note of it, into the same document, and renders the score to a 48 kHz stereo master at the style's loudness and under its true-peak ceiling. The same style, seed, length and hits give the same notes and the same audio bytes, on every run and every machine.

The library is the crate `pfx-compose`; the command is `pfx compose`.

## Try it

`samples/chase.pitofx` carries a public style, `[compose.style.build_drop]`: a through-composed build and drop for a film cue at 118 to 124 bpm, 18 roles on synthesised patches only, four sections (`rounds`, `tension`, `drop` and `card`), mastered to -16 LUFS short-term with a -1 dBTP ceiling. Compose 30 seconds with the drop and the end card on hits at 18 s and 26 s:

```
cp samples/chase.pitofx tmp/chase.pitofx
pfx compose tmp/chase.pitofx --style build_drop --seed 11 --seconds 30 --hits 18,26 --name title --wav tmp/title.wav --json
```

It prints one JSON line (A minor at 120 bpm, 413 notes in 19 stems, -16.00 LUFS short-term max, -19.98 LUFS integrated, -1.18 dBTP) and writes `tmp/title.wav`. Then:

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
| `harmony.secondary` | the chance of an applied dominant, such as `V7/ii`, on the second half of a chord before a different chord it leads to; a section's own `secondary` overrides it |
| `harmony.phrase`, `harmony.turn`, `harmony.turns` | chords in a phrase, and the numerals that close every phrase (`turn`), or a list of them taken phrase by phrase (`turns`) |
| `harmony.land` | the last chord plays as a triad, and the melody, counter and bass land on its root by stepwise cadence rules in the last bar |
| `melody.develop` | statements in turn, one per motif: `state`, `repeat`, `vary` (the first half kept, a new second half), `sequence` (moved by `melody.sequence` scale steps, turning back at the range), `invert` or `return`; the first motif is the theme |
| `melody.sequence` | scale steps a sequence moves, 1 by default |
| `melody.split`, `melody.fragment` | from this intensity on, long notes split into two stepping toward the next note, and a sequence plays its first half twice |
| `melody.passing` | the chance of a passing tone between two chord tones a third apart, one or two beats apart in one section, for a `chord_tones` melody |

A numeral without an accidental takes the key's own scale degree, and its case sets the third.

### `[[role]]`

Each role is one instrument line and becomes one stem.

| Key | Meaning |
|---|---|
| `name`, `patch` | the stem's name and the patch it plays |
| `plays` | `chord`, `bass`, `arp`, `melody`, `hit`, `counter`, `answer`, or one of the section events `riser`, `reverse`, `roll`, `impact` |
| `patterns` | `x` and `.` steps for a `bass`, `arp` or `hit` role, picked by the section's intensity |
| `euclid`, `rotate` | `[hits, steps]` Euclidean rhythms in place of patterns, and their rotations |
| `tones`, `octaves`, `semis` | cycles, one value per hit: the chord tone, the octave and a transposition |
| `voicing` | `lead` (the smallest movement from the last chord), `root` or `top` |
| `range` | the midi range a chord is voiced in |
| `octave`, `gain` | the role's octave and level |
| `hold`, `accent`, `ramp` | the note length in beats, the velocity drop off the accent, and a velocity swell across a section |
| `follows` | the role a `counter` or `answer` follows: a counter is a line below it, chord tones on strong beats and moving against it; an answer echoes its phrases where it rests |
| `delay`, `interval` | how many beats after the leader an answer comes in (up to four bars) and how many scale steps away |
| `walk`, `inversions` | chances that a bass walks by step into the next chord, and takes the chord's third or fifth when that moves it by step; such a bass picks its own octave, so it takes no `octaves` |
| `enter` | the level from which the role plays in a section with a `level` and no `roles` |

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
| `stop`, `through` | the last `stop` beats rest, but for the roles named in `through` |
| `triads` | the section's chords play as triads |
| `secondary` | the section's chance of an applied dominant |
| `plans` | the plans the section is in: a section in a plan plays only when `--plan` composes that plan, and the plain form takes the sections in none |
| `level` | from 0 to 1: sets the section's `intensity` to `[level, level]` when it has none, its `tempo` from `[levels]` when it is 1, and its roles from their `enter` when it names none |

`[levels] tempo = [at level 0, at level 1]` gives a levelled section its tempo ratio, rising evenly in ratio between the two, each from 0.25 to 4 like a section's own `tempo`.

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

The score is the source of truth after composing: edit a note by hand, and `pfx compose render` plays the edit. `style = { file = "music.pitofx", name = "..." }` takes the style from another document in the document's folder or below it, never from outside it. Composing again under an existing name replaces that score, and the output says so (`replaced` in `--json`). When the document's `[music]` table is marked with that name, it is rewritten from the new score too, with or without `--music`, so a game never loops the new stems on the old score's sections. `pfx compose render` rewrites a marked table the same way, so `[music]` is generated from the score: a hand edit belongs in the score, and one made to the table itself is replaced on the next render. The output says `[music] written` (`"music": true` in `--json`) only when the table changed.

## Stored as a recipe, rendered on demand

A document holds the score and no audio: a 30 second, 19 stem score adds a few KB to the `.pitofx`. The audio is rendered from the score whenever it is needed, and the same score always renders the same bytes.

Before anything is allocated, the render refuses a score or style it can't play safely, naming the key: a length over 600 seconds, a tempo of 0 or over 400 bpm, a section tempo outside 0.25 to 4, an envelope stage, `at_ms`, `glide_ms` or `sweep_ms` over 60 seconds, a number that isn't finite in a note, a pan, a filter, a drive, an oscillator or the master, a gain over 64 either way, an oscillator `octave` outside -8 to 8, a cue that would render over 600 seconds, a role named `room` (the room stem's name), and more than 64 roles, 256 patches, 16 oscillators in a patch, 256 sections, 200,000 notes, 64 cues, 16 voices in a cue, 4,096 fires or 64 distinct samples. A sample chunk is refused from its size and its WAV header, before it is decompressed or decoded, when it is over 64 MiB, outside 8 to 192 kHz, over 8 channels, or over 600 seconds of stereo as stored or once resampled.

Each stem, cue and resampled sample is held as stereo 32-bit float for the whole render, 0.38 MB per second, alongside the master chain's working buffers and each patch's precomputed envelope curves. A 30 second score of 19 stems holds about 300 MB while it renders; at the 600 second limit each stem is 230 MB, so a dozen roles and the chain's buffers take about 4.6 GB. The render adds this up before it allocates and refuses a score that would hold over 6 GiB at once, or render over 2^35 oscillator frames, with the figure in the message.

The render runs each role's patch into its own stem on pfx-sound's offline note renderer, as many threads as the machine has cores each taking the next stem, then the master chain:

1. the sidechain ducks the bed stems under the key stems;
2. the room (Freeverb) runs on the patches' sends and becomes its own `room` stem;
3. the stems summed in role order, with the cues mixed in where they fire;
4. the end fade;
5. one gain to the style's maximum short-term loudness, from one loudness measurement;
6. the true-peak limiter at the ceiling, its 4x oversampled peak scan cut into time slices that run in parallel and read the same peaks as one pass.

`pfx compose` and `pfx compose render` keep the master in pfx's machine cache as the `compose` kind ([cache.md](cache.md)): one entry holding the master WAV and its measured levels, with everything that cache promises. `PFX_CACHE_DIR=off` reads and writes nothing, a relative `PFX_CACHE_DIR` or a folder other users can write in turns the cache off with one line on stderr, and the entry counts toward `PFX_CACHE_CAP`, where the entries used longest ago go first, and shows in `pfx cache info` and `pfx cache clean --kind compose`. The key is a SHA-256 of the recipe (the style, the score and the SHA-256 of each sample chunk the style reads), the render's own pinned output hashes, pfx's version and the entry's format version. So an edit renders again, and an unchanged score is read back in milliseconds. The pinned hashes are three renders' outputs (a test style with its game stems and cues, the `build_drop` sample and a sample oscillator), held by the render's tests: a change to what the render plays fails those tests until the hashes are pinned again, and pinning them moves every key, so a released build never reads another build's master. Between a change to the render and that pin, `--fresh` renders past the cache and replaces the entry. An entry that fails its checksum is removed and rendered again; with the cache off or out of reach the master is rendered each run, and a master the cache can't keep (over the cap, a full disk) says so on stderr and the run goes on. `--wav` writes the master out, and refuses a path that doesn't end in `.wav` or that names the document. A 30 second master takes about 11.5 MB of the cache.

## Loudness and true peak

The target is the maximum short-term loudness (ITU-R BS.1770, 3 second windows in 100 ms hops), the loudest moment, so a quiet build stays under its drop. A score lands within 0.5 LU of the style's `lufs`; when the limiter pulls it further, the gain is corrected and the limiter runs again, at most three times. Integrated loudness is measured and reported, not targeted. True peak is measured with BS.1770-4 Annex 2's 4x oversampling and stays at or under `true_peak`: the master, the sum of a game's stems, and each of its stingers on its own.

## The command line

```
pfx compose <doc.pitofx> --style <name> --seed <N> --seconds <S> [--name <score>] [--hits <t,t,...>] [--bpm <B>] [--key <E:minor>] [--plan <plan>] [--music] [--fresh] [--wav <out.wav>] [--json]
pfx compose render <doc.pitofx> --score <name> [--music] [--fresh] [--wav <out.wav>] [--json]
pfx compose list <doc.pitofx> [--json]
pfx compose print <doc.pitofx> [--style <name> | --score <name>] [--json]
pfx compose analyze <SOURCE> --into <doc.pitofx> --name <style> [--score <name>] [--json]
```

| | |
|---|---|
| `pfx compose` | composes a score, writes it into the document, renders it and reports it |
| `render` | renders a score already in the document, after a hand edit or on another machine |
| `list` | the document's styles and scores, and whether each score's master is in the cache |
| `print` | the `[compose]` TOML, or one style or score, as TOML or JSON |
| `analyze` | derives a style from music that already exists and writes it as `[compose.style.<name>]` into `--into` (see below) |
| `--hits` | rising seconds inside `--seconds`; a section with `hit = n` starts on the bar nearest the n-th |
| `--bpm`, `--key` | pin the tempo (20 to 400) or the key instead of drawing them |
| `--plan` | composes the form from the sections in that plan (`plans`) in place of the sections in none, and records the plan in the score |
| `--music` | writes the document's `[music]` table from the score, for a game |
| `--fresh` | renders the master even when the cache holds one, and replaces the cached entry |
| `--wav` | writes the master as a WAV |
| `--json` | one JSON line: the score, the style, the seed, bpm, key, chords, sections with their start seconds, the note count, the stems, the `--wav` path (null without one), whether it was cached and whether `[music]` was written, the ms to compose, render and master, the limiter passes, the gain, the short-term maximum and integrated LUFS, and dBTP |

A command line pfx cannot parse is refused as clap refuses it: on stderr, exit 2, with the usage line. A refusal after parsing, such as a style the document lacks, is one line, exit 1, and the document is left as it was.

## Deriving a style: `pfx compose analyze`

`analyze` reads a source, derives a style from it and writes it as `[compose.style.<name>]` into the `--into` document, created when it is missing. An existing document keeps every other table, comment and chunk byte for byte, and a style of the same name is replaced where it lives. The source is told apart by its contents:

- **a `.pitofx` with composed scores:** the exact notes of every `[compose.score.*]`, or of the one `--score` names. `--into` may be the same document, so the style lands beside its scores;
- **a film script** (a `.toml` with a `[film]` table, `[film] bpm` and `[film] score`): each score recipe a cue plays becomes a patch and a part, each cue (with its `every`, `until` and `accel` repeats) a note on the film's beat grid at the recipe's pitch plus the cue's `pitch`, each scene a section with its `score_db`, and an unpitched cue on a scene's first frame a hit. No image, clip or sample is read;
- **a `.wav` or a folder of `.wav` stems,** for music not written in pfx: tempo, meter, onsets, chroma and level from the audio, so harmony, melody and patches are approximate (`coarse`).

The key comes from the Temperley–Kostka–Payne profile on the duration-weighted pitch classes, Krumhansl–Kessler breaking ties; then the harmony's Roman-numeral Markov table, the melody's range, steps, rhythm, rests and motifs, each part's role, patterns, cycles and patch, the sections with their rounds, risers, rolls, impacts and hits, and the master. The style is plain data: tune any value by hand or in the editor. `--json` prints the report (bpm, meter, the ranked keys, the chords, roles, notes and sections), what was written and where, and the ms the analysis took. A source past analyze's bounds (sample rates, lengths, channel counts, sizes, notes outside MIDI 0 to 127, non-finite values) is refused, naming the file and the field.

## In a game

`--music` writes a `[music]` table for pfx-sound's adaptive music ([sound.md](sound.md#adaptive-music)), marked `score = "<score>"`: one stem per role and the room, named `compose/<score>/<role>.wav`, with gain curves from each role's presence per section; the sections by name with their bars (a name used twice becomes variants); and each cue as a `[[music.stinger]]` with its event. Each section keeps its own bars, and a style whose sections change tempo (`tempo` ratios or `[levels]`) adds a `tempo` list, the bpm from each section that changes it, so every section loops on its own downbeats.

The stems are not stored in the document, so pfx-sound's own loaders (`manifest::load`, `load_bytes`, `load_music`, `music_bytes`, `Manifest::track`) refuse a table marked `score` and name the loader to use. A game loads it with `pfx_compose::render::game::music(bytes, folder)`, which renders the score's stems and stingers on load and returns pfx-sound's `Music`, the adaptive track and its stingers. Call it on a worker thread before the music first plays; the 30 second sample takes about as long as its render and master below, and a three-minute score proportionally longer. A file the table names that is a FILE chunk in the document, or a file beside it, is read as it is; only the parts missing from both are rendered, and only from the score the table is marked with: a missing part under another score's name is refused. The rendered parts the table doesn't name and the master mix are dropped as soon as the render returns, and the parts it keeps count toward the same 6 GiB as every part read from a chunk or a file, so the whole load holds at most that. A part with a sample that is not a finite number is refused. Such a part is refused from its size and WAV header before it is read in full when it is over 256 MiB, outside 8 to 192 kHz, over 8 channels or over 600 seconds of stereo, and a table naming over 65 stems or 64 stingers is refused. Composing a score again under a name drops the chunks at that score's own part names (`compose/<score>/<role>.wav` and `compose/<score>/cue/<cue>.wav`), since they belong to the score it replaces; `pfx compose render` keeps them.

The game's stems are mastered on their own: the same gain and limiter as the master, measured on the stems without the cues and with no end fade, so a looped section neither fades nor dips where the film's cues fired, and the stems sum under the ceiling. Each stinger takes the same gain and its own limiter pass. A stinger played over a full stem mix can still pass the ceiling, so a game that plays `[music]` stingers turns on its mixer's limiter (`set_limiter`, [sound.md](sound.md)).

## Measured

`build_drop`, seed 11, 30 seconds, hits at 18 and 26 seconds, 413 notes in 19 stems, release build on a Ryzen 9 9950X, medians of 5 with other jobs keeping the load average at 5 to 7:

| | |
|---|---|
| compose (the notes) | 0.11 ms |
| render (the stems, over the cores) | 59 ms |
| master (sidechain, room, loudness, limiter, true peak) | 68 ms |
| the whole command, writing the document and the cached master | 142 ms |
| the same command with the master cached | 4.1 ms |

## Credits

The music theory is ported, not invented: the scale and chord tables and the Euclidean rhythms from Sonic Pi, the Roman numerals, voicings and applied dominants from tonal, and the counter-line's first-species rules and cadence from foox, all MIT. The loudness meter and true-peak scanner come from ebur128 (MIT) and the room from Freeverb (public domain), through pfx-sound. `analyze` takes its key profiles and correlation from music21 (BSD-3-Clause) and its audio onsets, tempo, chroma and flatness from librosa (ISC). Each is pinned by commit in [INSPIRATION.md](../INSPIRATION.md), with its licence in [NOTICE.md](../NOTICE.md).
