#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    Linear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pixels {
    Eight(Vec<u8>),
    Sixteen(Vec<u16>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub space: ColorSpace,
    pub pixels: Pixels,
}

impl Image {
    pub fn png(bytes: &[u8]) -> Result<Self, String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|e| format!("png: {e}"))?;
        let space = colour(reader.info());
        let (width, height) = reader.info().size();
        if width == 0 || height == 0 {
            return Err("png: the image is empty".into());
        }
        let size = reader
            .output_buffer_size()
            .ok_or("png: the image is too large")?;
        let mut buf = vec![0u8; size];
        let frame = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("png: {e}"))?;
        let pixels = expand(&frame, &buf)?;
        Ok(Image {
            width: frame.width,
            height: frame.height,
            space,
            pixels,
        })
    }

    pub fn rgba16(
        bytes: &[u8],
        width: u32,
        height: u32,
        space: ColorSpace,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("rgba16: the image is empty".into());
        }
        let samples = (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(4))
            .ok_or("rgba16: the image is too large")?;
        let need = samples
            .checked_mul(2)
            .ok_or("rgba16: the image is too large")?;
        if bytes.len() != need {
            return Err(format!(
                "rgba16: {width}x{height} needs {need} bytes, got {}",
                bytes.len()
            ));
        }
        let pixels = bytes
            .chunks_exact(2)
            .map(|sample| u16::from_le_bytes([sample[0], sample[1]]))
            .collect();
        Ok(Image {
            width,
            height,
            space,
            pixels: Pixels::Sixteen(pixels),
        })
    }
}

fn colour(info: &png::Info) -> ColorSpace {
    if info.srgb.is_some() {
        return ColorSpace::Srgb;
    }
    if let Some(gamma) = info.gama_chunk {
        let value = gamma.into_value();
        if (value - 1.0).abs() <= 0.01 {
            return ColorSpace::Linear;
        }
        if (value - (1.0 / 2.2)).abs() <= 0.02 {
            return ColorSpace::Srgb;
        }
    }
    if info.bit_depth == png::BitDepth::Sixteen {
        ColorSpace::Linear
    } else {
        ColorSpace::Srgb
    }
}

fn expand(frame: &png::OutputInfo, buf: &[u8]) -> Result<Pixels, String> {
    if frame.color_type == png::ColorType::Indexed {
        return Err("png: the palette was not expanded".into());
    }
    let channels = frame.color_type.samples();
    let wide = match frame.bit_depth {
        png::BitDepth::Eight => false,
        png::BitDepth::Sixteen => true,
        depth => return Err(format!("png: bit depth {} is unsupported", depth as u8)),
    };
    let sample_bytes = if wide { 2 } else { 1 };
    let pixel_bytes = channels * sample_bytes;
    if frame.line_size < frame.width as usize * pixel_bytes {
        return Err("png: a row is short".into());
    }
    let height = frame.height as usize;
    let width = frame.width as usize;
    if wide {
        let mut out = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            let row = buf
                .get(y * frame.line_size..)
                .ok_or("png: the frame is short")?;
            for x in 0..width {
                let pixel = row.get(x * pixel_bytes..).ok_or("png: a row is short")?;
                let sample =
                    |index: usize| u16::from_be_bytes([pixel[index * 2], pixel[index * 2 + 1]]);
                let (r, g, b, a) = match channels {
                    1 => {
                        let value = sample(0);
                        (value, value, value, 65535)
                    }
                    2 => {
                        let value = sample(0);
                        (value, value, value, sample(1))
                    }
                    3 => (sample(0), sample(1), sample(2), 65535),
                    4 => (sample(0), sample(1), sample(2), sample(3)),
                    _ => return Err("png: an unexpected channel count".into()),
                };
                out.extend([r, g, b, a]);
            }
        }
        Ok(Pixels::Sixteen(out))
    } else {
        let mut out = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            let row = buf
                .get(y * frame.line_size..)
                .ok_or("png: the frame is short")?;
            for x in 0..width {
                let pixel = row.get(x * pixel_bytes..).ok_or("png: a row is short")?;
                let (r, g, b, a) = match channels {
                    1 => (pixel[0], pixel[0], pixel[0], 255),
                    2 => (pixel[0], pixel[0], pixel[0], pixel[1]),
                    3 => (pixel[0], pixel[1], pixel[2], 255),
                    4 => (pixel[0], pixel[1], pixel[2], pixel[3]),
                    _ => return Err("png: an unexpected channel count".into()),
                };
                out.extend([r, g, b, a]);
            }
        }
        Ok(Pixels::Eight(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swatch8() -> Vec<u8> {
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 16, 32, 64, 128,
        ]
    }

    fn swatch16() -> Vec<u16> {
        vec![
            65535, 0, 0, 65535, 0, 65535, 0, 65535, 0, 0, 65535, 65535, 256, 512, 1024, 32768,
        ]
    }

    fn encode(image: &Image) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        match image.space {
            ColorSpace::Srgb => {
                encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
            }
            ColorSpace::Linear => {
                encoder.set_source_gamma(png::ScaledFloat::from_scaled(100_000));
            }
        }
        let data = match &image.pixels {
            Pixels::Eight(pixels) => {
                encoder.set_depth(png::BitDepth::Eight);
                pixels.clone()
            }
            Pixels::Sixteen(pixels) => {
                encoder.set_depth(png::BitDepth::Sixteen);
                pixels
                    .iter()
                    .flat_map(|sample| sample.to_be_bytes())
                    .collect()
            }
        };
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&data).unwrap();
        drop(writer);
        bytes
    }

    #[test]
    fn eight_and_sixteen_bit_pngs_round_trip() {
        let eight = Image::png(include_bytes!("../tests/data/swatch8.png")).unwrap();
        assert_eq!(eight.width, 2);
        assert_eq!(eight.height, 2);
        assert_eq!(eight.space, ColorSpace::Srgb);
        assert_eq!(eight.pixels, Pixels::Eight(swatch8()));
        let again = Image::png(&encode(&eight)).unwrap();
        assert_eq!(again, eight);

        let sixteen = Image::png(include_bytes!("../tests/data/swatch16.png")).unwrap();
        assert_eq!(sixteen.space, ColorSpace::Linear);
        assert_eq!(sixteen.pixels, Pixels::Sixteen(swatch16()));
        let again = Image::png(&encode(&sixteen)).unwrap();
        assert_eq!(again, sixteen);

        let mut linear = eight.clone();
        linear.space = ColorSpace::Linear;
        let decoded = Image::png(&encode(&linear)).unwrap();
        assert_eq!(decoded.space, ColorSpace::Linear);
        assert_eq!(decoded.pixels, eight.pixels);

        let mut srgb = sixteen.clone();
        srgb.space = ColorSpace::Srgb;
        let decoded = Image::png(&encode(&srgb)).unwrap();
        assert_eq!(decoded.space, ColorSpace::Srgb);
        assert_eq!(decoded.pixels, sixteen.pixels);
    }

    #[test]
    fn raw_rgba16_keeps_both_bytes_and_its_stated_space() {
        let image = Image::rgba16(
            include_bytes!("../tests/data/swatch.rgba16"),
            2,
            2,
            ColorSpace::Linear,
        )
        .unwrap();
        assert_eq!(image.pixels, Pixels::Sixteen(swatch16()));
        assert_eq!(image.space, ColorSpace::Linear);
        assert!(Image::rgba16(&[0, 1], 1, 1, ColorSpace::Linear).is_err());
    }
}
