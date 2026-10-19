use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use pfx_post::quantise::{self, Dither};

use exr::prelude::{
    Image as ExrImage, SpecificChannels, Vec2, WritableImage, attribute::Chromaticities, f16,
};

use super::args::Format;

pub const RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;

pub fn frame_name(index: u64, format: Format) -> String {
    format!("{index:06}.{}", format.extension())
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

fn display_table() -> Vec<f32> {
    (0..=u16::MAX).map(decoded).collect()
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
    table: &[f32],
    dither: Option<&quantise::Tile>,
) -> Result<(), String> {
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for (at, pixel) in pixels.chunks_exact(4).enumerate() {
        let [r, g, b] = [0, 1, 2].map(|k| table[pixel[k] as usize]);
        match dither {
            Some(tile) => {
                let at = at as u32;
                let offset = tile.at(at % width.max(1), at / width.max(1));
                rgb.extend([r, g, b].map(|v| quantise::dithered(v, offset)));
            }
            None => rgb.extend([r, g, b].map(quantise::code)),
        }
    }
    encode(path, width, height, png::BitDepth::Eight, &rgb)
}

pub fn png16(
    path: &Path,
    width: u32,
    height: u32,
    pixels: &[u16],
    table: &[f32],
) -> Result<(), String> {
    let rgb = quantise::sixteen(pixels.len() / 4, 3, |at| {
        let pixel = &pixels[at * 4..at * 4 + 4];
        [0, 1, 2, 3].map(|k| table[pixel[k] as usize])
    });
    encode(path, width, height, png::BitDepth::Sixteen, &rgb)
}

fn encode(
    path: &Path,
    width: u32,
    height: u32,
    depth: png::BitDepth,
    rgb: &[u8],
) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(depth);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    writer
        .write_image_data(rgb)
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
    table: Arc<Vec<f32>>,
    dither: Option<Arc<quantise::Tile>>,
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
            Format::Png => png(
                &job.path,
                job.width,
                job.height,
                &job.pixels,
                &table,
                dither.as_deref(),
            )?,
            Format::Png16 => png16(&job.path, job.width, job.height, &job.pixels, &table)?,
            Format::Exr => exr(&job.path, job.width, job.height, &job.pixels)?,
        }
    }
}

impl Writers {
    pub fn new(format: Format, dither: Option<Dither>) -> Self {
        let count = std::thread::available_parallelism()
            .map_or(4, |count| count.get())
            .clamp(1, 8);
        let (send, receive) = sync_channel::<Job>(count * 2);
        let jobs = Arc::new(Mutex::new(receive));
        let table = Arc::new(display_table());
        let dither = dither.map(|d| Arc::new(d.tile()));
        let workers = (0..count)
            .map(|_| {
                let jobs = jobs.clone();
                let table = table.clone();
                let dither = dither.clone();
                std::thread::spawn(move || work(jobs, format, table, dither))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(path: &Path) -> (png::BitDepth, Vec<u8>, bool) {
        let file = std::fs::File::open(path).unwrap();
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .unwrap();
        let srgb = reader.info().srgb.is_some();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        (info.bit_depth, bytes, srgb)
    }

    #[test]
    fn png16_keeps_the_float_frame_and_png_dithers_it_to_the_same_level() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/shot-write-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let one = f16::from_f32(1.0).to_bits();
        let half = f16::from_f32(0.5).to_bits();
        let pixels: Vec<u16> = (0..64).flat_map(|_| [half, half, one, one]).collect();
        let table = display_table();
        let wide = dir.join("wide.png");
        png16(&wide, 8, 8, &pixels, &table).unwrap();
        let (depth, bytes, srgb) = decoded(&wide);
        assert_eq!(depth, png::BitDepth::Sixteen);
        assert!(srgb);
        assert_eq!(&bytes[..6], &[0x80, 0x00, 0x80, 0x00, 0xff, 0xff]);
        let tile = Dither::seeded(0).tile();
        let eight = dir.join("eight.png");
        png(&eight, 8, 8, &pixels, &table, Some(&tile)).unwrap();
        let (depth, bytes, _) = decoded(&eight);
        assert_eq!(depth, png::BitDepth::Eight);
        assert!(
            bytes
                .chunks(3)
                .all(|p| (127..=129).contains(&p[0]) && p[2] == 255)
        );
        assert!(bytes.chunks(3).any(|p| p[0] != 128));
    }
}
