use std::sync::Arc;

use super::*;

fn sheet() -> Arc<SpriteSheet> {
    let atlas = Atlas::grid([256, 64], [64, 32], None).unwrap();
    let run = SpriteClip::new("run", vec![0, 1, 2, 3], 10.0, Playback::Loop)
        .unwrap()
        .with_events(vec![
            FrameEvent::new(2, "foot"),
            FrameEvent::new(0, "begin"),
        ])
        .unwrap();
    let hit = SpriteClip::new("hit", vec![4, 5, 6], 20.0, Playback::Once)
        .unwrap()
        .with_events(vec![FrameEvent::new(2, "done")])
        .unwrap();
    Arc::new(SpriteSheet::new(atlas, vec![run, hit]).unwrap())
}

#[test]
fn a_grid_lays_frames_out_row_by_row() {
    let atlas = Atlas::grid([256, 64], [64, 32], Some(6)).unwrap();
    assert_eq!(atlas.len(), 6);
    assert_eq!(atlas.frames()[4], [0, 32, 64, 32]);
    assert_eq!(atlas.frames()[5], [64, 32, 64, 32]);
    assert_eq!(atlas.uv(5), Some([0.25, 0.5, 0.5, 1.0]));
    assert_eq!(atlas.uv(6), None);
    assert!(Atlas::grid([256, 64], [64, 32], Some(9)).is_err());
    assert!(Atlas::grid([16, 16], [32, 8], None).is_err());
}

#[test]
fn rects_must_sit_inside_the_atlas() {
    let atlas = Atlas::rects([100, 50], vec![[0, 0, 10, 50], [90, 10, 10, 10]]).unwrap();
    assert_eq!(atlas.uv(1), Some([0.9, 0.2, 1.0, 0.4]));
    let outside = Atlas::rects([100, 50], vec![[95, 0, 10, 10]]);
    assert!(outside.unwrap_err().to_string().contains("frame 0"));
    assert!(Atlas::rects([100, 50], vec![[0, 0, 0, 10]]).is_err());
}

#[test]
fn a_clip_steps_at_its_fps_on_the_tick_clock() {
    let mut player = SpritePlayer::new(sheet(), 60);
    assert_eq!(player.frame(0), None);
    player.play("run", 12).unwrap();
    let frames: Vec<usize> = (12..=72)
        .step_by(6)
        .map(|tick| player.frame(tick).unwrap())
        .collect();
    assert_eq!(frames, [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2]);
    assert_eq!(player.frame(17), Some(0));
    assert_eq!(player.uv(18), Some([0.25, 0.0, 0.5, 0.5]));
    player.play("hit", 100).unwrap();
    assert_eq!(player.frame(100), Some(4));
    assert_eq!(player.frame(103), Some(5));
    assert_eq!(player.frame(200), Some(6));
    assert!(player.finished(106));
    assert!(!player.finished(105));
    assert!(player.play("fly", 0).is_err());
}

#[test]
fn frame_events_fire_on_entering_their_frame_and_once_clips_fire_once() {
    let mut player = SpritePlayer::new(sheet(), 60);
    player.play("run", 0).unwrap();
    let mut heard = Vec::new();
    for tick in 0..=60 {
        for fired in player.advance(tick) {
            heard.push((fired.tick, fired.event));
        }
    }
    let expected: Vec<(u64, String)> = [
        (0, "begin"),
        (12, "foot"),
        (24, "begin"),
        (36, "foot"),
        (48, "begin"),
        (60, "foot"),
    ]
    .into_iter()
    .map(|(tick, name)| (tick, name.to_string()))
    .collect();
    assert_eq!(heard, expected);
    player.play("hit", 100).unwrap();
    let count: usize = (100..300).map(|tick| player.advance(tick).len()).sum();
    assert_eq!(count, 1);
}

#[test]
fn sprite_speed_rebases_without_a_jump() {
    let mut player = SpritePlayer::new(sheet(), 60);
    player.play("run", 0).unwrap();
    assert_eq!(player.frame(15), Some(2));
    player.set_speed(0.5, 15).unwrap();
    assert_eq!(player.frame(15), Some(2));
    assert_eq!(player.frame(20), Some(2));
    assert_eq!(player.frame(21), Some(3));
}

#[test]
fn sheets_refuse_frames_past_the_atlas_and_events_past_the_clip() {
    let atlas = Atlas::grid([64, 64], [32, 32], None).unwrap();
    let wide = SpriteClip::new("wide", vec![0, 4], 8.0, Playback::Loop).unwrap();
    assert!(SpriteSheet::new(atlas, vec![wide]).is_err());
    let late = SpriteClip::new("late", vec![0, 1], 8.0, Playback::Loop)
        .unwrap()
        .with_events(vec![FrameEvent::new(2, "x")]);
    assert!(late.is_err());
    assert!(SpriteClip::new("still", vec![0], 0.0, Playback::Loop).is_err());
}

#[test]
fn a_ping_pong_sprite_turns_at_both_ends_and_a_start_skips_ahead() {
    let atlas = Atlas::grid([64, 16], [16, 16], None).unwrap();
    let swing = SpriteClip::new("swing", vec![0, 1, 2, 3], 10.0, Playback::PingPong)
        .unwrap()
        .with_events(vec![FrameEvent::new(3, "top")])
        .unwrap();
    let sheet = Arc::new(SpriteSheet::new(atlas, vec![swing]).unwrap());
    let mut player = SpritePlayer::new(sheet, 10);
    player.play("swing", 0).unwrap();
    let frames: Vec<usize> = (0..10).map(|tick| player.frame(tick).unwrap()).collect();
    assert_eq!(frames, [0, 1, 2, 3, 2, 1, 0, 1, 2, 3]);
    let tops: Vec<u64> = (0..20)
        .flat_map(|tick| player.advance(tick))
        .map(|fired| fired.tick)
        .collect();
    assert_eq!(tops, [3, 9, 15]);
    assert!(!player.finished(100));
    player.play_from("swing", 0.25, 0).unwrap();
    assert_eq!(player.frame(0), Some(2));
    assert_eq!(player.advance(0).len(), 0);
    assert_eq!(player.advance(1).len(), 1);
}
