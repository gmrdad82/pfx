#![cfg(feature = "vorbis")]

use pfx_sound::{Clip, Level, Mixer, MusicTrack, VorbisSource};

const OGG: &[u8] = include_bytes!("fixtures/sine.ogg");
const WAV: &[u8] = include_bytes!("fixtures/sine.wav");

#[test]
fn a_vorbis_file_decodes_to_its_wav_twin() {
    let decoded = Clip::from_vorbis(OGG).expect("decodes");
    let twin = Clip::from_wav(WAV).expect("twin parses");
    assert_eq!(decoded.rate, 44_100);
    assert_eq!(decoded.channels, 2);
    assert_eq!(decoded.rate, twin.rate);
    assert_eq!(decoded.channels, twin.channels);
    assert_eq!(decoded.frames(), 44_100);
    assert!(twin.frames() <= decoded.frames() && decoded.frames() - twin.frames() <= 256);
    let worst = decoded
        .samples
        .iter()
        .zip(&twin.samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(worst < 0.001, "largest error {worst}");
    let peak = decoded.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((peak - 0.5).abs() < 0.02, "peak {peak}");
}

#[test]
fn garbage_is_not_vorbis() {
    assert!(Clip::from_vorbis(b"not an ogg file").is_err());
    assert!(Clip::from_vorbis(&[]).is_err());
    assert!(Clip::from_vorbis(&OGG[..40]).is_err());
}

#[test]
fn a_decoded_clip_plays_as_music_and_loops() {
    let clip = Clip::from_vorbis(OGG).unwrap();
    let frames = clip.frames();
    let mut mixer = Mixer::new(44_100, 2);
    let track = MusicTrack::with_loop(clip.clone(), 11_025, 33_075);
    mixer.play_music(&track, 0.0, Level::default());
    let out = mixer.render(frames as usize * 2);
    assert_eq!(out[..2 * 33_075], clip.samples[..2 * 33_075]);
    assert_eq!(
        out[2 * 33_075..2 * 33_077],
        clip.samples[2 * 11_025..2 * 11_027]
    );
    assert!(mixer.music().is_some());
}

#[test]
fn a_missing_or_broken_stream_source_is_an_error() {
    let mut mixer = Mixer::new(44_100, 2);
    let broken = VorbisSource::Bytes(b"nope".to_vec().into());
    assert!(
        mixer
            .play_music_vorbis(&broken, None, 0.0, Level::default())
            .is_err()
    );
    let missing = VorbisSource::File("does-not-exist.ogg".into());
    assert!(
        mixer
            .stream_vorbis(
                pfx_sound::ChannelId::DIALOG,
                &missing,
                None,
                Level::default()
            )
            .is_err()
    );
}
