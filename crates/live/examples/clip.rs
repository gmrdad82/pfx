#[allow(dead_code)]
#[path = "../tests/support/desk.rs"]
mod desk;

use std::f32::consts::TAU;
use std::path::PathBuf;

use desk::{Desk, Pose, Show};
use pfx_live::demand::Need;

fn arg(index: usize, default: u32) -> u32 {
    std::env::args()
        .nth(index)
        .map(|value| value.parse().expect("frames, width and height are numbers"))
        .unwrap_or(default)
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: clip <dir> [frames] [width] [height]"),
    );
    let frames = arg(2, 180);
    let width = arg(3, 1280);
    let height = arg(4, 720);
    std::fs::create_dir_all(&out).unwrap();
    let mut desk = Desk::new(width, height);
    let show = Show {
        steam: true,
        overlay: true,
        ..Show::default()
    };
    for tick in 0..30 {
        desk.draw(Pose::default(), tick as f32 / 60.0, show, Some(Need::Full));
    }
    for frame in 0..frames {
        let phase = frame as f32 / frames as f32 * TAU;
        let pose = Pose {
            yaw: 20.0 * phase.sin(),
            pitch: 4.0 * (2.0 * phase).sin(),
            push: 0.04 * (1.0 - phase.cos()),
            zoom: 0.0,
        };
        desk.draw(pose, 0.5 + frame as f32 / 60.0, show, Some(Need::Full));
        let pixels = desk.pixels();
        desk::save_png(&out.join(format!("{frame:04}.png")), width, height, &pixels);
    }
    println!("{frames} frames of {width}x{height} in {}", out.display());
}
