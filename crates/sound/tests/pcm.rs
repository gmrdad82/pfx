use pfx_sound::{ChannelId, Level, Mixer, PCM_EDGE_SECONDS};

const RATE: u32 = 48_000;
const EDGE: usize = 96;

fn sine(frames: usize, from: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    (from..from + frames)
        .map(|i| (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin() * amplitude)
        .collect()
}

fn largest_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn the_edge_is_two_milliseconds() {
    assert_eq!((PCM_EDGE_SECONDS * RATE as f32).round() as usize, EDGE);
}

#[test]
fn pushed_frames_play_in_the_next_block_unchanged_after_the_start_ramp() {
    let mut mixer = Mixer::new(RATE, 1);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
    assert_eq!(mixer.render(64), vec![0.0; 64]);
    let input = sine(4800, 0, 440.0, 0.5);
    assert_eq!(sender.push(&input), 4800);
    let out = mixer.render(4800);
    assert!(out[1] != 0.0 || out[2] != 0.0);
    for i in EDGE..4800 {
        assert_eq!(out[i], input[i], "frame {i}");
    }
    assert!(
        out[..EDGE]
            .iter()
            .zip(&input)
            .all(|(a, b)| a.abs() <= b.abs() + 1e-6)
    );
    assert_eq!(sender.queued_frames(), 0);
}

#[test]
fn the_output_does_not_depend_on_the_block_size() {
    let input = sine(6000, 0, 300.0, 0.4);
    let render = |block: usize| {
        let mut mixer = Mixer::new(RATE, 1);
        let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
        sender.push(&input[..3000]);
        let mut out = Vec::new();
        let mut pushed = false;
        while out.len() < 8000 {
            out.extend(mixer.render(block));
            if !pushed && out.len() >= 3500 {
                sender.push(&input[3000..]);
                pushed = true;
            }
        }
        out.truncate(3500);
        out
    };
    let a = render(1);
    let b = render(64);
    let c = render(500);
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn an_underrun_fades_to_silence_without_a_click_and_resumes_without_one() {
    let mut mixer = Mixer::new(RATE, 1);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
    sender.push(&sine(3000, 0, 1000.0, 0.8));
    let first = mixer.render(3000 + 1000);
    let bound = std::f32::consts::TAU * 1000.0 / RATE as f32 * 0.8 * 1.05;
    assert!(
        largest_step(&first) <= bound.max(0.8 / EDGE as f32 + bound),
        "{}",
        largest_step(&first)
    );
    assert!(first[3000 + EDGE..].iter().all(|sample| *sample == 0.0));
    assert!(first[2999].abs() > 0.0 || first[2998].abs() > 0.0);
    let silent = mixer.render(500);
    assert!(silent.iter().all(|sample| *sample == 0.0));
    assert_eq!(mixer.voice_count(), 1);

    sender.push(&sine(3000, 3000, 1000.0, 0.8));
    let resumed = mixer.render(3000);
    assert!(
        largest_step(&resumed) <= bound + 0.8 / EDGE as f32 * 2.0,
        "{}",
        largest_step(&resumed)
    );
    assert!(resumed[0].abs() < 0.05);
    assert!(resumed[EDGE..].iter().any(|sample| sample.abs() > 0.7));
}

#[test]
fn a_short_gap_between_blocks_is_bridged_without_a_step() {
    let mut mixer = Mixer::new(RATE, 1);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
    sender.push(&sine(1000, 0, 200.0, 0.6));
    let mut out = mixer.render(1000);
    out.extend(mixer.render(10));
    sender.push(&sine(1000, 1000, 200.0, 0.6));
    out.extend(mixer.render(1000));
    let bound = std::f32::consts::TAU * 200.0 / RATE as f32 * 0.6 * 1.05;
    assert!(
        largest_step(&out) <= bound + 0.6 / EDGE as f32,
        "{}",
        largest_step(&out)
    );
}

#[test]
fn a_finished_stream_drains_fades_out_and_ends_its_voice() {
    let mut mixer = Mixer::new(RATE, 1);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
    sender.push(&sine(2000, 0, 500.0, 0.5));
    sender.finish();
    let out = mixer.render(2000 + 2 * EDGE);
    assert_eq!(mixer.voice_count(), 0);
    assert_eq!(*out.last().unwrap(), 0.0);
    assert!(largest_step(&out) < 0.05);
    assert!(out[1500..2000].iter().any(|sample| sample.abs() > 0.3));
}

#[test]
fn a_dropped_sender_ends_the_stream_like_a_finish() {
    let mut mixer = Mixer::new(RATE, 1);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
    sender.push(&sine(500, 0, 500.0, 0.5));
    drop(sender);
    mixer.render(500 + 2 * EDGE);
    assert_eq!(mixer.voice_count(), 0);
}

#[test]
fn a_stopped_voice_disconnects_its_sender() {
    let mut mixer = Mixer::new(RATE, 1);
    let (id, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
    assert!(sender.is_connected());
    mixer.stop(id, 0.0);
    assert!(!sender.is_connected());
}

#[test]
fn the_ring_holds_its_capacity_and_reports_what_it_took() {
    let mut mixer = Mixer::new(RATE, 2);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 2, 0.01, Level::default());
    assert_eq!(sender.capacity_frames(), 480);
    assert_eq!(sender.channels(), 2);
    assert_eq!(sender.rate(), RATE);
    assert_eq!(sender.push(&vec![0.1; 2 * 400]), 400);
    assert_eq!(sender.room_frames(), 80);
    assert_eq!(sender.push(&vec![0.1; 2 * 400]), 80);
    assert_eq!(sender.push(&[0.1]), 0);
    assert_eq!(sender.queued_frames(), 480);
    mixer.render(100);
    assert_eq!(sender.queued_frames(), 380);
}

#[test]
fn stereo_frames_keep_their_sides_and_extra_channels_are_skipped() {
    let mut mixer = Mixer::new(RATE, 2);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 2, 1.0, Level::default());
    let mut input = Vec::new();
    for _ in 0..500 {
        input.extend([0.5, -0.25]);
    }
    sender.push(&input);
    let out = mixer.render(500);
    assert_eq!(out[2 * 300], 0.5);
    assert_eq!(out[2 * 300 + 1], -0.25);

    let mut mixer = Mixer::new(RATE, 2);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 3, 1.0, Level::default());
    let mut input = Vec::new();
    for _ in 0..500 {
        input.extend([0.5, -0.25, 0.9]);
    }
    sender.push(&input);
    let out = mixer.render(500);
    assert_eq!(out[2 * 300], 0.5);
    assert_eq!(out[2 * 300 + 1], -0.25);

    let mut mixer = Mixer::new(RATE, 2);
    let (_, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
    sender.push(&vec![0.5; 500]);
    let out = mixer.render(500);
    assert_eq!(out[2 * 300], 0.5);
    assert_eq!(out[2 * 300 + 1], 0.5);
}

#[test]
fn the_voices_level_and_channel_apply_to_a_stream() {
    let mut mixer = Mixer::new(RATE, 1);
    let level = Level {
        volume: 0.5,
        ..Level::default()
    };
    let (_, sender) = mixer.pcm_stream(ChannelId::DIALOG, 1, 1.0, level);
    mixer.set_channel_volume(ChannelId::DIALOG, 0.5);
    sender.push(&vec![0.8; 3000]);
    let out = mixer.render(3000);
    assert!((out[2000] - 0.2).abs() < 1e-6, "{}", out[2000]);
}

#[test]
fn a_stop_with_a_fade_releases_a_stream_that_is_still_open() {
    let mut mixer = Mixer::new(RATE, 1);
    let (id, sender) = mixer.pcm_stream(ChannelId::EFFECTS, 1, 1.0, Level::default());
    sender.push(&vec![0.5; 20_000]);
    mixer.render(1000);
    mixer.stop(id, 0.02);
    let out = mixer.render(1200);
    assert_eq!(*out.last().unwrap(), 0.0);
    assert_eq!(mixer.voice_count(), 0);
    assert!(sender.queued_frames() > 0 || !sender.is_connected());
}
