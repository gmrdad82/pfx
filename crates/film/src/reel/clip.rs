use super::footage::{EPSILON, Picture, Source, sample};
use crate::score::CHANNELS;
use crate::shoot::RATE;
use pito_codecs::media::{self, AudioStream, Frame, Prefs, Video};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

pub struct Clip {
    path: PathBuf,
    video: Video,
    rotation: u16,
    next: Option<Frame>,
    held: Option<Frame>,
    ended: bool,
    shown: Option<Picture>,
    stamp: u64,
    sound: Option<AudioStream>,
    queue: VecDeque<f32>,
    cursor: u64,
}

fn seconds(frame: &Frame) -> f64 {
    frame.time_base.seconds(frame.pts)
}

impl Clip {
    pub fn open(path: &Path, from: f64, track: usize) -> Result<Clip, String> {
        let show = |e: media::Error| format!("{}: {e}", path.display());
        let prefs = Prefs::default();
        let probe = media::probe(path, &prefs).map_err(show)?;
        let from = from.max(0.0);
        let video = media::decode_video(path, from, None, &prefs).map_err(show)?;
        let sound = if track < probe.audio.len() {
            Some(media::audio_stream(path, track, from, RATE, CHANNELS, &prefs).map_err(show)?)
        } else {
            None
        };
        Ok(Clip {
            path: path.to_path_buf(),
            video,
            rotation: probe.rotation,
            next: None,
            held: None,
            ended: false,
            shown: None,
            stamp: 0,
            sound,
            queue: VecDeque::new(),
            cursor: sample(from),
        })
    }

    fn pull(&mut self) -> Result<(), String> {
        if self.next.is_some() || self.ended {
            return Ok(());
        }
        match self.video.next() {
            Some(frame) => {
                self.next = Some(frame.map_err(|e| format!("{}: {e}", self.path.display()))?)
            }
            None => self.ended = true,
        }
        Ok(())
    }

    fn take(&mut self) {
        if let Some(old) = self.held.take() {
            self.video.recycle(old);
        }
        self.held = self.next.take();
    }
}

impl Source for Clip {
    fn picture(&mut self, at: f64) -> Result<Option<&Picture>, String> {
        loop {
            self.pull()?;
            let first = self.shown.is_none() && self.held.is_none();
            match &self.next {
                Some(frame) if first || seconds(frame) <= at + EPSILON => self.take(),
                _ => break,
            }
        }
        if let Some(frame) = self.held.take() {
            let mut rgba = media::to_rgba(&frame);
            if !self.rotation.is_multiple_of(360) {
                rgba = media::rotate(&rgba, self.rotation);
            }
            self.video.recycle(frame);
            self.stamp += 1;
            self.shown = Some(Picture {
                size: [rgba.width, rgba.height],
                rgba: rgba.data,
                stamp: self.stamp,
            });
        }
        Ok(self.shown.as_ref())
    }

    fn audio(&mut self, from: u64, count: usize, out: &mut Vec<f32>) -> Result<(), String> {
        let want = (from.saturating_sub(self.cursor) as usize + count) * 2;
        while self.queue.len() < want {
            let Some(stream) = self.sound.as_mut() else {
                break;
            };
            match stream.next() {
                Some(block) => {
                    let block = block.map_err(|e| format!("{}: {e}", self.path.display()))?;
                    self.queue.extend(block.samples);
                }
                None => self.sound = None,
            }
        }
        let skip = (from.saturating_sub(self.cursor) as usize * 2).min(self.queue.len());
        self.queue.drain(..skip);
        self.cursor = self.cursor.max(from);
        for _ in 0..count * 2 {
            out.push(self.queue.pop_front().unwrap_or(0.0));
        }
        self.cursor += count as u64;
        Ok(())
    }
}
