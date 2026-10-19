#![cfg(feature = "video")]

use pfx_core::rate::Rate;
use pfx_film::reel::Roll;
use pfx_film::{Encoder, RATE};
use pito_codecs::media::{
    self, AudioBuffer, Canvas, Choice, ExportOptions, Frame, PixelFormat, Prefs, TimeBase,
    VideoFrame, export_frames,
};
use std::path::{Path, PathBuf};

const SIZE: [u32; 2] = [320, 180];
const FPS: Rate = Rate::whole(30);
const SOURCE: [u32; 2] = [64, 48];
const COUNT: u32 = 30;
const BACK: [u8; 3] = [192, 128, 64];

fn dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn video(path: &Path) {
    let [w, h] = SOURCE;
    let canvas = Canvas {
        width: w,
        height: h,
        fps: (FPS.num(), FPS.den()),
        frames: Some(u64::from(COUNT)),
    };
    let opts = ExportOptions {
        prefs: Prefs {
            decode: Choice::Cpu,
            encode: Choice::Cpu,
            ..Prefs::default()
        },
        ..ExportOptions::default()
    };
    export_frames(
        path,
        canvas,
        opts,
        |n, _| {
            if n >= u64::from(COUNT) {
                return Ok(None);
            }
            let mut frame = Frame::new(
                PixelFormat::Yuv420p8,
                w,
                h,
                n as i64,
                TimeBase::new(FPS.den(), FPS.num()),
            );
            let luma = 30 + 6 * n as u8;
            for (plane, value) in frame.planes.iter_mut().zip([luma, 128, 128]) {
                plane.data.fill(value);
            }
            Ok(Some(VideoFrame::Cpu(frame)))
        },
        |_, samples| {
            Ok(AudioBuffer {
                samples: vec![0.0; samples * 2],
                rate: RATE,
                channels: 2,
            })
        },
        |_| true,
    )
    .unwrap();
}

fn reference(path: &Path) -> Vec<[u8; 3]> {
    let prefs = Prefs {
        decode: Choice::Cpu,
        ..Prefs::default()
    };
    let decoded = media::decode_video(path, 0.0, None, &prefs).unwrap();
    decoded
        .map(|frame| {
            let rgba = media::to_rgba(&frame.unwrap());
            let at = ((rgba.height / 2 * rgba.width + rgba.width / 2) * 4) as usize;
            [rgba.data[at], rgba.data[at + 1], rgba.data[at + 2]]
        })
        .collect()
}

fn png(path: &Path, size: [u32; 2], colour: [u8; 3]) {
    let file = std::fs::File::create(path).unwrap();
    let mut png = png::Encoder::new(file, size[0], size[1]);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    let pixels: Vec<u8> = (0..size[0] * size[1]).flat_map(|_| colour).collect();
    png.write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

#[derive(Default)]
struct Tape {
    frames: Vec<Vec<u8>>,
}

impl Encoder for Tape {
    fn frame(&mut self, _index: u64, rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        self.frames.push(rgba);
        Ok(())
    }

    fn still(&mut self, _name: &str, _rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        Ok(())
    }

    fn finish(&mut self, _audio: Option<&[f32]>, _rate: u32, _channels: u16) -> Result<(), String> {
        Ok(())
    }
}

fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE[0] + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3], by: i32) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (i32::from(*x) - i32::from(y)).abs() <= by)
}

fn darker(shadowed: [u8; 3], alpha: f32) -> bool {
    shadowed.iter().zip(BACK).all(|(s, b)| {
        let want = f32::from(b) * (1.0 - alpha);
        (f32::from(*s) - want).abs() <= 2.0
    })
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_clip_art_plays_its_video_from_its_offset_with_round_corners_and_a_shadow() {
    let at = dir("clip-art");
    video(&at.join("inset.mp4"));
    png(&at.join("tile.png"), [32, 32], [20, 60, 220]);
    let script = at.join("inset.film.toml");
    std::fs::write(
        &script,
        r##"[film]
size = [320, 180]
layout = [320, 180]
fps = 30
background = "#c08040"

[[scene]]
name = "inset"
kind = "card"
seconds = 1.0

[[scene.art]]
kind = "clip"
file = "inset.mp4"
at = 0.1
fade = 0
out = 0
from = 0.2
place = [160, 90]
width = 160
height = 90
fit = "fill"
radius = 20
shadow = { offset = [12, 12], alpha = 0.5 }

[[scene]]
name = "tile"
kind = "card"
seconds = 0.2

[[scene.art]]
kind = "image"
file = "tile.png"
at = 0
fade = 0
out = 0
place = [160, 90]
width = 100
height = 100
radius = 30
shadow = { offset = [0, 40], blur = 4, alpha = 0.6 }
"##,
    )
    .unwrap();
    let roll = Roll::open(&script).unwrap();
    let mut tape = Tape::default();
    roll.roll(Some(&mut tape)).unwrap();
    assert_eq!(tape.frames.len(), 36);
    let source = reference(&at.join("inset.mp4"));
    assert_eq!(source.len(), COUNT as usize);
    assert!(near(pixel(&tape.frames[2], 160, 90), BACK, 2));
    for n in 0..20u32 {
        let shown = pixel(&tape.frames[(3 + n) as usize], 160, 90);
        let want = source[(6 + n) as usize];
        assert!(
            near(shown, want, 3),
            "frame {n}: {shown:?}, source {want:?}"
        );
        assert!(!near(shown, source[(5 + n) as usize], 1), "frame {n}");
    }
    let mid = &tape.frames[10];
    assert!(
        near(pixel(mid, 82, 47), BACK, 2),
        "{:?}",
        pixel(mid, 82, 47)
    );
    assert!(near(pixel(mid, 237, 47), BACK, 2));
    assert!(!near(pixel(mid, 100, 90), BACK, 8));
    assert!(
        darker(pixel(mid, 246, 100), 0.5),
        "{:?}",
        pixel(mid, 246, 100)
    );
    assert!(near(pixel(mid, 246, 50), BACK, 2));
    let tile = &tape.frames[32];
    assert!(
        near(pixel(tile, 160, 90), [20, 60, 220], 3),
        "{:?}",
        pixel(tile, 160, 90)
    );
    assert!(
        near(pixel(tile, 112, 42), BACK, 2),
        "{:?}",
        pixel(tile, 112, 42)
    );
    assert!(
        darker(pixel(tile, 160, 170), 0.6),
        "{:?}",
        pixel(tile, 160, 170)
    );
    std::fs::remove_dir_all(at).unwrap();
}
