use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub const FRAMES: &str = "frames";
pub const STILLS: &str = "stills";
pub const AUDIO: &str = "audio.wav";

pub trait Encoder {
    fn frame(&mut self, index: u64, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String>;
    fn still(&mut self, name: &str, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String>;
    fn finish(&mut self, audio: Option<&[f32]>, rate: u32, channels: u16) -> Result<(), String>;
}

pub struct Shot {
    pub path: PathBuf,
    pub rgba: Vec<u8>,
    pub size: [u32; 2],
}

pub struct Press {
    send: Option<SyncSender<Shot>>,
    workers: Vec<JoinHandle<Result<(), String>>>,
}

fn rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

pub fn png(shot: &Shot) -> Result<(), String> {
    let show = |e: &dyn std::fmt::Display| format!("{}: {e}", shot.path.display());
    let [w, h] = shot.size;
    if shot.rgba.len() != w as usize * h as usize * 4 {
        return Err(show(&"the frame's bytes don't match its size"));
    }
    let file = File::create(&shot.path).map_err(|e| show(&e))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| show(&e))?;
    writer
        .write_image_data(&rgb(&shot.rgba))
        .map_err(|e| show(&e))?;
    writer.finish().map_err(|e| show(&e))
}

fn work(queue: Arc<Mutex<Receiver<Shot>>>) -> Result<(), String> {
    loop {
        let next = queue.lock().map_err(|_| "a writer failed")?.recv();
        match next {
            Ok(shot) => png(&shot)?,
            Err(_) => return Ok(()),
        }
    }
}

impl Press {
    pub fn open() -> Press {
        let count = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .clamp(1, 16);
        let (send, receive) = sync_channel(count * 2);
        let queue = Arc::new(Mutex::new(receive));
        let workers = (0..count)
            .map(|_| {
                let queue = queue.clone();
                std::thread::spawn(move || work(queue))
            })
            .collect();
        Press {
            send: Some(send),
            workers,
        }
    }

    pub fn put(&self, shot: Shot) -> Result<(), String> {
        let send = self.send.as_ref().ok_or("the writers are closed")?;
        send.send(shot)
            .map_err(|_| "a frame writer stopped".to_string())
    }

    pub fn close(&mut self) -> Result<(), String> {
        self.send = None;
        let mut first = Ok(());
        for w in self.workers.drain(..) {
            let done = w
                .join()
                .unwrap_or_else(|_| Err("a frame writer panicked".into()));
            if first.is_ok() {
                first = done;
            }
        }
        first
    }
}

pub fn wav(path: &Path, samples: &[f32], rate: u32, channels: u16) -> Result<(), String> {
    let show = |e: std::io::Error| format!("{}: {e}", path.display());
    let bytes = u32::try_from(samples.len() * 2).map_err(|_| "the audio is too long")?;
    let mut out = BufWriter::new(File::create(path).map_err(show)?);
    let block = channels * 2;
    let mut head = Vec::with_capacity(44);
    head.extend_from_slice(b"RIFF");
    head.extend_from_slice(&(36 + bytes).to_le_bytes());
    head.extend_from_slice(b"WAVEfmt ");
    head.extend_from_slice(&16u32.to_le_bytes());
    head.extend_from_slice(&1u16.to_le_bytes());
    head.extend_from_slice(&channels.to_le_bytes());
    head.extend_from_slice(&rate.to_le_bytes());
    head.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    head.extend_from_slice(&block.to_le_bytes());
    head.extend_from_slice(&16u16.to_le_bytes());
    head.extend_from_slice(b"data");
    head.extend_from_slice(&bytes.to_le_bytes());
    out.write_all(&head).map_err(show)?;
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.write_all(&v.to_le_bytes()).map_err(show)?;
    }
    out.flush().map_err(show)
}

fn fresh(dir: &Path, make: bool) -> Result<(), String> {
    let show = |e: std::io::Error| format!("{}: {e}", dir.display());
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(show(e)),
    }
    if make {
        std::fs::create_dir_all(dir).map_err(show)?;
    }
    Ok(())
}

pub struct Files {
    out: PathBuf,
    press: Press,
}

impl Files {
    pub fn open(out: &Path, frames: bool, stills: bool) -> Result<Files, String> {
        fresh(&out.join(FRAMES), frames)?;
        fresh(&out.join(STILLS), stills)?;
        let audio = out.join(AUDIO);
        if audio.exists() {
            std::fs::remove_file(&audio).map_err(|e| format!("{}: {e}", audio.display()))?;
        }
        Ok(Files {
            out: out.to_path_buf(),
            press: Press::open(),
        })
    }
}

impl Encoder for Files {
    fn frame(&mut self, index: u64, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        self.press.put(Shot {
            path: self.out.join(FRAMES).join(format!("{index:06}.png")),
            rgba,
            size,
        })
    }

    fn still(&mut self, name: &str, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        self.press.put(Shot {
            path: self.out.join(STILLS).join(format!("{name}.png")),
            rgba,
            size,
        })
    }

    fn finish(&mut self, audio: Option<&[f32]>, rate: u32, channels: u16) -> Result<(), String> {
        self.press.close()?;
        match audio {
            Some(samples) => wav(&self.out.join(AUDIO), samples, rate, channels),
            None => Ok(()),
        }
    }
}
