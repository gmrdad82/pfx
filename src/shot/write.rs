use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use exr::prelude::{
    Image as ExrImage, SpecificChannels, Vec2, WritableImage, attribute::Chromaticities, f16,
};

use super::args::Format;

pub const RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;

pub fn frame_name(index: u64, format: Format) -> String {
    format!("{index:06}.{}", format.name())
}

pub fn is_frame_name(name: &str) -> bool {
    let Some((stem, extension)) = name.split_once('.') else {
        return false;
    };
    stem.len() == 6
        && stem.bytes().all(|byte| byte.is_ascii_digit())
        && matches!(extension, "png" | "exr")
}

fn decoded(bits: u16) -> f32 {
    f16::from_bits(bits).to_f32()
}

fn display_table() -> Vec<u8> {
    (0..=u16::MAX)
        .map(|bits| {
            let value = decoded(bits);
            let value = if value.is_nan() {
                0.0
            } else {
                value.clamp(0.0, 1.0)
            };
            (value * 255.0).round() as u8
        })
        .collect()
}

fn linear(encoded: f32) -> f32 {
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

pub fn png(
    path: &Path,
    width: u32,
    height: u32,
    pixels: &[u16],
    table: &[u8],
) -> Result<(), String> {
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in pixels.chunks_exact(4) {
        rgb.push(table[pixel[0] as usize]);
        rgb.push(table[pixel[1] as usize]);
        rgb.push(table[pixel[2] as usize]);
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    writer
        .write_image_data(&rgb)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    writer
        .finish()
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn exr(path: &Path, width: u32, height: u32, pixels: &[u16]) -> Result<(), String> {
    let halves: Vec<f16> = pixels
        .chunks_exact(4)
        .flat_map(|pixel| {
            [
                f16::from_f32(linear(decoded(pixel[0]).max(0.0))),
                f16::from_f32(linear(decoded(pixel[1]).max(0.0))),
                f16::from_f32(linear(decoded(pixel[2]).max(0.0))),
                f16::from_f32(decoded(pixel[3]).clamp(0.0, 1.0)),
            ]
        })
        .collect();
    let channels = SpecificChannels::rgba(|Vec2(x, y)| {
        let index = (y * width as usize + x) * 4;
        (
            halves[index],
            halves[index + 1],
            halves[index + 2],
            halves[index + 3],
        )
    });
    let mut image = ExrImage::from_channels((width as usize, height as usize), channels);
    image.attributes.chromaticities = Some(Chromaticities {
        red: Vec2(0.64, 0.33),
        green: Vec2(0.30, 0.60),
        blue: Vec2(0.15, 0.06),
        white: Vec2(0.3127, 0.3290),
    });
    image
        .write()
        .non_parallel()
        .to_file(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn audio_frames(frames: u64, fps: u32) -> usize {
    ((frames as f64 * f64::from(RATE) / f64::from(fps.max(1))).round()) as usize
}

pub fn wav(path: &Path, samples: Vec<f32>) -> Result<(), String> {
    let clip = pfx_sound::Clip {
        rate: RATE,
        channels: CHANNELS,
        samples,
    };
    std::fs::write(path, clip.to_wav()).map_err(|e| format!("{}: {e}", path.display()))
}

struct Job {
    path: PathBuf,
    width: u32,
    height: u32,
    pixels: Vec<u16>,
}

pub struct Writers {
    send: Option<SyncSender<Job>>,
    workers: Vec<JoinHandle<Result<(), String>>>,
    format: Format,
}

fn work(
    jobs: Arc<Mutex<Receiver<Job>>>,
    format: Format,
    table: Arc<Vec<u8>>,
) -> Result<(), String> {
    loop {
        let job = {
            let receiver = jobs
                .lock()
                .map_err(|_| "a frame writer stopped".to_string())?;
            receiver.recv()
        };
        let Ok(job) = job else {
            return Ok(());
        };
        match format {
            Format::Png => png(&job.path, job.width, job.height, &job.pixels, &table)?,
            Format::Exr => exr(&job.path, job.width, job.height, &job.pixels)?,
        }
    }
}

impl Writers {
    pub fn new(format: Format) -> Self {
        let count = std::thread::available_parallelism()
            .map_or(4, |count| count.get())
            .clamp(1, 8);
        let (send, receive) = sync_channel::<Job>(count * 2);
        let jobs = Arc::new(Mutex::new(receive));
        let table = Arc::new(display_table());
        let workers = (0..count)
            .map(|_| {
                let jobs = jobs.clone();
                let table = table.clone();
                std::thread::spawn(move || work(jobs, format, table))
            })
            .collect();
        Self {
            send: Some(send),
            workers,
            format,
        }
    }

    pub fn write(
        &self,
        dir: &Path,
        index: u64,
        width: u32,
        height: u32,
        pixels: Vec<u16>,
    ) -> Result<(), String> {
        let path = dir.join(frame_name(index, self.format));
        self.send
            .as_ref()
            .ok_or("the frame writers are closed")?
            .send(Job {
                path,
                width,
                height,
                pixels,
            })
            .map_err(|_| "a frame writer failed".to_string())
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.send = None;
        let mut first = Ok(());
        for worker in self.workers.drain(..) {
            let outcome = worker
                .join()
                .unwrap_or_else(|_| Err("a frame writer panicked".to_string()));
            if first.is_ok() {
                first = outcome;
            }
        }
        first
    }
}
