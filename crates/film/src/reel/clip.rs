use super::footage::{EPSILON, Picture, Source, sample};
use crate::score::CHANNELS;
use crate::shoot::RATE;
use pito_codecs::media::{self, AlphaVideo, AudioStream, Frame, Plane, Prefs, Video};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

enum Feed {
    Plain(Video),
    Alpha(Box<AlphaVideo>),
}

type Shot = (Frame, Option<Plane>);

pub struct Clip {
    path: PathBuf,
    video: Feed,
    rotation: u16,
    next: Option<Shot>,
    held: Option<Shot>,
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
        let video = if probe.alpha {
            Feed::Alpha(Box::new(media::decode_alpha(path, from).map_err(show)?))
        } else {
            Feed::Plain(media::decode_video(path, from, None, &prefs).map_err(show)?)
        };
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
        let pulled = match &mut self.video {
            Feed::Plain(video) => video.next().map(|frame| frame.map(|frame| (frame, None))),
            Feed::Alpha(video) => video.next(),
        };
        match pulled {
            Some(shot) => {
                self.next = Some(shot.map_err(|e| format!("{}: {e}", self.path.display()))?)
            }
            None => self.ended = true,
        }
        Ok(())
    }

    fn recycle(&mut self, frame: Frame) {
        if let Feed::Plain(video) = &mut self.video {
            video.recycle(frame);
        }
    }

    fn take(&mut self) {
        if let Some((old, _)) = self.held.take() {
            self.recycle(old);
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
                Some((frame, _)) if first || seconds(frame) <= at + EPSILON => self.take(),
                _ => break,
            }
        }
        if let Some((frame, alpha)) = self.held.take() {
            let mut rgba = match &self.video {
                Feed::Alpha(video) => media::to_rgba_alpha(&frame, alpha.as_ref(), 0, video.color),
                Feed::Plain(_) => media::to_rgba(&frame),
            };
            if !self.rotation.is_multiple_of(360) {
                rgba = media::rotate(&rgba, self.rotation);
            }
            self.recycle(frame);
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

#[cfg(test)]
mod tests {
    use super::*;
    use pito_codecs::media::{Canvas, Codec, ExportOptions, Rgba, export_rgba};

    #[test]
    fn a_webm_with_alpha_keeps_its_transparency() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("film-alpha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("half.webm");
        let (w, h) = (32u32, 32u32);
        let canvas = Canvas {
            width: w,
            height: h,
            fps: (4, 1),
            frames: Some(4),
        };
        let opts = ExportOptions {
            video: Codec::Vp9,
            ..ExportOptions::default()
        };
        export_rgba(
            &path,
            canvas,
            true,
            opts,
            |n, _| {
                if n >= 4 {
                    return Ok(None);
                }
                let mut frame = Rgba::new(w, h);
                for (i, px) in frame.data.chunks_exact_mut(4).enumerate() {
                    let opaque = (i as u32 % w) >= w / 2;
                    px.copy_from_slice(&[220, 40, 40, if opaque { 255 } else { 0 }]);
                }
                Ok(Some(frame))
            },
            |_| true,
        )
        .unwrap();
        let mut clip = Clip::open(&path, 0.0, 0).unwrap();
        let picture = clip.picture(0.0).unwrap().unwrap();
        let alpha = |x: u32, y: u32| picture.rgba[((y * w + x) * 4 + 3) as usize];
        assert!(alpha(4, 16) < 16, "the clear half reads {}", alpha(4, 16));
        assert!(
            alpha(28, 16) > 240,
            "the opaque half reads {}",
            alpha(28, 16)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
