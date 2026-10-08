use std::io::BufWriter;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u16>,
}

pub struct Linear {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
}

fn decode_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=u16::MAX)
            .map(|v| {
                let c = v as f32 / 65535.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            })
            .collect()
    })
}

fn encode(c: f32) -> u16 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 65535.0 + 0.5) as u16
}

impl Frame {
    pub fn read_raw(path: &Path, width: u32, height: u32) -> Result<Frame, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let expected = width as usize * height as usize * 8;
        if bytes.len() != expected {
            return Err(format!(
                "{}: {} bytes, expected {expected} for {width}x{height} RGBA16",
                path.display(),
                bytes.len()
            ));
        }
        let rgba = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        Ok(Frame {
            width,
            height,
            rgba,
        })
    }

    pub fn load(path: &Path) -> Result<Frame, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::IDENTITY);
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Sixteen {
            return Err(format!("{}: expected RGBA 16-bit", path.display()));
        }
        let rgba = buf[..info.buffer_size()]
            .chunks_exact(2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .collect();
        Ok(Frame {
            width: info.width,
            height: info.height,
            rgba,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = self.rgba.iter().flat_map(|v| v.to_be_bytes()).collect();
        writer.write_image_data(&bytes).map_err(|e| e.to_string())
    }

    pub fn save8(&self, path: &Path) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = self
            .rgba
            .iter()
            .map(|&v| ((v as u32 * 255 + 32767) / 65535) as u8)
            .collect();
        writer.write_image_data(&bytes).map_err(|e| e.to_string())
    }

    pub fn raw(&self) -> Vec<u8> {
        self.rgba.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    pub fn linear(&self) -> Linear {
        let table = decode_table();
        let rgba = self
            .rgba
            .chunks_exact(4)
            .flat_map(|p| {
                [
                    table[p[0] as usize],
                    table[p[1] as usize],
                    table[p[2] as usize],
                    p[3] as f32 / 65535.0,
                ]
            })
            .collect();
        Linear {
            width: self.width,
            height: self.height,
            rgba,
        }
    }
}

impl Linear {
    pub fn frame(&self) -> Frame {
        let rgba = self
            .rgba
            .chunks_exact(4)
            .flat_map(|p| {
                [
                    encode(p[0]),
                    encode(p[1]),
                    encode(p[2]),
                    (p[3].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16,
                ]
            })
            .collect();
        Frame {
            width: self.width,
            height: self.height,
            rgba,
        }
    }

    pub fn mix(&self, other: &Linear, t: f32) -> Linear {
        let rgba = self
            .rgba
            .iter()
            .zip(&other.rgba)
            .map(|(a, b)| a + (b - a) * t)
            .collect();
        Linear {
            width: self.width,
            height: self.height,
            rgba,
        }
    }

    pub fn resize(&self, width: u32, height: u32) -> Linear {
        if (width, height) == (self.width, self.height) {
            return Linear {
                width,
                height,
                rgba: self.rgba.clone(),
            };
        }
        let premultiplied: Vec<f32> = self
            .rgba
            .chunks_exact(4)
            .flat_map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]])
            .collect();
        let across = weights(self.width, width);
        let mut mid = vec![0.0f32; width as usize * self.height as usize * 4];
        for y in 0..self.height as usize {
            let row =
                &premultiplied[y * self.width as usize * 4..(y + 1) * self.width as usize * 4];
            for (x, taps) in across.iter().enumerate() {
                let mut acc = [0.0f32; 4];
                for &(i, w) in taps {
                    for c in 0..4 {
                        acc[c] += row[i * 4 + c] * w;
                    }
                }
                mid[(y * width as usize + x) * 4..(y * width as usize + x) * 4 + 4]
                    .copy_from_slice(&acc);
            }
        }
        let down = weights(self.height, height);
        let mut out = vec![0.0f32; width as usize * height as usize * 4];
        for (y, taps) in down.iter().enumerate() {
            for x in 0..width as usize {
                let mut acc = [0.0f32; 4];
                for &(i, w) in taps {
                    for c in 0..4 {
                        acc[c] += mid[(i * width as usize + x) * 4 + c] * w;
                    }
                }
                let a = acc[3].clamp(0.0, 1.0);
                let un = if a > 1e-6 { 1.0 / a } else { 0.0 };
                out[(y * width as usize + x) * 4..(y * width as usize + x) * 4 + 4]
                    .copy_from_slice(&[acc[0] * un, acc[1] * un, acc[2] * un, a]);
            }
        }
        Linear {
            width,
            height,
            rgba: out,
        }
    }
}

fn lanczos(x: f32) -> f32 {
    let x = x.abs();
    if x < 1e-6 {
        return 1.0;
    }
    if x >= 3.0 {
        return 0.0;
    }
    let px = std::f32::consts::PI * x;
    3.0 * px.sin() * (px / 3.0).sin() / (px * px)
}

fn weights(from: u32, to: u32) -> Vec<Vec<(usize, f32)>> {
    let scale = from as f32 / to as f32;
    let stretch = scale.max(1.0);
    let reach = 3.0 * stretch;
    (0..to)
        .map(|o| {
            let centre = (o as f32 + 0.5) * scale;
            let lo = (centre - reach).floor().max(0.0) as usize;
            let hi = ((centre + reach).ceil() as usize).min(from as usize);
            let mut taps: Vec<(usize, f32)> = (lo..hi)
                .map(|i| (i, lanczos((i as f32 + 0.5 - centre) / stretch)))
                .filter(|t| t.1 != 0.0)
                .collect();
            let total: f32 = taps.iter().map(|t| t.1).sum();
            for t in &mut taps {
                t.1 /= total;
            }
            taps
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: u32, height: u32, v: u16) -> Frame {
        Frame {
            width,
            height,
            rgba: vec![v; (width * height * 4) as usize],
        }
    }

    #[test]
    fn a_flat_frame_stays_flat_through_linear_light_and_a_downscale() {
        let f = flat(64, 36, 40000);
        let small = f.linear().resize(32, 18).frame();
        assert_eq!((small.width, small.height), (32, 18));
        assert!(small.rgba.iter().all(|&v| (v as i32 - 40000).abs() <= 2));
    }

    #[test]
    fn half_way_between_black_and_white_is_mid_grey_in_linear_light() {
        let (black, white) = (flat(4, 4, 0).linear(), flat(4, 4, 65535).linear());
        let grey = black.mix(&white, 0.5).frame();
        let expected = encode(0.5);
        assert!((grey.rgba[0] as i32 - expected as i32).abs() <= 1);
        assert!(grey.rgba[0] > 32767);
    }

    #[test]
    fn a_transparent_neighbour_never_tints_an_edge() {
        let mut rgba = Vec::new();
        for x in 0..8 {
            if x < 4 {
                rgba.extend([1.0, 0.0, 0.0, 1.0]);
            } else {
                rgba.extend([0.0, 1.0, 0.0, 0.0]);
            }
        }
        let small = Linear {
            width: 8,
            height: 1,
            rgba,
        }
        .resize(4, 1);
        for p in small.rgba.chunks_exact(4).filter(|p| p[3] > 0.01) {
            assert!(p[1].abs() < 1e-4, "{p:?}");
        }
    }

    #[test]
    fn png16_round_trips() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("test-png-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = flat(3, 2, 1234);
        f.rgba[5] = 65535;
        let path = dir.join("f.png");
        f.save(&path).unwrap();
        assert_eq!(Frame::load(&path).unwrap(), f);
        std::fs::remove_dir_all(&dir).ok();
    }
}
