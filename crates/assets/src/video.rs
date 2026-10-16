use std::path::Path;

pub const FPS: u32 = 60;

pub const MASTER_WHAT: &str = "H.264 CRF 12 from pito-codecs, High, yuv420p, BT.709 limited";

pub fn video_size(text: &str) -> Option<(u32, u32)> {
    match text {
        "720p" => Some((1280, 720)),
        "1080p" => Some((1920, 1080)),
        "1440p" => Some((2560, 1440)),
        "2160p" => Some((3840, 2160)),
        _ => None,
    }
}

pub use pfx_run::encode::Master as Pipe;

pub fn from_file(input: &Path, fps: u32, out: &Path) -> Result<(), String> {
    pfx_run::encode::from_file(input, fps, out, None)
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
