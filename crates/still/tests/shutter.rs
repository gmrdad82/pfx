use pfx_build::Registry;
use pfx_still::{Reel, Seed, Step, Take, looked_with};
use std::path::PathBuf;

const RECIPE: &str = r#"recipe = 1
name = "shutter"

[timeline]
fps = 60
frames = 3

[motion_blur]
shutter = 0.5
position = "POSITION"

[sun]
model = "authored"
toward = [0.3, 0.6, 1.0]
radius_deg = 0.5
color = [1.0, 1.0, 1.0]
irradiance = 4.0

[[camera]]
name = "main"
position = [0.75, 0.5, 4.0]
target = [0.75, 0.5, 0.0]
fov_y_deg = 45.0

[materials.white]
base = [0.9, 0.9, 0.9]
roughness = 0.8

[[object]]
name = "block"
kind = "box"
size = [0.5, 0.5, 0.5]
material = "white"
at = [0.0, 0.5, 0.0]
keys = { at = [[0.0, [0.0, 0.5, 0.0]], [0.05, [1.5, 0.5, 0.0]]] }
"#;

const SIZE: [u32; 2] = [240, 120];
const BAND: std::ops::Range<u32> = 50..70;

fn recipe(position: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/still-tests")
        .join(format!("shutter-{position}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shutter.toml");
    std::fs::write(&path, RECIPE.replace("POSITION", position)).unwrap();
    path
}

fn shoot(reel: &mut Reel, moving: bool, time: f64) -> Vec<u8> {
    let camera = match moving {
        true => reel.pose_moving(time, "main").unwrap(),
        false => {
            reel.pose(time).unwrap();
            reel.recipe.cameras[0].clone()
        }
    };
    let take = Take {
        size: SIZE,
        samples: 8,
        noise: None,
        seed: Seed::Locked,
    };
    let rendered = reel.frame(&camera, take, &mut |_: Step| {}).unwrap();
    looked_with(&rendered.film, (&rendered.finish, &rendered.overlay), false).unwrap()
}

fn luma(image: &[u8], x: u32, y: u32) -> f32 {
    let at = ((y * SIZE[0] + x) * 4) as usize;
    image[at..at + 3].iter().map(|&v| f32::from(v)).sum::<f32>() / 3.0
}

fn sides(sharp: &[u8], blurred: &[u8]) -> (f32, f32) {
    let background = BAND.clone().map(|y| luma(sharp, 0, y)).sum::<f32>() / BAND.len() as f32;
    let block: Vec<u32> = (0..SIZE[0])
        .filter(|&x| {
            BAND.clone()
                .any(|y| (luma(sharp, x, y) - background).abs() > 20.0)
        })
        .collect();
    let (left, right) = (block[0], *block.last().unwrap());
    let change = |x: u32| {
        BAND.clone()
            .map(|y| (luma(blurred, x, y) - luma(sharp, x, y)).abs())
            .sum::<f32>()
    };
    let behind = (0..left.saturating_sub(2)).map(change).sum();
    let ahead = (right + 3..SIZE[0]).map(change).sum();
    (behind, ahead)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_timeline_traces_every_frame_with_its_shutter_held_to_the_timeline_and_placed_by_position() {
    let mut measured = Vec::new();
    for position in ["start", "centre", "end"] {
        let mut reel = Reel::open(&recipe(position), None, &[], None, &Registry::new()).unwrap();
        assert!(reel.blurs());
        let mut frames = Vec::new();
        for frame in 0..=3u32 {
            let time = f64::from(frame) / 60.0;
            let blurred = shoot(&mut reel, true, time);
            let sharp = shoot(&mut reel, false, time);
            frames.push((sides(&sharp, &blurred), blurred == sharp));
        }
        println!("{position}: (behind, ahead) and unchanged, frames 0 to 3: {frames:?}");
        measured.push(frames);
    }
    let [start, centre, end] = [&measured[0], &measured[1], &measured[2]];
    let streaks = |side: f32| side > 200.0;
    let clean = |side: f32| side < 50.0;
    assert!(
        end[0].1,
        "a shutter closing at frame 0 holds the first pose: {end:?}"
    );
    assert!(
        start[3].1,
        "a shutter opening at the last frame holds the last pose: {start:?}"
    );
    let ((behind, ahead), _) = centre[0];
    assert!(
        clean(behind) && streaks(ahead),
        "frame 0 centred: {centre:?}"
    );
    let ((behind, ahead), _) = centre[3];
    assert!(
        streaks(behind) && clean(ahead),
        "the last frame centred: {centre:?}"
    );
    for frame in 1..=2 {
        let ((behind, ahead), _) = start[frame];
        assert!(
            clean(behind) && streaks(ahead),
            "start, frame {frame}: {start:?}"
        );
        let ((behind, ahead), _) = centre[frame];
        assert!(
            streaks(behind) && streaks(ahead),
            "centre, frame {frame}: {centre:?}"
        );
        let ((behind, ahead), _) = end[frame];
        assert!(
            streaks(behind) && clean(ahead),
            "end, frame {frame}: {end:?}"
        );
    }
}

fn over(reel: &mut Reel, time: f64, frame_seconds: f64) -> Vec<u8> {
    let camera = reel.pose_moving_over(time, "main", frame_seconds).unwrap();
    let take = Take {
        size: SIZE,
        samples: 8,
        noise: None,
        seed: Seed::Locked,
    };
    let rendered = reel.frame(&camera, take, &mut |_: Step| {}).unwrap();
    looked_with(&rendered.film, (&rendered.finish, &rendered.overlay), false).unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_time_maps_slope_scales_the_shutter_a_freeze_is_sharp_and_reverse_streaks_back() {
    let mut reel = Reel::open(&recipe("start"), None, &[], None, &Registry::new()).unwrap();
    let (time, step) = (1.0 / 60.0, 1.0 / 60.0);
    let sharp = shoot(&mut reel, false, time);
    let whole = shoot(&mut reel, true, time);
    assert!(
        over(&mut reel, time, step) == whole,
        "a slope of 1 is the timeline's own shutter"
    );
    let half = over(&mut reel, time, 0.5 * step);
    let frozen = over(&mut reel, time, 0.0);
    let back = over(&mut reel, time, -step);
    let measured = [&whole, &half, &back].map(|image| sides(&sharp, image));
    println!("(behind, ahead) at slopes 1, 0.5 and -1: {measured:?}");
    assert!(frozen == sharp, "a freeze has no blur");
    let [
        (_, whole_ahead),
        (half_behind, half_ahead),
        (back_behind, back_ahead),
    ] = measured;
    assert!(
        half_behind < 50.0 && half_ahead > 100.0 && half_ahead < 0.75 * whole_ahead,
        "half the slope streaks about half as far: {measured:?}"
    );
    assert!(
        back_behind > 200.0 && back_ahead < 50.0,
        "a reverse shutter runs back from the frame: {measured:?}"
    );
}
