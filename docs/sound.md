# pfx-sound

Live mixing for games and apps, and deterministic mixdown for rendered clips. One `Mixer` does both: the audio device drives it through `Output`, a render drives it directly.

## Channels

Every voice plays on one channel. Channels sit under a master.

```
voice (pitch, gain, level) -> channel filters -> channel drive (if set) -> channel volume/mute -> ducking -> master volume/mute -> soft limiter (if set) -> clamp
```

| Id | Use |
|---|---|
| `ChannelId::MUSIC` | the music track |
| `ChannelId::EFFECTS` | sound effects; every call that names no channel plays here |
| `ChannelId::DIALOG` | voices and narration |
| `ChannelId::custom(n)` | anything a game adds (`Ui`, `Ambience`), with no engine change |

```rust
mixer.set_master_volume(0.8);
mixer.set_channel_volume(ChannelId::MUSIC, 0.4);
mixer.set_channel_muted(ChannelId::DIALOG, true);
let ui = ChannelId::custom(0);
mixer.play_on(ui, &click, Level::default());
```

- Volumes run 0 to 1. A change ramps linearly over `RAMP_SECONDS` (10 ms), so it never clicks; mute ramps the same way. The largest step per frame is 1 / (rate * 0.01), about 0.002 at 48 kHz.
- A channel set before it has audio takes its volume at once, with no ramp.
- `play`, `play_at` and `Timeline` placements are unchanged and play on `EFFECTS` at volume 1, so a render's mixdown stays byte for byte what it was.
- `Mixer::stop(id, fade_seconds)` ends a voice, with an equal-power fade when the time is above zero.
- `Output` mirrors all of this (`play_on`, `set_channel_volume`, `set_master_muted`, `play_music`, ...), behind the same lock the audio thread takes. It resamples a clip, registers a sound and opens a decoded stream before it takes the lock, and the voices that end in the audio thread are handed back and freed on the caller's thread at its next call, so the audio thread neither waits on that work nor frees a clip.
- `Prepared::new(&clip, rate)` (or `Prepared::track(&track, rate)` for a track's loop) converts a clip to the mixer's rate once; `play_prepared(channel, &prepared, level)` and `play_music_prepared(&prepared, fade, level)` then play it without copying the samples, so a sound played every tick costs no allocation. They play exactly what `play_on` and `play_music` play for the same clip.

## Music

```rust
let track = MusicTrack::with_loop(clip, loop_start, loop_end);
mixer.play_music(&track, 2.0, Level::default());
```

- `MusicTrack::once`, `looping` (the whole clip) and `with_loop` (frames in the clip's own rate; scaled when the clip is resampled to the output).
- The loop is a hard wrap from `loop_end` back to `loop_start`, sample exact. Author the points on a zero crossing or a bar line; the engine adds no smoothing.
- `play_music` while a track is playing crossfades: the old track fades out as the new fades in over the given seconds, **equal power** (cos and sin curves, so the squared gains always sum to 1; two unrelated tracks keep their summed power within 0.5 dB). Zero seconds switches at once.
- `stop_music(fade)` fades the current track out. `music()` is the playing track's voice, if any.
- `Mixer::music_position()` is the current track's render position in seconds into the track: the next frame the mixer renders, loop-aware (a looped track reads back inside its loop). `None` when no track plays.
- `Output::music_heard()` is the frame reaching the speaker now, in seconds into the track, for a meter or a playhead read every UI frame. The audio callback records the track's position at the start of each block it renders and when that block plays (the callback's time plus the latency cpal reports, playback minus callback) in a few atomics, without a lock; the read extrapolates from there by the elapsed time and the track's pitch, at most one block ahead (so it holds still if callbacks stop), and steps back across a loop point to the end of the loop. It reads `0.0` until a new track's start is heard, and `None` once no track plays (the old track's fade-out is not followed). A decoded music stream (`play_music_vorbis`) reads the frames its stream has delivered, not wrapped by its loop.

## Adaptive music

An `AdaptiveTrack` is one piece of music made of stems: N clips of one length (one tempo, one bar grid), each with a gain curve over a live intensity from 0 to 1, and named sections the music moves between on the bar or the beat. It plays in the music slot, so it crossfades with a `MusicTrack` like any other track.

```rust
let mut track = AdaptiveTrack::new(stems, 120.0, 4);
track.sections = vec![
    Section { variants: vec![(0, 8), (8, 16)], avoid_repeat: true, ..Section::new("calm", (0, 8)) },
    Section { next: Next::Section("calm".into()), ..Section::new("fight", (16, 24)) },
];
let prepared = track.prepare(mixer.rate())?;
mixer.play_adaptive(&prepared, 1.0, Level::default());
mixer.set_music_intensity(0.8, 2.0);
mixer.request_music_section("fight", Quantize::Bar);
```

- **Stems.** `Stem { name, clip, gain }`: mono or stereo clips, all at one rate and of one length (`prepare` refuses anything else, naming the stem). Every stem reads the same source frame on every output frame, so they stay sample-exact in sync through intensity changes, section jumps and crossfades.
- **Gain curves.** `StemGain { keys, power }`: `[(intensity, gain), ...]` keys, held before the first and after the last. Between two keys the gain moves linearly, or with `power` along a quarter sine (rising) or cosine (falling), so a stem leaving over the same span another enters keeps the summed power constant. `StemGain::above(0.3, 0.1)` brings a stem in from 0.3 to 0.4 (drums above 0.3), `below` takes one out, `constant` holds it.
- **Intensity.** `set_music_intensity(value, seconds)` eases from the current value to the new one over the given time with the track's `ease`: `Linear`, `In(n)`, `Out(n)` or `InOut(n)` (the integer power easings of kira's tweens, see [INSPIRATION.md](../INSPIRATION.md)). The stems' gains are worked out every 16 frames (`CONTROL_FRAMES`) on the track's own clock and ramp linearly inside each block, so the block size the device asks for never changes a sample. `music_intensity()` reads it back.
- **Sections.** `Section { name, variants, next, avoid_repeat }`: each variant is a `[from, to)` range of whole bars of the stems, on the grid of `bpm`, `beats_per_bar` and `offset` (seconds before bar 0). When a variant ends the section flows to `next`: `Repeat` (the default; a fresh variant), `Section(name)` or `Stop` (the track ends and its voice is freed). With no sections the whole length loops as `main`.
- **Variants.** Entering a section picks one of its variants from the track's seeded generator, which takes its seed from the mixer's (`set_seed`) when the track starts: the same seed and the same calls at the same frames give the same samples. `avoid_repeat` never plays the same variant of a section twice in a row.
- **Transitions.** `request_music_section(name, at)` is taken at `Quantize::Now`, the next `Beat`, the next `Bar` or the current variant's `End`, and never later than that end. A boundary the music sits on is taken at once, and a later request replaces one still waiting. `music_section()` is the section playing. With `transition` (seconds, default 0) a jump crossfades equal power: the old position keeps playing on (its tail past the bar line) under a cosine while the new one comes in under a sine; at 0 the cut is sample exact on the boundary, as a `MusicTrack` loop is.
- **Not here.** Pitch does not apply to an adaptive track (the bar grid stays exact); its voice's level, fades, channel and ducking apply as to any music. `Output` mirrors `play_adaptive` (preparing the track before it takes the lock), `set_music_intensity`, `music_intensity`, `request_music_section` and `music_section`.

### The `[music]` table (feature `music`)

A game ships its music as a `[music]` table in a scene, its stems as files inside the same `.pitofx` (or beside it). `pfx_sound::manifest::load(path)` opens a `.pitofx`, finds the first scene in it with a `[music]` table and reads each stem's WAV from the file's own chunks by name, then from the folder beside it (never outside it); `load_bytes` does the same from memory. `Manifest::parse(text)` reads a scene document's TOML and `Manifest::from_table` the table the scene reader hands over (`Scene::music`, as written), and `Manifest::track(read)` loads the stems through the caller's reader.

```toml
[music]
bpm = 120
beats_per_bar = 4
start = "calm"
ease = "in_out"
transition = 0.02

[[music.stem]]
name = "pads"
file = "music/pads.wav"
gain = [[0.0, 1.0], [0.6, 0.4]]

[[music.stem]]
name = "drums"
file = "music/drums.wav"
gain = [[0.3, 0.0], [0.4, 1.0]]

[[music.section]]
name = "calm"
bars = [[0, 8], [8, 16]]
avoid_repeat = true

[[music.section]]
name = "fight"
bars = [16, 24]
next = "calm"
```

| Key | Default | Meaning |
|---|---|---|
| `bpm` | required | beats a minute |
| `beats_per_bar` | 4 | beats in a bar |
| `offset` | 0 | seconds of the stems before bar 0 |
| `start` | the first section | the section the track opens with |
| `intensity` | 0 | the intensity it opens at |
| `ease`, `ease_power` | `"linear"`, 2 | `"linear"`, `"in"`, `"out"` or `"in_out"`, and the power of the last three |
| `transition` | 0 | seconds of equal-power crossfade at a section change |
| `[[music.stem]] name`, `file` | required | the stem's name and its WAV, a chunk name or a path relative to the file's folder |
| `[[music.stem]] gain` | 1 | a number, or `[[intensity, gain], ...]` keys |
| `[[music.stem]] shape` | `"power"` | `"power"` (sine and cosine between keys) or `"linear"` |
| `[[music.section]] name` | required | the section's name (`stop` is kept for `next`) |
| `[[music.section]] bars` | required | `[from, to]`, or `[[from, to], ...]` for variants, in whole bars |
| `[[music.section]] next` | repeat | a section's name, or `"stop"` |
| `[[music.section]] avoid_repeat` | false | never the same variant twice in a row |

Unknown keys and wrong types are errors naming the table and the key.

## Ducking

Off until a game sets it:

```rust
mixer.set_ducking(Some(Ducking::dialog_over_music(9.0, 0.15, 0.6)));
```

While any voice plays on the trigger channel (dialog by default) the target channel (music) dips by `depth_db`. The dip moves linearly in dB: full depth after `attack` seconds, back to 0 dB after `release` seconds. The envelope runs per frame, so it does not depend on the block size the device asks for. `Ducking` is a plain struct: `trigger` and `target` take any channel, so a game can duck ambience under effects. `None` turns it off.

## Pitch and playback rate

Pitch is a ratio: 1 is as recorded, 2 is an octave up and twice as fast, 0.5 an octave down and half as fast. It runs from 1/16 to 8; anything else is clamped.

```rust
mixer.set_pitch(voice, 1.5);
mixer.glide_pitch(voice, 4.0, 2.0);
mixer.set_channel_pitch(ChannelId::MUSIC, intensity);
mixer.glide_channel_pitch(ChannelId::EFFECTS, intensity, 0.5);
```

- A voice plays at `voice pitch x channel pitch`. Buffers, music loops and decoded streams all follow it, and so does a tone's frequency.
- Changes ramp linearly per frame: `set_*` over `RAMP_SECONDS` (10 ms), `glide_*` over the seconds you give. The read position is continuous, so there is no click, only the pitch moving.
- Fades and loop points are in the clip's own frames, so a pitched sound fades and loops in step with its pitch.
- At exactly 1 the clip's samples are read as they are, so the output, and every render's mixdown, is unchanged.
- Any other ratio resamples in the voice's `Resample` mode. `Resample::Sinc` (the default for `play`, music and streams) is a Kaiser-windowed sinc, 16 zero crossings each side, its cutoff at 0.9 of Nyquist. Above 1 the kernel widens by the ratio so the source's top octaves are filtered out before they can fold back, which is the aliasing of cheaper interpolators. Measured at 48 kHz, a tone that would fold back sits at **-80 dB** or lower below the source at 2x (17 kHz source: -80 dB, 21 kHz: -98 dB) and **-91 dB** or lower at 4x (11 kHz: -91 dB, 21 kHz: -106 dB); a tone in the passband comes out within 0.001 dB of its level at both. The cost grows with the ratio (32 taps a channel at 1x, 128 at 4x).
- `Resample::Cubic` (the default for `sound` and `tone_sound`) is a 4-point, third-order Hermite interpolation ported from kira's `interpolate_frame` (MIT or Apache-2.0); `Resample::Linear` reads between two frames. Both read only the frames around the cursor and allocate nothing, about 9 ns a frame against the sinc's 200 or more. They do not widen with the ratio, so a large pitch-up folds its top octaves back: keep them for short effects and small ratios (a few semitones of jitter), and the sinc for music and long sounds.
- `Mixer::play_prepared_pitched_at(channel, at, clip, ratio, level)` starts a prepared clip at an exact frame already at its pitch, with no ramp from 1, so a pitched hit lands on its frame at its pitch (pfx-film's score samples play this way).
- `SoundSpec::resample` picks the mode for every trigger of a sound; `Mixer::set_resample(id, mode)` (and `Output::set_resample`) changes any voice.

## Voice limiting, stealing and jitter

A burst of hits becomes a capped, varied set:

```rust
let hit = mixer.sound(&clip, SoundSpec {
    channel: ChannelId::EFFECTS,
    max_voices: 12,
    steal: Steal::Oldest,
    pitch_jitter: 2.0,
    volume_jitter: 0.25,
    ..SoundSpec::default()
});
mixer.set_seed(run_seed);
mixer.trigger(hit);
```

- `sound` registers a clip once, resampled to the mixer's rate and shared by every voice. `trigger` and `trigger_at` play one.
- `tone_sound(wave, hz, seconds, spec)` registers a tone the same way, so tones take the same cap, stealing, jitter, envelope and channel through the same code: a trigger plays the wave at `hz` times its drawn pitch for `seconds`, then releases (through its envelope, or a `steal_fade` fade without one). `None` plays until stopped or stolen. Each trigger's noise seed comes from the mixer's generator.
- `trigger_in(offset, sound)` and `tone_in(offset, channel, wave, hz, volume)` start the voice `offset` frames after the mixer's clock (`Mixer::clock()`), so hits fired on fast ticks spread through the next block instead of all landing on its first frame. `trigger_at` takes an absolute frame. `trigger_with(sound, volume)`, `trigger_in_with` and `trigger_at_with` also scale this voice's volume (on top of the sound's level and its drawn jitter), so an escalating cue starts at its own level instead of gliding there after the trigger. The output from the start frame on is the same, bit for bit, as the voice started at the block's edge.
- At `max_voices` live voices of the sound, `trigger` steals one first: `Steal::Oldest` takes the earliest started, `Steal::Quietest` the one with the lowest level times its fade times how much of the clip is left. A stolen voice that has already played fades out over `steal_fade` (3 ms by default); one that never reached the mixer is dropped. `max_voices: 0` plays nothing; the default is no cap.
- Each trigger draws a pitch of up to `pitch_jitter` semitones either way and a volume of up to `volume_jitter` (a fraction) either way, from the mixer's own seeded generator. `set_seed` starts it; the same seed and the same triggers give the same samples, bit for bit, on every platform: the mixer's sines, cosines, exponentials and powers (the jitter's pitch, fades, envelopes, filter coefficients, the sinc table) go through pfx-core's bit-exact `sim` functions or the sine table below, never the platform's libm, and a golden test pins the bits of a seeded mix of tones and jittered hits. The mixer never reads the clock or the system's randomness.
- 10,000 triggers in a second leave at most `max_voices` live plus those fading out.
- `set_limiter(Some(threshold))` puts a soft limiter on the master: below the threshold the signal is untouched, above it a smooth knee bends towards full scale without reaching it. It is off by default, and the hard clamp stays behind it.

## Tones

A sustained synth voice for alerts and drones:

```rust
let alarm = mixer.tone(ChannelId::EFFECTS, Wave::Saw, 440.0, 0.4);
mixer.glide_tone_frequency(alarm, 1760.0, 3.0);
mixer.glide_gain(alarm, 0.0, 0.5);
mixer.stop(alarm, 0.05);
```

- `Wave::Sine` reads a 2,049-entry table (one cycle, built at compile time from a Taylor series) with linear interpolation: within 1.2e-6 of a true sine (about -118 dB), the same bits on every platform, and cheaper than a libm call. Equal-power fades read the same table.
- `Wave::Square` and `Saw`. Square and saw are band-limited with PolyBLEP, so their edges do not fold back at high notes.
- `Wave::White` and `Wave::Pink` are noise (see Noise below).
- The game drives the frequency (`set_tone_frequency`, `glide_tone_frequency`), the pitch ratio (`set_pitch`, which multiplies the frequency, as the channel's pitch does) and the volume (`set_gain`, `glide_gain`, a ramped gain on top of the level). All ramp.
- The phase is a 64-bit accumulator: the measured frequency is within 0.1 % of what was asked, for every wave.
- A tone plays until stopped.

## Noise

```rust
mixer.set_seed(run_seed);
let wind = mixer.tone(ChannelId::EFFECTS, Wave::Pink, 0.0, 0.3);
let hiss = mixer.tone(ChannelId::EFFECTS, Wave::White, 0.0, 0.2);
```

- `Wave::White` is a uniform draw in -1 to 1 at every frame: a flat spectrum, an RMS of 0.577 times the volume. `Wave::Pink` is Paul Kellet's refined seven-pole filter over the same draws, scaled by 0.11: power falls 3 dB an octave (measured -3.02 dB an octave from 200 Hz to 12.8 kHz at 48 kHz, every octave within 1.5 dB of the line), and it stays within -1 to 1.
- Each noise voice takes its own seed from the mixer's generator when it is created, so `set_seed` and the order of creation fix the samples bit for bit; two noise voices never play the same noise. The generator is the one jitter uses, so creating a tone moves the jitter draws that follow.
- The frequency does nothing to noise; the pitch ratio, the gain, the level, the channel and the envelope work as on any tone. Kellet's coefficients are tuned for 44.1 kHz; at 48 kHz the slope stays within the tolerance above.

## Envelopes

Attack, decay, sustain and release, per voice, on tones and clips:

```rust
let env = Envelope {
    attack_ms: 5.0,
    decay_ms: 120.0,
    sustain: 0.3,
    release_ms: 80.0,
    shape: Shape::Exponential,
};
let id = mixer.tone(ChannelId::EFFECTS, Wave::Square, 220.0, 0.4);
mixer.set_envelope(id, Some(env));
mixer.stop(id, 0.0);

let hit = mixer.sound(&clip, SoundSpec { envelope: Some(env), ..SoundSpec::default() });
```

- The gain runs 0 to 1 over `attack_ms`, falls to `sustain` (0 to 1) over `decay_ms`, holds there, and on `stop` falls from wherever it stands to 0 over the release. Default `Envelope`: no attack or decay, sustain 1, the shortest release.
- `Shape::Linear` moves in a straight line; `Shape::Exponential` moves fast first and settles (the curve is (1 - e^-5p) / (1 - e^-5), so it lands exactly on its target). Both are continuous at every stage boundary.
- Stages count frames of the voice's own playing, so a voice started later with `play_at` starts its attack when it starts.
- No click: an attack or release is never shorter than 2 ms, so a zero time is a 2 ms edge, and the first sample is always 0. A square tone started with an envelope begins at silence and ends at silence.
- `stop(id, seconds)` on an enveloped voice releases for the longer of the envelope's release and the seconds given, then the voice ends. A voice that never played is dropped at once. `stop_music` and stealing fade as they always did.
- `set_envelope(id, None)` takes the envelope off. Set it right after creating the voice, before the first render: on a voice already playing it starts at the top of its decay, so the gain does not jump. `envelope_level(id)` reads the current envelope gain.
- `SoundSpec::envelope` gives every trigger of a sound its own envelope.
- A clip that reaches its end plays out its tail as recorded; only `stop` releases.

## PCM streams

A voice a game or an editor feeds itself, for audio rendered elsewhere (a play session's `take_audio`, a synth in another thread):

```rust
let (voice, pcm) = output.pcm_stream(ChannelId::EFFECTS, 2, 0.5, Level::default());
let taken = pcm.push(&session.take_audio());
```

- `Mixer::pcm_stream` and `Output::pcm_stream` take the channel, the interleaved channel count of what will be pushed, the ring's capacity in seconds and a `Level`, and return the voice and a `PcmSender`. Samples are f32 at the mixer's rate (`output.rate()`), so a mixer that renders for it is built at that rate. One channel plays on both sides; with more than two, the first two play and the rest are skipped.
- The ring is a fixed buffer allocated with the stream, single producer and single consumer, with no lock: neither side waits on the other. `PcmSender::push` returns the frames it took, which is fewer than offered when the ring is full (the producer decides what to drop; nothing is overwritten) and ignores a trailing partial frame. `queued_frames`, `room_frames`, `capacity_frames`, `channels` and `rate` read the state. The sender is `Send`, so it can live on another thread.
- Latency is what is queued. There is no prebuffer: a frame pushed before a block is rendered plays in that block, so a producer that pushes a tick of audio at a time hears it one block later at most. Keeping the queue short keeps the latency short.
- No click on underrun. When the ring runs dry the voice holds its last frame and fades it to silence over 2 ms (`PCM_EDGE_SECONDS`); when frames return they fade in over the same 2 ms from wherever the fade has got to. The first frames of a new stream fade in the same way. Between 2 ms of silence and a longer gap the output is the same; a gap shorter than 2 ms is bridged by the held frame, and the step across it is the signal's own.
- `finish` (or dropping the sender) ends the stream after the queued frames play and the tail fades out; the voice then ends. `is_connected` is false once the voice is gone (`stop`, a finished stream), so a producer can stop.
- The voice takes the usual level, pan, gain, channel, filters and ducking. Its pitch ratio does nothing, since frames are consumed one for one. `stop(voice, seconds)` fades it as for any voice.

## Filters

A low-pass and a high-pass per channel, second order (Butterworth, Q 0.707), 3 dB down at the cutoff:

```rust
mixer.set_channel_lowpass(ChannelId::MUSIC, Some(600.0));
mixer.set_channel_highpass(ChannelId::EFFECTS, Some(300.0));
mixer.glide_channel_lowpass(ChannelId::MUSIC, None, 1.0);
```

- `Some(hz)` ramps the cutoff there in log frequency from the open position (the filter starts open); `None` ramps it open again and then switches the filter off, so a closed filter leaves the audio bit for bit alone.
- `set_*` ramps over `FILTER_RAMP_SECONDS` (30 ms), `glide_*` over the time you give.
- A filter that has been idle starts from the steady state of its first sample, so it adds no step.

## Drive

A saturation stage per channel, after the filters and before the channel's volume, so the fader does not change how hard the channel is driven. Off until a game sets it; off costs nothing and leaves the audio bit for bit alone.

```rust
mixer.set_channel_drive(ChannelId::EFFECTS, Some(Drive {
    drive_db: 18.0,
    curve: DriveCurve::Tanh,
    mix: 0.8,
    compensate: false,
    output_db: -3.0,
}));
mixer.glide_channel_drive(ChannelId::EFFECTS, 36.0, 1.5);
mixer.set_channel_drive(ChannelId::EFFECTS, None);
```

- `drive_db` multiplies the signal before the curve (-24 to 60 dB). `curve` is `Soft` (x / (1 + |x|), the gentlest), `Tanh` (a rational tanh: x (27 + x^2) / (27 + 9 x^2), exactly 1 from |x| = 3 on) or `Hard` (a clamp to -1 and 1). Every curve is plain adds, multiplies and divides, so the bits are the same on every platform.
- `compensate: true` divides the curved signal by the drive's gain, so a quiet signal comes out at the level it went in and only the loud peaks are bent; `false` leaves the curve's output as it is, which is louder and flatter, the one for a braam. `output_db` (-60 to 24 dB) scales the wet signal after that.
- `mix` is dry against wet, 0 to 1, equal power (the square roots of `1 - mix` and `mix`). Soft, hard and the drive-and-mix blend are ported from kira's `Distortion` effect (MIT or Apache-2.0).
- `set_channel_drive(channel, Some(drive))` fades the wet signal in over `RAMP_SECONDS` (10 ms) from nothing and moves `drive_db` there if already on; `None` fades it out and then switches the stage off. `glide_channel_drive(channel, drive_db, seconds)` ramps the drive amount in dB (a ramp in dB, so it moves evenly to the ear); it does nothing while the stage is off. `channel_drive(channel)` reads the target. `Output` mirrors all three.

## Scheduled glides

A glide that starts at a future frame, in the same terms as `trigger_in` and `trigger_at`:

```rust
mixer.glide_channel_lowpass_in(offset, ChannelId::MUSIC, Some(400.0), 0.8);
mixer.glide_pitch_at(mixer.clock() + 24_000, voice, 0.5, 2.0);
```

- `glide_channel_lowpass`, `glide_channel_highpass`, `glide_channel_pitch`, `glide_channel_drive`, `glide_pitch`, `glide_gain` and `glide_tone_frequency` each have an `_in` form (frames after `Mixer::clock()`, the next block's first frame at 0) and an `_at` form (an absolute frame), with the arguments of the glide they schedule. `Output` mirrors them all, plus `Output::clock()` for the `_at` forms.
- The block is cut at the frame, so the glide starts exactly there, not at the block's edge: the output is the same, bit for bit, as calling the plain glide when the clock reaches that frame. A frame already past is applied at the start of the next block. Events at the same frame apply in the order they were scheduled.
- It applies the plain glide then, to whatever the target is by then: a voice that has ended is skipped, a channel or voice that did not exist is as in the plain call. `scheduled_glides()` counts the ones still waiting.
- With nothing scheduled the block is rendered in one piece, as before.

## Vorbis (feature `vorbis`)

The feature decodes Ogg Vorbis in pure Rust with [lewton](https://crates.io/crates/lewton) 0.10.2 (pinned). lewton over symphonia: it is only an Ogg Vorbis decoder, about four small crates against symphonia's container and codec framework, and it is all the games need. It is off by default; other media files go through the `media` feature below.

```rust
let clip = Clip::from_vorbis(&bytes)?;

let source = VorbisSource::File(path);
mixer.play_music_vorbis(&source, Some((loop_start, loop_end)), 2.0, Level::default())?;
mixer.stream_vorbis(ChannelId::DIALOG, &source, None, Level::default())?;
```

- `Clip::from_vorbis` decodes whole, trimmed to the stream's final granule position (lewton alone leaves the last page's tail on). It refuses a file of more than 8 channels, and stops with an error once the clip would hold more than 57,600,000 samples (10 minutes of 48 kHz stereo), so a small crafted file cannot expand into gigabytes; a sound that long streams instead.
- The stream decodes on its own thread, two seconds ahead of the mixer, and resamples to the mixer's rate with the same windowed-sinc as pitch (below). Looping streams (frames of the file's own rate) re-open the source and skip to the loop start, so the wrap is sample exact; the thread exits when its voice is dropped.
- `VorbisSource` is bytes held in memory or a file path.
- Stereo and mono play as they are; a file with more channels plays its first two.

## Media files (feature `media`)

`Decode::start(&DecodeSpec)` and `Output::stream` decode a media file to f32 PCM at the spec's rate and channel count, on their own thread two seconds ahead of the mixer, through [pito-codecs](https://github.com/gmrdad82/pito-codecs) v1.16.3 (MIT, pinned by tag, CPU decoders only); no other program runs. The feature is off by default and the `pfx` tool turns it on. The build takes pito-codecs' audio-only set (`audio`, AAC, AC-3, MP3, Opus, MP4 and MKV), so no video decoder or GPU code is compiled in.

```rust
let spec = DecodeSpec {
    media: "reel.mov".into(),
    start: 1.5,
    length: 10.0,
    rate: 48_000,
    channels: 2,
    tracks: vec![(1, 0.4), (2, 1.0)],
};
let voice = output.stream(&spec, Level::default())?;
```

- MP4, MOV, MKV and WebM tracks in AAC, Opus, AC-3, E-AC-3, MP1, MP2, MP3 or PCM are read sample-exact from `start` for `length` seconds through pito-codecs' `audio_stream`, one stream a track, which pfx-sound mixes by the tracks' gains, and resampled with its windowed-sinc.
- WAV (PCM 8, 16, 24 and 32-bit, float 32 and 64-bit), raw MP3 (MPEG-1, 2 and 2.5, layers I to III, an ID3v2 tag skipped) and raw ADTS AAC are decoded whole, then shaped to the channel count, resampled and cut the same way.
- `tracks` names audio tracks by their order among the file's audio tracks, 0 first, each with a gain; the tracks it leaves out are silent and the mix is stereo. Empty, the first audio track plays at `channels`. Mono becomes stereo on both sides and more channels fold down with pito-codecs' downmix.
- Refused with a plain reason when `start` opens: Ogg (the `vorbis` feature reads Ogg Vorbis), FLAC, ALAC, DTS and any other codec pito-codecs does not decode, an audio track the file lacks, and more than two channels from a track with another count.

What changed from the ffmpeg reader before it: `DecodeSpec` has no `ffmpeg` path, `tracks` count audio tracks instead of a container's stream numbers, the default is the first audio track instead of ffmpeg's pick, and a raw MP3's Xing frame and LAME encoder delay play as leading silence instead of being trimmed (47 ms on a LAME 192 kbit/s file; past it the samples are within 4e-7 of ffmpeg's). A raw ADTS AAC file decodes within 8e-4 of ffmpeg's, sample for sample.

Cost, measured once on 10 s of 48 kHz stereo at 192 kbit/s (release build, Ryzen 9 9950X under a load average near 30, three runs): a raw MP3 decodes whole in 8 to 17 ms; an AAC track in MP4 opens in 20 to 32 ms, most of it pito-codecs' one-time look for a GPU decoder, and streams through in 18 to 47 ms. ffmpeg as a child process took 40 to 44 ms and 35 to 38 ms for the same files.

## Voice effects (feature `voicefx`)

`pfx_sound::voicefx` holds offline building blocks for baking voices from recordings, once, in a tools build; nothing in it runs live. Every call takes interleaved `&[f32]` with its channel count and rate and returns a new `Vec<f32>`; the same input and seed give the same output. The feature is off by default.

### Pitch, formants and time (`voicefx::stretch`)

A port of Signalsmith Stretch 1.4.0 and the STFT and FFT of Signalsmith Linear 0.6.4 (MIT, see [NOTICE.md](../NOTICE.md)): a phase vocoder that predicts each bin's phase from the previous block and its neighbouring bins, maps frequencies through the spectrum's peaks so harmonics move together, and keeps a smoothed spectral envelope it can move on its own, so pitch and formants shift independently.

- `pitch_shift(samples, channels, rate, semitones)`: the same length, every frequency moved by `2^(semitones/12)`. Formants move with the pitch (a raised voice sounds smaller); keep them with the struct below.
- `formant_shift(samples, channels, rate, semitones)`: the same length and pitch, the spectral envelope moved by `2^(semitones/12)` (up sounds smaller, down sounds bigger).
- `time_stretch(samples, channels, rate, factor)`: `round(frames * factor)` frames out at the same pitch; `factor` above 1 is slower. Past 2x the bins' phase steps are drawn at random, seeded.
- `Stretch::new(channels, rate)` (or `with_preset(channels, rate, Preset::Cheaper)`, or `with_blocks(channels, rate, block, interval)`), then `set_transpose_semitones(semitones, tonality_limit_hz)` (or `set_transpose_factor`; a limit above 0 stops shifting above that frequency, keeping the hiss of a voice where it was), `set_formant_semitones(semitones, formant_compensation)` (or `set_formant_factor`; with `formant_compensation` the envelope is moved back against the pitch shift, so a pitch shift keeps the formants), `set_formant_base(hz)` (a rough fundamental for the envelope; 0, the default, estimates it), `set_seed(seed)`, and `process(input, output_frames)`: the whole buffer in one call, any input length to any output length, so one call shifts pitch and stretches time together.
- Presets: `Default` is 120 ms blocks every 30 ms, `Cheaper` 100 ms blocks every 40 ms. All channels are analysed together and stay phase locked to the loudest one, so stereo images hold.
- Input shorter than about one block (120 ms at the default) is padded with silence for the analysis and cut back to the asked length.

The port matches the C++ original bit for bit: built with g++ 16 at `-O2 -ffp-contract=off` against the same pinned headers and fed the same synthesised signals (a 100 Hz to 4 kHz sweep, a 140 Hz vowel-like harmonic tone with three formants, white noise, a stereo pair), eleven cases (pitch up and down, formant only, pitch with compensated formants, pitch and formant apart, a tonality limit, stretches of 0.6, 1.25, 1.5 and 3, the cheaper preset) gave a maximum difference of 0 over every sample, the 3x stretch with the C++ given the same seeded generator.

Cost, measured once on 3 s of a 48 kHz mono voice-like tone, one thread, release build, Ryzen 9 9950X, in seconds of processing per second of audio:

| Call | Cost |
|---|---|
| `pitch_shift` +5 | 0.0094 |
| `formant_shift` +4 | 0.0074 |
| `time_stretch` 1.5 | 0.0107 |
| `time_stretch` 0.75 | 0.0054 |

The cost grows with the output length and the channel count; the C++ original takes about the same.

`crates/sound/tests/stretch.rs` guards the length kept by `pitch_shift` and `formant_shift`, a 440 Hz sine moved by 7, -5 and 12 semitones within 1 % (and kept by a formant shift and a time stretch), `time_stretch`'s output length at 0.5, 1.5 and 3, and the same output for the same input and seed.

### Ring, crush, chorus, flanger, vocoder and syllables


```rust
use pfx_sound::voicefx::{chorus, crush, flanger, ring, syllables, vocoder};

let robot = vocoder::vocoder(&voice, 1, 48_000, vocoder::Carrier::Wave { wave: Wave::Saw, hz: 110.0, seed: 0 }, &vocoder::Vocoder::default());
let radio = crush::crush(&voice, 1, 48_000, &crush::Crush { bits: Some(6), hold: 6.0, average: true, low_pass: Some(4_000.0) });
let alien = ring::ring(&voice, 1, 48_000, &ring::Ring { hz: 90.0, mix: 0.6, feedback: 0.0 });

let found = syllables::syllables(&voice, 1, 48_000, &syllables::Split::default());
let grains: Vec<Vec<f32>> = syllables::cut(&voice, 1, &found)
    .iter()
    .map(|g| syllables::repitch(g, 1, 1.2, Resample::Cubic))
    .collect();
let mumble = syllables::join(&syllables::reorder(&grains, seed), 1, 48_000, 0.01);
```

- `ring::ring` multiplies by a sine carrier at `hz`, with `feedback` (0 to 1, scaled by 0.95) of the last output and `mix` dry against wet. Ported from mda RingMod.
- `crush::crush` holds each sample for `hold` frames (fractional, so 6.0 at 48 kHz is an 8 kHz rate; `average: true` holds the mean of the frames it replaces instead of the first), then truncates it to `bits` (1 to 24; levels at multiples of 2^-(bits-1)), then, with `low_pass: Some(hz)`, runs it through an 8-pole low-pass. Ported from mda Degrade.
- `chorus::chorus` mixes four taps of a delay line: tap k sits at `separation` x k seconds plus up to `depth` seconds, swung by its own seeded fractal noise at `rate` Hz (each tap 0.02 Hz faster than the last); `feedback` (-1 to 1, through a tanh) feeds their mean back into the line, `mix` is dry against wet, and the default 0.8 is fundsp's chorus exactly. The same `seed` gives the same swing.
- `flanger::flanger` reads one tap swung by a sine at `rate` Hz between `delay` and `delay + depth` seconds, `feedback` (-1 to 1, through a tanh) feeds it back, and `mix` is dry against wet. Both are ported from fundsp.
- `vocoder::vocoder` is a channel vocoder: the modulator (the voice) is split into 8 or 16 bands (`bands`) by 4th-order resonators run at half rate, an envelope follower on each band (`envelope_ms`) sets that band of the carrier, and a high band above the bank lets sibilants through (`hi_band` from the carrier, `hi_thru` from the voice, both 0 to 1). `q` (0 to 1) narrows the bands, `range_hz` (800 by default) moves the bank up or down in proportion, `output_db` trims the level (the default is about 10 dB above the voice; at 16 bands, trim it before mixing). The carrier is a `Wave` oscillator from pfx-sound (`Saw`, `Square`, `Sine`, `White`, `Pink`) at `hz` with a `seed` for the noises, or any mono buffer, looped; each channel of the modulator runs on its own. Ported from mda Vocoder; its envelope speeds are rescaled from the 44.1 kHz they were tuned at.
- `syllables::syllables` cuts speech into syllable-sized grains, `Vec<Syllable { start, end }>` in frames: the RMS of each `hop` (10 ms) against the loudest hop opens a grain at `open_db` (-30 dB) and closes it below `close_db` (-40 dB), grains closer than `min_gap` (40 ms) merge and grains shorter than `min_length` (60 ms) are dropped. `onsets: true` also splits a grain where spectral flux over the vocoder's 16 bands peaks (librosa's onset detection and constants), for syllables that run into each other without a dip.
- The grain helpers: `cut` copies the grains out, `reverse` plays one backwards, `repitch(grain, channels, ratio, Resample)` changes its pitch by resampling (1.5 is a fifth up and two thirds as long; `Sinc`, `Cubic` or `Linear`, as for playback), `permutation(len, seed)` and `reorder(items, seed)` shuffle them by a seed, and `join(grains, channels, rate, crossfade)` puts them back end to end with an equal-power crossfade of `crossfade` seconds (shortened to fit a grain shorter than that); the result is the grains' total length less the overlaps.

Cost, measured once on 10 s of 48 kHz mono voice-like audio (a gliding 140 Hz tone with breath noise, gated four times a second), release build, Ryzen 9 9950X, CPU time per second of audio:

| Call | Cost |
|---|---|
| `ring` | 0.55 ms |
| `crush`, 8 bits | 0.34 ms |
| `crush`, 6 bits, hold 6, averaged, 4 kHz low-pass | 0.28 ms |
| `chorus` | 1.9 ms |
| `flanger` | 2.0 ms |
| `vocoder`, 8 bands, saw carrier | 1.9 ms |
| `vocoder`, 8 bands, white noise carrier | 0.85 ms |
| `vocoder`, 16 bands, saw carrier | 3.8 ms |
| `syllables` | 0.03 ms |
| `syllables` with `onsets` | 0.73 ms |
| `cut`, `reorder`, `join` (10 ms crossfades) | under 0.05 ms each |
| `reverse` every grain | 0.07 ms |
| `repitch` every grain by 1.3, `Sinc` | 12.8 ms |
| `repitch` every grain by 1.3, `Cubic` / `Linear` | 0.38 / 0.29 ms |

## Cleaning recordings (feature `clean`)

`pfx_sound::clean` cleans a field recording offline: an engine, a motor or a voice recorded outdoors, with wind, barks, door slams or voices on top. It is off by default (`clean` turns on `voicefx`, whose half-bin real FFT it reuses) and the `pfx` tool builds it in (`sound-clean`, a default feature). `clean::clean(&clip, &Options)` returns a `Cleaned`: the cleaned `Clip` at the input's length, rate and channel count, a `Report` of what was removed, the `Masks` it applied and `preview()`, an RGB image of the masks. Nothing reads the clock or a global generator, so the same input and options give the same samples. Several channels are analysed as their mean and every channel gets the same masks.

```text
pfx sound clean <in.wav> <out.wav> [--keep harmonic [--f0 LOW,HIGH]] [--remove transients] [--gate [--noise START,END] [--reduce DB]] [--mask <png>] [--json]
```

The modes combine and run in this order, each on the output of the one before:

- **`--keep harmonic`** (`Options::harmonic`, `Harmonic { low_hz, high_hz }`, default 40..400 Hz, `--f0 LOW,HIGH`) follows a moving fundamental, an engine's firing frequency, a motor or a voice, and keeps only its harmonic series. On 170 ms frames every quarter frame (8192 samples at 48 kHz), each candidate on a 1/96-octave grid is scored by subharmonic summation (Hermes 1988, weights 0.84 per harmonic, up to 30 harmonics below 5 kHz): each harmonic's peak against the loudest bin in the middle half of the gap to the next, so twice, three times or half the fundamental score low. A Viterbi path through the scores, at 0.5 dB a grid step and at most 8 steps a frame (after pYIN), keeps the track continuous; parabolic interpolation refines it. The mask is a comb along the track: each harmonic keeps its main lobe (1 bin each side, widened by how far it drifts across the window), a raised-cosine edge of 1.5 bins, and everything off the series drops to -60 dB. The comb only separates what the window resolves: below about 3 bins between harmonics (18 Hz at this size) it keeps everything.
- **`--remove transients`** (`Options::transients`) finds short events that stand out of the background and inpaints them. On 85 ms frames every quarter frame (4096 samples at 48 kHz), the background is a median over one second per frequency (librosa's harmonic median, `reflect` edges) of the spectrum max-filtered over 8 % of each frequency, so a gliding partial stays background. A frame is part of an event when over 1 % of the bins from 100 Hz to 12 kHz stand 12 dB over the background; the event grows while clustered bins stand 6 dB over it, takes 2 frames before it and, after it, a hidden tail extrapolated from its own decay (20 dB under the background, at least 4 frames, at most the event's length). Events longer than 2 s are left alone. Inside an event every bin from half an octave under its lowest novel cluster to the top is replaced: its level interpolated in dB between the two frames each side, its phase carried on at the bin's measured advance where the bin was steady before the gap (under 0.5 rad of change), a seeded phase where it was noise. The level is kept, the waveform under the event is not: what is filled is the background's continuation, not the samples the bark covered.
- **`--gate`** (`Options::gate`, `Gate { noise, reduce_db }`) is noisereduce's stationary spectral gate: 1024-sample frames every 256, the noise print's mean and standard deviation in dB per frequency (80 dB range per frequency), a threshold 1.5 standard deviations over the mean, a 0/1 mask scaled by `prop_decrease` and smoothed by a triangular kernel 500 Hz by 50 ms. The print comes from `--noise START,END` (seconds) or, without it, the quietest second of the input (the whole input when it is under 2 s); it is measured on the output of the modes before. Without `--reduce` the gate is full (`prop_decrease` 1.0, noisereduce's default); `--reduce DB` sets `prop_decrease = 1 - 10^(-DB/20)`. One change from noisereduce: the smoothing renormalises at the spectrum's and the clip's edges instead of padding with zeros, which darkened the lowest bins (an engine's fundamental) and the first and last 50 ms. Like noisereduce, it gates whatever stands only a few dB over the print's threshold, so it suits a steady floor (hiss, hum, room tone) better than gusting wind.

The report gives the fundamental's range, median and voiced share (frames whose score passes 3 dB) and the dB the comb removed; each event's start and end in seconds and the dB removed in it; the gate's print, whether it was picked, its amount, and the dB it removed overall and in the print; and the input's and output's RMS level. `--json` prints it as one object (`clean`, `input`, `output`, `mask`, `seconds`, `rate`, `channels`, `input_db`, `output_db`, `ms`, `harmonic`, `transients`, `gate`; a mode not asked for is `null`). `--mask <png>` writes the preview: time across (one column a frame, at most 2000), log frequency up from 20 Hz to Nyquist on 400 rows, the input's level over 80 dB as brightness, what was kept grey and what was removed red. A 16-bit input is written back as 16-bit, anything else as 32-bit float. `Masks::apply(&clip)` replays the masks' gains (without the fill) on another signal of the same length and rate, which is how the tests measure each part of a mix.

Measured on the generated test signal (below), through the masks unless said otherwise:

| Mode | Result |
|---|---|
| `--remove transients`, engine + barks + wind | the four barks down 61.3, 59.4, 57.6 and 56.1 dB in their time ranges; the engine and wind 0.07 dB from where they were over the whole clip (output against input), 0.26 to 0.52 dB inside the barks |
| `--keep harmonic`, engine + wind | the track 45.1 to 75.0 Hz against the true 45 to 75 Hz, voiced 99.6 %; the engine 0.04 dB from its level, the wind down 5.8 dB (its energy sits under the low harmonics, 45 to 75 Hz apart, which the comb keeps) |
| `--gate --reduce 6` and `--reduce 12`, engine + steady hiss | the hiss down 5.74 and 11.22 dB where it is alone, the engine 0.03 and 0.05 dB from its level |
| all three, engine + barks + breeze and hiss, print 0.1..0.9 s, `--reduce 10` | the barks down 58.6, 58.6, 52.0 and 56.2 dB, the engine 0.58 dB from its level, the wind and hiss down 7.1 dB |

Cost, measured once on 10 s of 48 kHz mono, one thread, release build, Ryzen 9 9950X under a load average of 24: `--keep harmonic` 83 ms, `--remove transients` 91 ms, `--gate` 54 ms, all three 229 ms (about 23 ms a second of audio), and the mask preview 13 to 27 ms. The cost grows linearly with the length and the channel count (each channel is resynthesised).

`crates/sound/tests/clean.rs` generates its signal: an engine (a fundamental gliding from 45 to 75 Hz with a 1 Hz wobble at 0.3 Hz, harmonics at h^-0.7 fading out by 4 kHz), four barks (0.25 s harmonic bursts falling from 650 to 480 Hz under three formants at 1, 2.2 and 3.3 kHz, an 8 ms attack and a 70 ms decay), wind (seeded noise through two 250 Hz poles, gusting 30 % at 0.2 Hz) and hiss (seeded noise under 6 kHz). It guards the barks at least 30 dB down with the engine within 1 dB, the events found where the barks are, the tracked range, the gate lowering steady noise by its amount within 1.5 dB with the engine within 1 dB, the length kept and the same output twice; `cost_of_ten_seconds` (ignored) measures the cost. A unit test guards the STFT giving its input back at unit gains.

## Shot mix (feature `shot`)

`pfx_sound::shot` mixes a rendered shot's sound offline from a `*.sound.toml` script: curves over the shot's time drive synthesised layers, and WAVs the user supplies are placed on top. Everything is synthesised from pfx-sound's own blocks, seeded; no third-party samples ship.

```rust
let mix = pfx_sound::shot::render(Path::new("shot.sound.toml"), 600, 60)?;
let wav = Clip { rate: pfx_sound::shot::RATE, channels: 2, samples: mix }.to_wav();
```

- `render(script_path, frames, fps)` returns 48 kHz (`RATE`) stereo interleaved samples, exactly `frames / fps` seconds long: `audio_frames(frames, fps)` = round(frames x 48000 / fps) frames, so 600 frames at 60 fps are 480,000 frames. Frame n of the picture is time n / fps on the script's clock.
- `Script::load(path)` and `Script::parse(text, dir)` read a script into `Script` (curves, layers, samples, all public fields), and `Script::mix(audio_frames)` renders it, so a caller can change a value before mixing. Unknown keys, a curve named twice and a layer reading a curve that does not exist are errors, with the table and key named.
- The same script gives the same samples, bit for bit: time comes from the frame count, every noise and event draw from the script's `seed` (each layer takes its own seed from it and its place, or `seed` on the layer), and the sines, exponentials and powers go through pfx-sound's sine table and pfx-core's `sim` functions, never the platform's libm.
- The mix is the layers summed, then `[mix] gain`, then the soft limiter if `limiter` is set (the mixer's own `set_limiter` curve), then a clamp to -1..1.

### `pfx sound mix`

```
pfx sound mix <script.sound.toml> --frames N [--fps N] --out <out.wav> [--json]
```

renders a script to a WAV with no scene and no GPU: `render(script, N, fps)` (`--fps` 60 by default and the same rates as `pfx shot`'s, `24000/1001` and `23.976` included; the length is N × 48000 × den / num samples, rounded to the nearest), written as a 48 kHz stereo 32-bit float WAV, the very function and writer `pfx shot --audio` uses, so the two give the same bytes for the same script, frames and fps (a test in `src/sound.rs` checks it byte for byte at 90 frames at 30 fps and 37 at 24). The output's folder is made if missing. It prints one line (the frames, the length, the rate and the time it took), or with `--json` an object with `mix` (1), `script`, `output`, `frames`, `fps`, `samples` (frames of audio), `seconds`, `rate`, `channels` and `ms`. Input it can't parse is refused the way clap refuses it (exit 2, `Usage: pfx sound mix [OPTIONS] --frames <FRAMES> --out <OUT> <SCRIPT>`); a script that fails to load is one line, `pfx: …`, exit 1. The car-pass example, 600 frames at 60 fps (10 s of audio), mixes and writes in 155 ms, median of five runs (release build, one core, Ryzen 9 9950X under a load average of 24).

### Script keys

| Key | Default | Meaning |
|---|---|---|
| `seed` | 0 | the script's seed |
| `[mix] gain` | 1 | master gain |
| `[mix] limiter` | off | soft limiter threshold, 0.05 to 0.999 |
| `[[curve]] name` | required | the name layers read it by |
| `[[curve]] keys` | required | `[[t, v], ...]`, t in seconds, not going backwards; held before the first key and after the last |
| `[[curve]] ease` | `"linear"` | `"linear"`, or `"smooth"`: a monotone cubic (Fritsch-Carlson) that passes every key and never overshoots between them |

Every layer table (`[[engine]]`, `[[road]]`, `[[squeal]]`, `[[tyre]]`, `[[spray]]`, `[[whoosh]]`, `[[ambience]]`) takes these, and any key marked *curve* below takes either a number or a curve's name:

| Key | Default | Meaning |
|---|---|---|
| `name` | none | a label |
| `gain` | 1 | *curve*; linear gain, ramped inside each 16-frame control block |
| `pan` | 0 | *curve*; -1 left to 1 right, equal power on mono layers, a balance on stereo ones (whoosh, ambience) |
| `start`, `end` | 0, the shot's end | seconds the layer plays between |
| `fade_in`, `fade_out` | 0 | seconds of smoothstep fade after `start` and before `end` |
| `seed` | from the script's | the layer's own seed |
| `highpass`, `lowpass` | off | *curve*; the layer's own high-pass and low-pass, cutoff in Hz, 10 to 24000 (a curve's values are held to 5 Hz and 0.49 x the rate as it plays); each a 12 dB an octave biquad (RBJ, the mixer's channel filter), applied to the layer's sound before its gain and pan, so `pfx sound mix` and `pfx shot --audio` give the same bytes. A keyed cutoff glides on a log scale through each 16-frame block, the coefficients worked out again at every sample while it moves, so a long sweep has no zipper. On `[[engine]]`, `lowpass` stays the engine's own load-following low-pass (a number) and only `highpass` is the layer's |
| `highpass_q`, `lowpass_q` | 0.707 | the filter's Q, 0.1 to 10 (0.707 is flat, Butterworth); only with its filter |

| Layer | Keys | What it is |
|---|---|---|
| `[[engine]]` | `rpm` (*curve*, 900), `load` (*curve*, 0 to 1, 1), `cylinders` (1 to 16, 8; with `sample`, no synthesised engine unless given), `drive` (dB, 0 to 40, 6), `lowpass` (Hz, 6000), `crackle` (0 to 1, 0) | a combustion engine, ported from enginesound (MIT, see [NOTICE.md](../NOTICE.md)): a four-stroke crank turns at `rpm` / 120 cycles a second and each cylinder fires once a cycle, so the firing rate is rpm x cylinders / 120 Hz; each cylinder's piston and ignition pulse excites valve-modulated intake, exhaust and extractor waveguides into collectors, a straight pipe and four muffler elements. `load` scales the ignition (35 % to 100 %), the intake and the exhaust, so more load means more upper harmonics; the drive (a rational tanh, `drive` x 40 % to 100 % by load) and a low-pass at `lowpass` x 30 % to 100 % by load follow, and a 25 Hz high-pass. With `crackle`, exhaust pops (seeded 3 to 12 ms band-passed noise bursts) fire on lift-off (a falling load) and on overrun (low load above 2,500 rpm). The model runs 0.5 s before the layer starts, so it opens warm |
| `[[road]]` | `speed` (*curve*, m/s, 0), `roughness` (0 to 1, 0.4), `surface` (`"asphalt"`), `load` (*curve*, 0 to 1, 1), `spacing` (m, 0.02 to 5, 0.2) | tyre road noise: pink noise low-passed at 180 + 28 x speed Hz, a brown rumble and a band of hiss at 1500 + 45 x speed Hz by `roughness`, at a level of (speed / 25)^1.3; `surface` changes what the tyres roll on (see [Road surfaces](#road-surfaces)) |
| `[[squeal]]` | `at` (`[[t0, t1], ...]`), `curve` (a curve name), `threshold` (0.5), `range` (0.5), `hz` (*curve*, 1400) | a tyre squeal: a tone with its octave and a Q 14 resonant band of noise at `hz`, wobbling 2 % and fluttering; it sounds inside each `at` window and while the curve is above `threshold`, reaching full at `threshold + range`, with a 40 ms attack and 120 ms release |
| `[[tyre]]` (or `[[skid]]`) | `slip` (*curve*, m/s, 0 to 200, 0), `load` (*curve*, 0 to 4, 1), `grip` (m/s, 0.01 to 20, 0.5), `threshold` (m/s, 0.05 to 40, 2), `range` (m/s, 0.05 to 40, 3), `hz` (200 to 4000, 1000), `rise` (Hz per m/s, 0 to 400, 25), `scrub`, `chirp`, `squeal` (0 to 4, 1 each) | a tyre slipping over the road, driven by its slip speed: the rubber scrub, a chirp as each slide starts and the squeal above `threshold`, together (see [Tyre slip](#tyre-slip)) |
| `[[spray]]` | `speed` (*curve*, m/s, 0), `wet` (*curve*, 0 to 1, 1) | wet road spray: white noise high-passed at 1.8 kHz and low-passed at 5000 + 120 x speed Hz, fluttering, at wet x (speed / 25)^1.2 |
| `[[whoosh]]` | `at` (`[t, ...]`, seconds of closest approach), `speed` (m/s, 20), `distance` (m, 3), `hz` (700), `sweep` (`[from, to]`, `[-1, 1]`), `length` (s, 2) | passing vehicles: for each time, a source at `speed` passing `distance` away: pink noise band-passed at `hz` and low-passed at 350 Hz, both moved by the Doppler factor 343 / (343 + radial speed), at distance / range, panned from `sweep[0]` to `sweep[1]` as it passes, over `length` seconds |
| `[[ambience]]` | `bed` (1), `rumble` (1), `horns` (per minute, 3), `birds` (per minute, 6) | a stereo city bed: pink noise low-passed at 700 Hz with a little air, a distant traffic rumble under 110 Hz that swells slowly, distant two-tone horns (a square and a saw a major third apart through a 1.4 kHz low-pass, 0.18 to 0.6 s, one in three honking twice) and bird chirp groups (2 to 5 sine chirps at 2.6 to 4.8 kHz with sweeps), placed at seeded Poisson times with seeded pitch, level and pan |
| `[[sample]]` | `path` (required, relative to the script), `at` (s, 0), `gain` (1), `pan` (0), `start`, `end` (s of the file, 0 and its end), `fade_in`, `fade_out` (s, 0), `loop` (false), `loop_crossfade` (s, 0.01), `length` (s) | a WAV the user supplies (their own or CC0), resampled to 48 kHz stereo, trimmed to `start`..`end` of the file and added from `at`. Without `loop` it plays once (cut to `length` if given); with `loop` it repeats until `length` or the shot's end, each pass from `loop_crossfade` seconds in, the clip's last `loop_crossfade` seconds fading out (equal power) under its first ones fading in, so the seam has no step; `fade_in` and `fade_out` are smoothstep fades from `at` and before the placed end. Anything past the shot's end is cut |

### Road surfaces

`[[road]] surface` is one name, `"asphalt"` (the default), `"gravel"`, `"wet"` or `"cobbles"`, or a list of timed names, `[[t, "name"], ...]` with t in seconds not going backwards, read like a curve of surfaces: the first name holds before its time and the last after its time, two neighbouring keys with the same name hold it between them, and two with different names crossfade (equal power) over the whole time between them, so `[[0, "asphalt"], [3, "asphalt"], [3.5, "gravel"]]` leaves the asphalt for gravel over half a second. A list of names reads better than one weight curve per surface: the weights always sum to one in power and a script cannot leave the road silent by mistake.

| Surface | Sound |
|---|---|
| `asphalt` | the tyre noise above, unchanged: a road without `surface`, or with only `"asphalt"`, mixes bit for bit as before |
| `wet` | the asphalt plus the `[[spray]]` hiss at full wetness (the same generator: white noise high-passed at 1.8 kHz and low-passed at 5000 + 120 x speed Hz, fluttering, at 0.8 x (speed / 25)^1.2) |
| `gravel` | the asphalt at 40 %, plus stone crunch: seeded impulse grains at 60 a metre of travel x (50 % to 100 % by `load`), each a 0.3 to 1 impulse into one of four Q 3 band-passes at 900, 1500, 2400 and 3600 Hz moved up by 0.7 + 0.5 x min(speed / 25, 1.5) x the load share (brighter with speed and load), one in sixteen a pop into a Q 2 band-pass at 300 to 600 Hz, at a level of min(speed / 25, 1.5)^0.8; plus stones kicked into the wheel arches: 0.3 a metre, each an impulse ringing three Q 40 band-passes at 2.5 to 4.2 kHz x 1, 1.47 and 2.09 (a small metallic tick) |
| `cobbles` | the asphalt at 70 %, plus a thump at every stone: a stone every `spacing` metres along the distance travelled (each joint moved by up to 5 % of the spacing, seeded, without drifting), so the thumps come speed / spacing times a second; each is a 6 ms decaying kick into a Q 2.5 body resonance at 90 Hz and, at 30 %, a 1.8 kHz click, at a level of min(speed / 25, 2)^0.9 |

The gravel, cobbles and wet generators draw their seeds after the asphalt's, only when the surface list names them, and run through the whole layer once named, so a crossfade picks up a running sound. A 10 s road at 20 m/s through all four surfaces costs 37 ms against 10.5 ms for asphalt alone (release build, Ryzen 9 9950X, best of five).

```toml
[[road]]
speed = "speed"
load = "load"
surface = [[0.0, "asphalt"], [4.0, "asphalt"], [4.3, "cobbles"], [7.0, "cobbles"], [7.5, "wet"]]
spacing = 0.15
```

### Tyre slip

`[[tyre]]` gives a tyre that slips (`[[skid]]` is the same layer under another name). Its `slip` curve is the slip speed, how fast the contact patch slides over the road in m/s (0 while the tyre grips; a few m/s in a hard corner; the road speed for a locked wheel), and `load` the normal load as a share of the tyre's usual one. Three parts sound together, each with its own level key:

| Part | Sound |
|---|---|
| `scrub` | the broadband rubber scrub: pink noise with a little white, low-passed at 500 + 400 x slip Hz (up to 14 kHz, so it brightens as the slide speeds up) and high-passed at 120 Hz, with a 60 Hz seeded grain, at a level of min(slip / 10, 4)^0.8 x sqrt(load); silent at no slip |
| `chirp` | a short transient as slip crosses `grip` going up, the moment the tyre breaks from grip into a slide: a tone at `hz` (seeded within 6 %) gliding down from 1.5 x `hz` with its octave, and a snap of noise band-passed at 2.6 kHz, rising over 2 ms and dying over a seeded 40 to 80 ms, louder when the slip rises fast (half strength at a slow creep, full at 10 m/s each second or faster); it fires again only after slip falls below half of `grip`, and not at all for a slide already under way when the layer starts |
| `squeal` | the tonal squeal: the `[[squeal]]` layer's tone with its octave and its Q 14 resonant band of noise, wobbling and fluttering, at `hz` + `rise` x (slip - `threshold`) Hz; it sets in above `threshold`, reaching full at `threshold + range`, with a 40 ms attack and a 120 ms release as grip returns, and is silent at or below `threshold` |

A heavier load makes all three louder (by sqrt(load)) and the chirp and squeal higher (by 1 + 0.15 x (load - 1), between 0.8 and 1.3 times). `grip` above `threshold`, and any key outside its bounds (a fixed `slip` or `load` included), are refused with the table and key named; a curve's values are held to the bounds as it plays. The model follows the friction picture of squeal: a tyre in a sliding contact sticks and slips, the pulses of slip set the pitch, and the squeal lives between full grip and the slide; the curve squeal of rail wheels (Heckl and Abrahams, Journal of Sound and Vibration 229(3), 2000) is the same mechanism. No code is ported: the parts are pfx-sound's own noise, state-variable filters and squeal tone.

```toml
[[curve]]
name = "slip"
keys = [[0.0, 0.0], [2.0, 0.0], [2.1, 4.0], [3.5, 7.0], [4.0, 0.0]]

[[tyre]]
slip = "slip"
gain = 0.8
```

### A recorded engine

An `[[engine]]` with `sample = "<file.wav>"` plays a real recording of any car or engine, extended to any length and rpm. It is a key of `[[engine]]`, not a layer of its own, because it shares the engine's rpm and load curves, its drive, filter and crackle, and blends with the synthesised engine in the same voice. With `sample` and no `cylinders` the recording plays alone; with both, the two blend.

| Key | Default | Meaning |
|---|---|---|
| `sample` | none | the recording, a WAV relative to the script, read by pfx-sound's WAV reader, resampled to 48 kHz and mixed to mono |
| `idle` | detected | `[start_s, end_s]` of a steady idle in the recording |
| `idle_rpm` | detected | the rpm of that idle |
| `revs` | none | `[[rpm, start_s, end_s], ...]`: stretches held at a known rpm |
| `keep_formants` | true | keep the recording's tone across large rpm changes (below) |
| `reach` | 1.25 | with `cylinders`: how far past the highest recorded rpm the recording reaches before the synthesised engine takes over |
| `blend` | 0.5 | *curve*; with `cylinders`, the synthesised engine's share, 0 (the recording only) to 1 (the synthesis only) |
| `crank`, `crank_at` | none, the layer's `start` | `[start_s, end_s]` of a start-up in the recording, played once at `crank_at` |
| `shutdown`, `shutdown_at` | none, so it ends at the layer's end | `[start_s, end_s]` of a shut-down, played once from `shutdown_at` |

The layer keys `start` and `end` stay the layer's times, so the one-shots take engine words.

- **The idle.** Without `idle`, the recording is cut into 0.2 s blocks and every 1 s window clear of `revs`, `crank` and `shutdown` and louder than 30 % of the loudest block is scored by the spread of its block levels; of the steadiest (within 1.5 times the least spread, up to 8 apart), the one whose autocorrelation is periodic (a peak of 0.5 or more) at the longest lag between 25 and 250 ms is the idle. Without `idle_rpm`, that lag is read as one engine cycle (two turns), so idle_rpm = 120 / lag; an engine whose cylinders sound alike may be read at its firing rate instead, so give `idle_rpm` when you know it. With `idle_rpm` (or a rev's rpm) the cycle is refined by autocorrelation within 15 % of 120 / rpm, on a 4x decimated copy and then at the full rate with a parabolic peak.
- **Levels.** The whole recording is scaled so the idle's RMS is 0.14, close to the synthesised engine's level before the drive, so `drive`, `lowpass` and `blend` mean the same for any recording.
- **Grains.** Each recorded stretch (the idle and each rev) plays through two Hann grains at 50 % overlap, which sum to one, so the stream never dips or clicks. A grain is three engine cycles or 60 ms, whichever is longer, cut down to a quarter of the stretch divided by the largest ratio it will play at (Mutable Instruments Clouds' grain-size limit). Each new grain starts where the grain before it is reading, jumped by a seeded whole number of cycles anywhere inside the stretch, so the splice is in phase and the loop never repeats in order; a grain reading past the stretch's end steps back by whole cycles. Reads are cubic Hermite.
- **Following the rpm: resampling.** Every grain reads its stretch at ratio = rpm / the stretch's rpm, updated each 16-frame control block, so the firing rate follows the curve exactly, sample by sample, with no block latency and with the firing pulses' attacks intact. The Stretch port is not used for this: it is a whole-buffer phase vocoder, it smears the pulses that make an engine sound like one, and its fixed blocks would lag a fast rev. Resampling moves the recording's resonances with the rpm too, which is right for small ratios and turns chipmunk for large ones, so:
- **Large ratios: the Stretch port, once.** With `keep_formants`, before mixing, the gaps between recorded rpms wider than 1.5x (and the curve's range past the lowest and highest recordings, to the curve's own lowest and highest rpm, or to `reach` with `cylinders`) are filled with copies of the nearest recording (nearest in log rpm) pitch-shifted by the `voicefx` Stretch port with formant compensation, evenly spaced in log rpm at most 1.5x apart. Each copy then plays as a stretch of its own, so no grain is resampled by more than 1.5x and the tone keeps its resonances. With `keep_formants = false` only the recordings play, resampled by any ratio.
- **Crossfading.** The stretches crossfade by rpm between the two nearest in log rpm with equal-power weights.
- **The synthesised engine.** With `cylinders`, the synthesised share is `blend`, and above the highest recorded rpm it rises to all of it by a smoothstep in log rpm over `reach` (by default from 1x to 1.25x the top rpm), so the engine keeps sounding where the recording has nothing; the two are summed with equal power (they are uncorrelated).
- **Load.** The sum goes through the engine's own tone: the drive (`drive` x 40 % to 100 % by load), the low-pass (`lowpass` x 30 % to 100 % by load, brighter under load) and the lift-off and overrun `crackle`.
- **One-shots.** `crank` plays from `crank_at`; the running engine is silent until the crank's last 0.15 s and fades in over them (equal power) as the crank fades out. `shutdown` fades in over its first 0.15 s while the running engine fades out, then plays to its end; nothing runs after it.

The same script gives the same samples: the grain jumps draw from the layer's seed, the Stretch copies take seeded draws, and the reads, windows and fades go through pfx-sound's sine table and pfx-core's `sim` functions.

```toml
[[engine]]
sample = "my-car.wav"
idle = [0.5, 3.0]
idle_rpm = 850
revs = [[2500, 4.0, 5.5], [4500, 7.0, 8.0]]
crank = [0.0, 0.5]
cylinders = 6
blend = 0.2
rpm = "rpm"
load = "load"
crackle = 0.5
```

Cost, measured once (`bench_recorded_engine` in `crates/sound/tests/shot.rs`), the generated 4.5 s test recording with an idle at 900 rpm and a rev at 3000, a 10 s curve 900 to 6000 rpm and back, one thread, release build, Ryzen 9 9950X, best of five, the load (WAV read, Stretch copies) included:

| Case | 10 s of audio |
|---|---|
| recording only, `keep_formants` (four Stretch copies) | 98 ms |
| recording only, `keep_formants = false` | 23 ms |
| recording with a 4-cylinder synthesised engine, `blend = 0.3` | 171 ms |

The grains cost about 1 ms a second of audio per stretch; each Stretch copy of a 1 s stretch costs about 19 ms once, at load.

`crates/sound/examples/car-pass.sound.toml` is a neutral example: a V8 pulling through three gears with lift-off crackle, road noise and spray by a speed curve, a squeal through a corner, three cars passing and a city bed; the road is high-passed at 60 Hz and the city bed low-passed by a keyed cutoff, so the example covers the layer filters too. Its curves:

```toml
[[curve]]
name = "rpm"
ease = "smooth"
keys = [[0.0, 2400], [2.2, 6400], [2.45, 4300], [4.8, 6900], [5.05, 4800], [7.6, 7000], [8.2, 3200], [10.0, 2600]]

[[engine]]
rpm = "rpm"
load = "load"
cylinders = 8
crackle = 0.8
```

Cost, measured once (`bench_shot_mix` in `crates/sound/tests/bench.rs`), the example's 10 s mix layer by layer, one thread, release build, Ryzen 9 9950X under other load, best of five:

| Layer | 10 s of audio |
|---|---|
| engine, 8 cylinders | 113 ms |
| road | 8.8 ms |
| squeal | 3.7 ms |
| spray | 7.1 ms |
| whoosh, 3 passes | 3.8 ms |
| ambience | 11 ms |
| the whole example | 143 ms |

The engine costs about 3.3 ms a second of audio for its pipes plus 1.2 ms a second per cylinder.

## True peak and the limiter

`true_peak(samples, channels)` measures interleaved audio's true peak as ITU-R BS.1770-4 defines it: each channel oversampled 4x through the recommendation's 48-tap, 4-phase polyphase filter (Annex 2, its coefficients kept exactly), the largest rectified value, or the largest sample if that is higher; `decibels` turns it into dBTP. `local_peaks` gives each frame the largest of its own samples and the oversampled points on either side of it, and `oversampled` returns one channel's 4x stream. The Annex 2 filter reads a little high near the top of the band: 0.09 to 0.25 dB over a quarter-rate tone's true peak, 0.38 dB at 17 kHz, within 0.3 dB of the ebur128 crate's windowed-sinc meter on every test signal, and over the true value on each of them, which is the safe side for a ceiling.

`Limiter::new(ceiling_db)` holds a master under a true-peak ceiling, offline, over the whole buffer: `master(samples, channels, rate)` leaves audio already under it untouched (and returns false); otherwise each frame's gain is the ceiling over its true peak, linked across channels, held as a moving minimum over `attack` plus `hold` (5 ms each by default), released exponentially over `release` (80 ms) and smoothed by a box over the attack, with the audio read `attack` ahead. It measures the result again, up to four passes, and scales by whatever is left over, so no oversampled point ends above the ceiling. A three-minute stereo master is measured in about 0.28 s and limited in about 1 s on one core (release build).

## Cost

`crates/sound/tests/bench.rs` (`cargo test --release -p pfx-sound --test bench -- --ignored --nocapture`): a 48 kHz stereo mixer rendering 800-frame ticks (60 a second), a 0.1 s mono hit triggered 2,000 times a second with `max_voices: 20`, `Steal::Oldest`, 2 semitones of pitch jitter and 0.2 of volume jitter (about 20 live voices plus the stolen ones fading), and 20 live sine tones. Release build, Ryzen 9 9950X, CPU time a tick:

| Case | Before | After |
|---|---|---|
| Hits, no pitch jitter | 0.40 ms | 0.16 ms |
| Hits, jitter, sinc | 5.00 ms | 3.55 ms |
| Hits, jitter, cubic (the `SoundSpec` default) | - | 0.30 ms |
| Hits, jitter, linear | - | 0.27 ms |
| Hits, jitter, cubic, spread through the tick with `trigger_in` | - | 0.23 ms |
| 20 sine tones (`tone`) | 0.22 ms | 0.10 ms |
| 20 sine tones from one `tone_sound` with jitter | - | 0.10 ms |

Channel drive (`bench_channel_drive`, 4 saw tones on one channel, 800-frame stereo ticks, 18 dB drive, mix 0.8): the drive stage adds 1.3 us a tick with `Soft`, 2.3 us with `Tanh` and 2.2 us with `Hard` (about 1 ns a sample, against 16 us for the 4 tones); off adds nothing.

Before, 92 % of the jittered case was the sinc: for every pitched frame `Kernel::taps` built 33 weights (a distance in f64, a table read and a lerp each, then a normalising divide over all of them) and the voice read 33 frames through bounds and loop checks. Now a voice with no ramp, curve, envelope or fade in flight renders its block in one tight loop with its gain, pan and step worked out once, and the cubic read is four loads and a Horner polynomial. The fades of stolen voices read the sine table instead of a libm cosine.

Adaptive music (`bench_adaptive_eight_stems`): eight stereo stems at 48 kHz, 800-frame ticks, release build, Ryzen 9 9950X, CPU time per second of audio: 0.50 ms with every stem playing and the intensity settled, 0.48 ms with the intensity ramping between 0 and 1 over 2 s (the silent stems are skipped), and 0.90 ms with a 0.5 s crossfade between sections running all the time (the old position is mixed as well). That is about 0.05 % of one core for the plain case.

## Tests

No audio device: tests drive the `Mixer` directly. They cover the volume product, mute, the ramp step size, the loop seam, crossfade power, the ducking envelope's timing and block-size independence, pitch and rate ramps without clicks, aliasing at 2x and 4x, the voice cap, stealing and seeded jitter (and a 10,000-hit burst), the soft limiter, the tone's frequency within 0.1 %, tone sounds sharing the cap, stealing order and jitter, a frame-offset start matching a block-edge start bit for bit, a golden hash of a seeded mix of sine tones and jittered cubic hits, PCM streams (pushed frames out unchanged after the ramp, the block size not mattering, underrun and resume without a step, finish and drop, the capacity; `crates/sound/tests/pcm.rs`), noise (seeded, bounded, white flat and pink -3 dB an octave), envelopes (exact linear stages, exponential landing, release from any level, no step at start or stop, delayed starts, `SoundSpec`; `crates/sound/tests/noise_envelope.rs`), the filters' -3 dB at their cutoff, drive off bit for bit, the three curves pinned by a hash, a compensated drive keeping a quiet tone's level, scheduled glides matching the plain call made at the same frame bit for bit (and leaving the frames before it alone), and a 1 s sine fixture (`crates/sound/tests/fixtures/sine.ogg`, made with the pinned ffmpeg's libvorbis; `sine.wav` is that ffmpeg's own decode of it) decoded within 0.001 of its twin. The voice effects' tests cover the crusher's bit-depth steps and sample-and-hold runs, the chorus's seed (the same seed the same samples, another seed others), the vocoder singing at the carrier's harmonics and falling silent with the voice, the segmentation finding synthesised bursts between silences (with and without onsets) and onsets splitting a run that steps up without a gap, and the grain helpers' lengths, seeded order and a repitch landing on the moved pitch. The media reader's tests (feature `media`) cut, resample and shape a WAV to its spec, decode an AAC tone in MP4 to its exact length and at its track's gain, and refuse Ogg, FLAC and DTS by name. The crate's own tests turn `vorbis`, `voicefx`, `shot`, `media`, `clean` and `music` on through a self dev-dependency, so the workspace gate runs them. The shot mix's tests (`crates/sound/tests/shot.rs`, feature `shot`) cover the mix's exact length for a frame count and rate, the same script giving the same samples (and another seed others), and the engine's cycle following its rpm curve (the autocorrelation's period within 2 % of 120 / rpm seconds at 2,000 and 5,000 rpm). The recorded engine's tests generate their recording in the test (a four-cylinder pulse train with uneven cylinders and a little seeded noise: an idle at 900 rpm, a rise, a rev held at 3,000 and a fall; no third-party sample) and cover its exact length, the same seed giving the same samples (and another seed others), its firing period following an rpm curve within 3 % at 900, 2,000, 3,000 and 5,500 rpm (past the recording) both alone, through the Stretch copies, and with the synthesised engine taking over, an unmarked recording finding its idle and rpm, grain seams no steeper (relative to the level) than 1.5 times the recording's own steepest step, and a looped `[[sample]]` whose crossfaded seam stays within 1.5 times a sine's own steepest step where a hard loop jumps, with its trims and fades holding. The road surfaces' tests cover asphalt mixing bit for bit as a road without `surface`, gravel's grains coming four times as often at four times the speed (a unit test in `src/shot/surface.rs`), the cobbles' thump period matching spacing / speed within 3 % at two spacings, and the same surfaces and seed giving the same samples (and another seed others). The tyre's tests cover the same seed and slip curve giving the same samples (another seed others, and `[[skid]]` the same as `[[tyre]]`), the squeal exactly silent at or below its threshold, sounding above it and silent again within one and a half seconds of grip returning, the chirp sounding just after each onset (twice for two slides) and nowhere else, not for a slide under way at the start, the scrub silent at no slip and rising at least 1.5 times from 1 to 4 to 12 m/s and with load, and out-of-bounds keys refused by name. The layer filters' tests cover a high-pass at 130 Hz on a road taking the band from 20 to 65 Hz down by more than 10 dB while leaving 600 to 1500 Hz within 1 dB or so, a low-pass at 1 kHz doing the same above 4 kHz, a keyed cutoff giving the same samples twice and others than a fixed one, and the bounds refused by name; the car-pass example, which `pfx sound mix` and `pfx shot --audio` must turn into the same bytes, carries a fixed and a keyed filter. The adaptive music's tests (`src/adaptive.rs`, `src/manifest.rs`) cover the stem curves (equal power across a crossing pair) and kira's easings, stems crossing by an eased intensity, two stems of opposite sign summing to exact silence through intensity ramps, beat jumps and crossfades in odd block sizes (sample-exact sync), a section change taken exactly at its beat, bar or end, sections flowing to their next and stopping, the seeded variant pick (the same seed the same picks, another seed others, `avoid_repeat` never twice), the block size not changing a sample, mismatched stems and sections refused, a golden hash of eight stems through a seeded intensity ramp with two section changes (bit-exact on every platform), and the `[music]` table loading from a `.pitofx` built in the test with its stems as chunks, with its errors naming the table and key. `crates/sound/tests/peak.rs` checks the true-peak meter against the true peaks of quarter-rate tones at 0, 45, 60 and 67.5 degrees, a +3 dBTP tone whose samples sit at full scale, tones at 1, 9.6 and 17 kHz and a click (within -0.4 and +0.4 dB, the EBU Tech 3341 window widened by the Annex 2 filter's 0.2 dB over-read near the top of the band) and against the ebur128 crate on the same signals, and that a generated mix at +2.6 dBTP masters to -1 dBTP with nothing over it while a quiet tone stays bit for bit.
