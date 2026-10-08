# pfx-sound

Live mixing for games and apps, and deterministic mixdown for rendered clips. One `Mixer` does both: the audio device drives it through `Output`, a render drives it directly.

## Channels

Every voice plays on one channel. Channels sit under a master.

```
voice (pitch, gain, level) -> channel filters -> channel volume/mute -> ducking -> master volume/mute -> soft limiter (if set) -> clamp
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
- `Output` mirrors all of this (`play_on`, `set_channel_volume`, `set_master_muted`, `play_music`, ...), behind the same lock the audio thread takes.

## Music

```rust
let track = MusicTrack::with_loop(clip, loop_start, loop_end);
mixer.play_music(&track, 2.0, Level::default());
```

- `MusicTrack::once`, `looping` (the whole clip) and `with_loop` (frames in the clip's own rate; scaled when the clip is resampled to the output).
- The loop is a hard wrap from `loop_end` back to `loop_start`, sample exact. Author the points on a zero crossing or a bar line; the engine adds no smoothing.
- `play_music` while a track is playing crossfades: the old track fades out as the new fades in over the given seconds, **equal power** (cos and sin curves, so the squared gains always sum to 1; two unrelated tracks keep their summed power within 0.5 dB). Zero seconds switches at once.
- `stop_music(fade)` fades the current track out. `music()` is the playing track's voice, if any.

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
- Any other ratio resamples with a Kaiser-windowed sinc, 16 zero crossings each side, its cutoff at 0.9 of Nyquist. Above 1 the kernel widens by the ratio so the source's top octaves are filtered out before they can fold back, which is the aliasing of cheaper interpolators. Measured at 48 kHz, a tone that would fold back sits at **-80 dB** or lower below the source at 2x (17 kHz source: -80 dB, 21 kHz: -98 dB) and **-91 dB** or lower at 4x (11 kHz: -91 dB, 21 kHz: -106 dB); a tone in the passband comes out within 0.001 dB of its level at both. The cost grows with the ratio (32 taps a channel at 1x, 128 at 4x).

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
- At `max_voices` live voices of the sound, `trigger` steals one first: `Steal::Oldest` takes the earliest started, `Steal::Quietest` the one with the lowest level times its fade times how much of the clip is left. A stolen voice that has already played fades out over `steal_fade` (3 ms by default); one that never reached the mixer is dropped. `max_voices: 0` plays nothing; the default is no cap.
- Each trigger draws a pitch of up to `pitch_jitter` semitones either way and a volume of up to `volume_jitter` (a fraction) either way, from the mixer's own seeded generator. `set_seed` starts it; the same seed and the same triggers give the same samples, bit for bit. The mixer never reads the clock or the system's randomness.
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

- `Wave::Sine`, `Square` and `Saw`. Square and saw are band-limited with PolyBLEP, so their edges do not fold back at high notes.
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
- `PcmSender::push` returns the frames it took, which is fewer than offered when the ring is full (the producer decides what to drop; nothing is overwritten) and ignores a trailing partial frame. `queued_frames`, `room_frames`, `capacity_frames`, `channels` and `rate` read the state. The sender is `Send`, so it can live on another thread.
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

## Vorbis (feature `vorbis`)

Games ship with no ffmpeg, so the feature decodes Ogg Vorbis in pure Rust with [lewton](https://crates.io/crates/lewton) 0.10.2 (pinned). lewton over symphonia: it is only an Ogg Vorbis decoder, about four small crates against symphonia's container and codec framework, and it is all the games need. It is off by default; the ffmpeg path (`Decode`, `DecodeSpec`) is untouched and stays for the renderers.

```rust
let clip = Clip::from_vorbis(&bytes)?;

let source = VorbisSource::File(path);
mixer.play_music_vorbis(&source, Some((loop_start, loop_end)), 2.0, Level::default())?;
mixer.stream_vorbis(ChannelId::DIALOG, &source, None, Level::default())?;
```

- `Clip::from_vorbis` decodes whole, trimmed to the stream's final granule position (lewton alone leaves the last page's tail on).
- The stream decodes on its own thread, two seconds ahead of the mixer, and resamples to the mixer's rate with the same windowed-sinc as pitch (below). Looping streams (frames of the file's own rate) re-open the source and skip to the loop start, so the wrap is sample exact; the thread exits when its voice is dropped.
- `VorbisSource` is bytes held in memory or a file path.
- Stereo and mono play as they are; a file with more channels plays its first two.

## Tests

No audio device: tests drive the `Mixer` directly. They cover the volume product, mute, the ramp step size, the loop seam, crossfade power, the ducking envelope's timing and block-size independence, pitch and rate ramps without clicks, aliasing at 2x and 4x, the voice cap, stealing and seeded jitter (and a 10,000-hit burst), the soft limiter, the tone's frequency within 0.1 %, PCM streams (pushed frames out unchanged after the ramp, the block size not mattering, underrun and resume without a step, finish and drop, the capacity; `crates/sound/tests/pcm.rs`), noise (seeded, bounded, white flat and pink -3 dB an octave), envelopes (exact linear stages, exponential landing, release from any level, no step at start or stop, delayed starts, `SoundSpec`; `crates/sound/tests/noise_envelope.rs`), the filters' -3 dB at their cutoff, and a 1 s sine fixture (`crates/sound/tests/fixtures/sine.ogg`, made with the pinned ffmpeg's libvorbis; `sine.wav` is that ffmpeg's own decode of it) decoded within 0.001 of its twin. The crate's own tests turn `vorbis` on through a self dev-dependency, so the workspace gate runs them.
