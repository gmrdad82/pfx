use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin};

pub const FPS: u32 = 60;

pub fn video_size(text: &str) -> Option<(u32, u32)> {
    match text {
        "720p" => Some((1280, 720)),
        "1080p" => Some((1920, 1080)),
        "1440p" => Some((2560, 1440)),
        "2160p" => Some((3840, 2160)),
        _ => None,
    }
}

fn ffmpeg() -> Result<PathBuf, String> {
    pfx_run::ffmpeg::resolve(&pfx_run::ffmpeg::cache()?, "pfx", &pfx_run::ffmpeg::pin()?)
}

pub struct Pipe {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Pipe {
    pub fn open(width: u32, height: u32, fps: u32, out: &Path) -> Result<Pipe, String> {
        let mut child = pfx_run::encode::raw(
            &ffmpeg()?,
            width,
            height,
            fps,
            &pfx_run::encode::master_args(fps),
            out,
        )?;
        let stdin = child.stdin.take();
        Ok(Pipe { child, stdin })
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.stdin
            .as_mut()
            .ok_or("ffmpeg closed its input")?
            .write_all(bytes)
            .map_err(|e| format!("ffmpeg: {e}"))
    }

    pub fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("ffmpeg failed: {status}"))
        }
    }
}

pub fn from_file(input: &Path, fps: u32, out: &Path) -> Result<(), String> {
    pfx_run::encode::file(&ffmpeg()?, input, &pfx_run::encode::master_args(fps), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_sizes_are_sixteen_by_nine() {
        assert_eq!(video_size("1440p"), Some((2560, 1440)));
        assert_eq!(video_size("999p"), None);
    }
}
