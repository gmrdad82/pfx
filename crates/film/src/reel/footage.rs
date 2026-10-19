use super::wav;
use crate::shoot::RATE;
use pfx_core::rate::Rate;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const EPSILON: f64 = 1e-6;
pub const MANIFEST: &str = "manifest.json";
const AUDIO: &str = "audio.wav";
const NESTED: &str = "frames";

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
    pub stamp: u64,
}

pub trait Source {
    fn picture(&mut self, at: f64) -> Result<Option<&Picture>, String>;
    fn audio(&mut self, from: u64, count: usize, out: &mut Vec<f32>) -> Result<(), String>;
}

pub fn sample(seconds: f64) -> u64 {
    (seconds.max(0.0) * f64::from(RATE)).round() as u64
}

#[derive(Default, Deserialize)]
struct Manifest {
    rate: Option<Rate>,
    fps: Option<Rate>,
    frames: Option<u64>,
    wav: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub dir: PathBuf,
    pub fps: Rate,
    pub count: u64,
    pub wav: Option<PathBuf>,
}

fn frame_path(dir: &Path, index: u64) -> PathBuf {
    dir.join(format!("{index:06}.png"))
}

fn counted(dir: &Path) -> u64 {
    let mut n = 0;
    while frame_path(dir, n).is_file() {
        n += 1;
    }
    n
}

impl Sequence {
    pub fn find(dir: &Path, film_fps: Rate) -> Result<Sequence, String> {
        let show = |e: &dyn std::fmt::Display| format!("{}: {e}", dir.display());
        if !dir.is_dir() {
            return Err(show(&"no such folder"));
        }
        let manifest = match std::fs::read(dir.join(MANIFEST)) {
            Ok(bytes) => serde_json::from_slice::<Manifest>(&bytes)
                .map_err(|e| format!("{}: {e}", dir.join(MANIFEST).display()))?,
            Err(_) => Manifest::default(),
        };
        let frames = if frame_path(dir, 0).is_file() {
            dir.to_path_buf()
        } else {
            dir.join(NESTED)
        };
        let count = match manifest.frames {
            Some(n) => n.min(counted(&frames)),
            None => counted(&frames),
        };
        if count == 0 {
            return Err(show(&"no 000000.png in the folder or its frames/"));
        }
        let fps = manifest.rate.or(manifest.fps).unwrap_or(film_fps);
        if !fps.within(0, 1000) {
            return Err(show(&format!("a frame rate of {fps}")));
        }
        let wav = manifest
            .wav
            .map(|w| dir.join(w))
            .filter(|w| w.is_file())
            .or_else(|| Some(dir.join(AUDIO)).filter(|w| w.is_file()));
        Ok(Sequence {
            dir: frames,
            fps,
            count,
            wav,
        })
    }

    pub fn index(&self, at: f64) -> u64 {
        ((self.fps.frames(at) + EPSILON).floor().max(0.0) as u64).min(self.count - 1)
    }
}

pub fn decode_png(path: &Path) -> Result<([u32; 2], Vec<u8>), String> {
    let show = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| show(&e))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| show(&e))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| show(&"too large"))?
    ];
    let info = reader.next_frame(&mut buf).map_err(|e| show(&e))?;
    let bytes = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => bytes
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(show(&"an indexed PNG the decoder did not expand")),
    };
    Ok(([info.width, info.height], rgba))
}

pub struct Folder {
    sequence: Sequence,
    index: Option<u64>,
    shown: Option<Picture>,
    stamp: u64,
    sound: Vec<f32>,
}

impl Folder {
    pub fn open(sequence: Sequence) -> Result<Folder, String> {
        let sound = match &sequence.wav {
            Some(path) => wav::read(path)?,
            None => Vec::new(),
        };
        Ok(Folder {
            sequence,
            index: None,
            shown: None,
            stamp: 0,
            sound,
        })
    }
}

impl Source for Folder {
    fn picture(&mut self, at: f64) -> Result<Option<&Picture>, String> {
        let index = self.sequence.index(at);
        if self.index != Some(index) {
            let (size, rgba) = decode_png(&frame_path(&self.sequence.dir, index))?;
            self.stamp += 1;
            self.shown = Some(Picture {
                size,
                rgba,
                stamp: self.stamp,
            });
            self.index = Some(index);
        }
        Ok(self.shown.as_ref())
    }

    fn audio(&mut self, from: u64, count: usize, out: &mut Vec<f32>) -> Result<(), String> {
        let start = (from as usize).saturating_mul(2);
        for i in 0..count * 2 {
            out.push(self.sound.get(start + i).copied().unwrap_or(0.0));
        }
        Ok(())
    }
}

pub struct Card;

impl Source for Card {
    fn picture(&mut self, _at: f64) -> Result<Option<&Picture>, String> {
        Ok(None)
    }

    fn audio(&mut self, _from: u64, count: usize, out: &mut Vec<f32>) -> Result<(), String> {
        out.extend(std::iter::repeat_n(0.0, count * 2));
        Ok(())
    }
}
