use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "pfx-alpha-master";
pub const VERSION: u32 = 1;
pub const MANIFEST: &str = "manifest.json";
pub const SUFFIX: &str = ".alpha.master";
pub const COLOUR: &str = "sRGB (IEC 61966-2-1): BT.709 primaries, D65 white, the sRGB transfer, full range; each PNG carries an sRGB chunk with the gAMA and cHRM fallbacks";
pub const ALPHA: &str = "straight";
pub const CHANNELS: &str = "RGBA";
pub const BIT_DEPTH: u8 = 16;
pub const FRAME_WHAT: &str = "alpha master frame, 16-bit RGBA PNG, straight alpha, sRGB";
pub const MANIFEST_WHAT: &str =
    "alpha master: 16-bit RGBA PNG frames, straight alpha, sRGB, with their sha256";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameFile {
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub size: [u32; 2],
    pub fps: u32,
    pub frames: usize,
    pub colour: String,
    pub alpha: String,
    pub channels: String,
    pub bit_depth: u8,
    pub rev: String,
    pub files: Vec<FrameFile>,
}

pub fn folder(stem: &str) -> String {
    format!("{stem}{SUFFIX}")
}

pub fn frame_name(index: usize) -> String {
    format!("{index:06}.png")
}

pub fn encode(width: u32, height: u32, rgba: &[u16]) -> Result<Vec<u8>, String> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(format!(
            "an alpha master frame of {width}x{height} holds {expected} values, got {}",
            rgba.len()
        ));
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.set_source_gamma(png::ScaledFloat::from_scaled(45455));
    encoder.set_source_chromaticities(png::SourceChromaticities {
        white: (
            png::ScaledFloat::from_scaled(31270),
            png::ScaledFloat::from_scaled(32900),
        ),
        red: (
            png::ScaledFloat::from_scaled(64000),
            png::ScaledFloat::from_scaled(33000),
        ),
        green: (
            png::ScaledFloat::from_scaled(30000),
            png::ScaledFloat::from_scaled(60000),
        ),
        blue: (
            png::ScaledFloat::from_scaled(15000),
            png::ScaledFloat::from_scaled(6000),
        ),
    });
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = rgba.iter().flat_map(|v| v.to_be_bytes()).collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn decode(path: &Path) -> Result<(u32, u32, Vec<u16>), String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| at(&e))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|e| at(&e))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| at(&"too large"))?
    ];
    let info = reader.next_frame(&mut buf).map_err(|e| at(&e))?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Sixteen {
        return Err(at(&"not a 16-bit RGBA PNG"));
    }
    let rgba = buf[..info.buffer_size()]
        .chunks_exact(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect();
    Ok((info.width, info.height, rgba))
}

pub struct Writer {
    dir: PathBuf,
    manifest: Manifest,
}

impl Writer {
    pub fn create(
        dir: &Path,
        width: u32,
        height: u32,
        fps: u32,
        rev: &str,
    ) -> Result<Writer, String> {
        let at = |e: std::io::Error| format!("{}: {e}", dir.display());
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(at)?;
        }
        std::fs::create_dir_all(dir).map_err(at)?;
        Ok(Writer {
            dir: dir.to_path_buf(),
            manifest: Manifest {
                format: FORMAT.into(),
                version: VERSION,
                size: [width, height],
                fps,
                frames: 0,
                colour: COLOUR.into(),
                alpha: ALPHA.into(),
                channels: CHANNELS.into(),
                bit_depth: BIT_DEPTH,
                rev: rev.into(),
                files: Vec::new(),
            },
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn push(&mut self, rgba: &[u16]) -> Result<PathBuf, String> {
        let [width, height] = self.manifest.size;
        self.push_encoded(&encode(width, height, rgba)?)
    }

    pub fn push_encoded(&mut self, bytes: &[u8]) -> Result<PathBuf, String> {
        let file = frame_name(self.manifest.files.len());
        let path = self.dir.join(&file);
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        self.manifest.files.push(FrameFile {
            file,
            bytes: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
        });
        self.manifest.frames = self.manifest.files.len();
        Ok(path)
    }

    pub fn finish(self) -> Result<(PathBuf, Manifest), String> {
        if self.manifest.files.is_empty() {
            return Err(format!("{} holds no frames", self.dir.display()));
        }
        let path = self.dir.join(MANIFEST);
        let text = serde_json::to_vec_pretty(&self.manifest).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok((path, self.manifest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(width: u32, height: u32, count: usize) -> Vec<Vec<u16>> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        (0..count)
            .map(|_| {
                (0..width * height * 4)
                    .map(|_| {
                        state = state
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        (state >> 48) as u16
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_cutout_sequence_round_trips_every_value_and_its_manifest_lists_the_frames() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("alpha-master-{}", std::process::id()))
            .join(folder("clip"));
        let mut input = frames(7, 5, 3);
        input[0][3] = 0;
        input[0][0] = 40000;
        input[1][7] = u16::MAX;
        let mut writer = Writer::create(&dir, 7, 5, 60, "aabbccddeeff").unwrap();
        let written: Vec<PathBuf> = input.iter().map(|f| writer.push(f).unwrap()).collect();
        let (path, manifest) = writer.finish().unwrap();
        assert_eq!(path, dir.join(MANIFEST));
        for (n, (file, want)) in written.iter().zip(&input).enumerate() {
            assert_eq!(file, &dir.join(format!("{n:06}.png")));
            let (width, height, got) = decode(file).unwrap();
            assert_eq!((width, height), (7, 5));
            assert_eq!(&got, want);
            let bytes = std::fs::read(file).unwrap();
            for chunk in [b"sRGB", b"gAMA", b"cHRM"] {
                assert!(bytes.windows(4).any(|w| w == chunk), "{chunk:?}");
            }
        }
        let read: Manifest = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read, manifest);
        assert_eq!((read.format.as_str(), read.version), (FORMAT, VERSION));
        assert_eq!((read.size, read.fps, read.frames), ([7, 5], 60, 3));
        assert_eq!((read.alpha.as_str(), read.bit_depth), ("straight", 16));
        assert_eq!(read.rev, "aabbccddeeff");
        let names: Vec<&str> = read.files.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(names, ["000000.png", "000001.png", "000002.png"]);
        for entry in &read.files {
            let bytes = std::fs::read(dir.join(&entry.file)).unwrap();
            assert_eq!(entry.bytes, bytes.len() as u64);
            assert_eq!(entry.sha256, hex::encode(Sha256::digest(&bytes)));
        }
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_frame_of_the_wrong_size_and_an_empty_master_are_refused() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("alpha-refuse-{}", std::process::id()));
        let mut writer = Writer::create(&dir, 2, 2, 30, "r").unwrap();
        assert!(writer.push(&[0; 12]).unwrap_err().contains("16 values"));
        assert!(writer.finish().unwrap_err().contains("no frames"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
