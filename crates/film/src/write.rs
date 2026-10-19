use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub const FRAMES: &str = "frames";
pub const STILLS: &str = "stills";
pub const AUDIO: &str = "audio.wav";
pub const CLAIM: &str = "film.txt";
const IEND: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];

pub trait Encoder {
    fn frame(&mut self, index: u64, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String>;
    fn still(&mut self, name: &str, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String>;
    fn finish(&mut self, audio: Option<&[f32]>, rate: u32, channels: u16) -> Result<(), String>;
    fn kept(&self, _index: u64) -> bool {
        false
    }
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
    pfx_core::atomic::write_with(&shot.path, |temp| {
        let file = File::create(temp).map_err(|e| show(&e))?;
        let mut encoder = png::Encoder::new(BufWriter::new(file), w, h);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| show(&e))?;
        writer
            .write_image_data(&rgb(&shot.rgba))
            .map_err(|e| show(&e))?;
        writer.finish().map_err(|e| show(&e))
    })
}

pub fn finished(path: &Path, size: [u32; 2]) -> bool {
    let read = || -> Option<[u32; 2]> {
        let mut file = File::open(path).ok()?;
        let mut end = [0u8; 12];
        file.seek(SeekFrom::End(-12)).ok()?;
        file.read_exact(&mut end).ok()?;
        if end != IEND {
            return None;
        }
        file.rewind().ok()?;
        let reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .ok()?;
        let info = reader.info();
        (info.bit_depth == png::BitDepth::Eight).then_some([info.width, info.height])
    };
    read() == Some(size)
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
    pfx_core::atomic::write_with(path, |temp| wav_to(temp, samples, rate, channels))
}

fn wav_to(path: &Path, samples: &[f32], rate: u32, channels: u16) -> Result<(), String> {
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
    kept: Vec<bool>,
}

#[derive(Clone, Copy, Debug)]
pub struct Resume<'a> {
    pub claim: &'a str,
    pub frames: u64,
    pub size: [u32; 2],
}

fn frame_path(out: &Path, index: u64) -> PathBuf {
    out.join(FRAMES).join(format!("{index:06}.png"))
}

impl Files {
    pub fn open(out: &Path, frames: bool, stills: bool) -> Result<Files, String> {
        fresh(&out.join(FRAMES), frames)?;
        Files::opened(out, stills, Vec::new())
    }

    pub fn resume(out: &Path, frames: bool, stills: bool, resume: Resume) -> Result<Files, String> {
        let claim = out.join(CLAIM);
        let same =
            frames && std::fs::read_to_string(&claim).is_ok_and(|found| found == resume.claim);
        let kept = if same {
            pfx_core::atomic::sweep(&out.join(FRAMES))?;
            (0..resume.frames)
                .map(|index| finished(&frame_path(out, index), resume.size))
                .collect()
        } else {
            fresh(&out.join(FRAMES), frames)?;
            Vec::new()
        };
        pfx_core::atomic::write(&claim, resume.claim.as_bytes())?;
        Files::opened(out, stills, kept)
    }

    fn opened(out: &Path, stills: bool, kept: Vec<bool>) -> Result<Files, String> {
        fresh(&out.join(STILLS), stills)?;
        let audio = out.join(AUDIO);
        if audio.exists() {
            std::fs::remove_file(&audio).map_err(|e| format!("{}: {e}", audio.display()))?;
        }
        pfx_core::atomic::sweep(out)?;
        Ok(Files {
            out: out.to_path_buf(),
            press: Press::open(),
            kept,
        })
    }

    pub fn done(&self) -> u64 {
        self.kept.iter().filter(|&&kept| kept).count() as u64
    }

    pub fn first_missing(&self) -> u64 {
        self.kept
            .iter()
            .position(|&kept| !kept)
            .unwrap_or(self.kept.len()) as u64
    }
}

impl Encoder for Files {
    fn frame(&mut self, index: u64, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        self.press.put(Shot {
            path: frame_path(&self.out, index),
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

    fn kept(&self, index: u64) -> bool {
        usize::try_from(index)
            .ok()
            .and_then(|at| self.kept.get(at))
            .is_some_and(|&kept| kept)
    }
}

#[cfg(test)]
mod tests;
