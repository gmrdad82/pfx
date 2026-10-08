use std::path::Path;
use std::process::{Child, Command};

pub fn stem(product: &str, shot: &str, width: u32, height: u32, fps: u32) -> String {
    format!("{product}-clip-{shot}-{width}x{height}-{fps}fps")
}

pub fn poster(stem: &str) -> String {
    format!("{stem}.poster.png")
}

pub fn master(stem: &str) -> String {
    format!("{stem}.master.mp4")
}

pub fn from_raw(
    ffmpeg: &Path,
    width: u32,
    height: u32,
    fps: u32,
    out: &Path,
) -> Result<Child, String> {
    pfx_run::encode::raw(
        ffmpeg,
        width,
        height,
        fps,
        &pfx_run::encode::master_args(fps),
        out,
    )
}

pub fn from_file(ffmpeg: &Path, input: &Path, fps: u32, out: &Path) -> Result<(), String> {
    pfx_run::encode::file(ffmpeg, input, &pfx_run::encode::master_args(fps), out)
}

pub fn first_frame(ffmpeg: &Path, input: &Path, out: &Path) -> Result<(), String> {
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-y", "-i"])
        .arg(input)
        .args(["-frames:v", "1", "-pix_fmt", "rgba"])
        .arg(out)
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "ffmpeg could not take the first frame of {}",
            input.display()
        ))
    }
}
