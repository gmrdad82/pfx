use std::path::Path;
use std::sync::Arc;

use egui::{
    Color32, ColorImage, Context, Id, Rgba, TextureHandle, TextureId, TextureOptions, Vec2,
};

pub static PFX: &[u8] = include_bytes!("../assets/pfx-icon.png");

const GROUND_SLACK: u8 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    size: [usize; 2],
    linear: Vec<[f32; 4]>,
}

fn channel(value: u8) -> f32 {
    egui::ecolor::linear_f32_from_gamma_u8(value)
}

fn decode(bytes: &[u8]) -> Result<([usize; 2], Vec<[u8; 4]>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("the picture is too large")?
    ];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    buffer.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgba => buffer
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect(),
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .map(|p| [p[0], p[1], p[2], u8::MAX])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks_exact(2)
            .map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buffer.iter().map(|&g| [g, g, g, u8::MAX]).collect(),
        png::ColorType::Indexed => return Err("an indexed picture was not expanded".into()),
    };
    Ok(([info.width as usize, info.height as usize], pixels))
}

fn bounds(size: [usize; 2], pixels: &[[u8; 4]]) -> Option<[usize; 4]> {
    let [width, height] = size;
    let ground = pixels[0];
    let opaque = pixels.iter().all(|p| p[3] == u8::MAX);
    let inked = |p: &[u8; 4]| {
        if opaque {
            (0..3).any(|c| p[c].abs_diff(ground[c]) > GROUND_SLACK)
        } else {
            p[3] > 0
        }
    };
    let mut found: Option<[usize; 4]> = None;
    for y in 0..height {
        for x in 0..width {
            if inked(&pixels[y * width + x]) {
                let [x0, y0, x1, y1] = found.unwrap_or([x, y, x, y]);
                found = Some([x0.min(x), y0.min(y), x1.max(x), y1.max(y)]);
            }
        }
    }
    found.map(|[x0, y0, x1, y1]| [x0, y0, x1 + 1, y1 + 1])
}

fn weights(from: usize, to: usize) -> Vec<Vec<(usize, f32)>> {
    let step = from as f32 / to as f32;
    (0..to)
        .map(|out| {
            let (start, end) = (out as f32 * step, (out + 1) as f32 * step);
            let mut taps = Vec::new();
            let mut at = start.floor() as usize;
            while (at as f32) < end && at < from {
                let cover = (end.min(at as f32 + 1.0) - start.max(at as f32)).max(0.0);
                if cover > 0.0 {
                    taps.push((at, cover / (end - start)));
                }
                at += 1;
            }
            taps
        })
        .collect()
}

impl Mark {
    pub fn read(bytes: &[u8]) -> Result<Mark, String> {
        let (size, pixels) = decode(bytes)?;
        let [x0, y0, x1, y1] = bounds(size, &pixels).ok_or("the picture is empty")?;
        let linear = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| {
                let [r, g, b, a] = pixels[y * size[0] + x];
                let alpha = f32::from(a) / 255.0;
                [
                    channel(r) * alpha,
                    channel(g) * alpha,
                    channel(b) * alpha,
                    alpha,
                ]
            })
            .collect();
        Ok(Mark {
            size: [x1 - x0, y1 - y0],
            linear,
        })
    }

    pub fn open(path: &Path) -> Result<Mark, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        Mark::read(&bytes).map_err(|error| format!("{}: {error}", path.display()))
    }

    pub fn width_for(&self, height: usize) -> usize {
        ((height * self.size[0]) as f32 / self.size[1] as f32)
            .round()
            .max(1.0) as usize
    }

    pub fn image(&self, height: usize) -> ColorImage {
        let height = height.max(1);
        let width = self.width_for(height);
        let [from_w, from_h] = self.size;
        let across = weights(from_w, width);
        let mut rows = vec![[0.0f32; 4]; width * from_h];
        for y in 0..from_h {
            for (x, taps) in across.iter().enumerate() {
                let mut sum = [0.0f32; 4];
                for &(at, weight) in taps {
                    let texel = self.linear[y * from_w + at];
                    for c in 0..4 {
                        sum[c] += texel[c] * weight;
                    }
                }
                rows[y * width + x] = sum;
            }
        }
        let down = weights(from_h, height);
        let pixels = down
            .iter()
            .flat_map(|taps| {
                let rows = &rows;
                (0..width).map(move |x| {
                    let mut sum = [0.0f32; 4];
                    for &(at, weight) in taps {
                        let texel = rows[at * width + x];
                        for c in 0..4 {
                            sum[c] += texel[c] * weight;
                        }
                    }
                    Color32::from(Rgba::from_rgba_premultiplied(
                        sum[0], sum[1], sum[2], sum[3],
                    ))
                })
            })
            .collect();
        ColorImage::new([width, height], pixels)
    }
}

type Read = Arc<Result<Arc<Mark>, String>>;

pub fn source(
    ctx: &Context,
    key: &str,
    read: impl FnOnce() -> Result<Mark, String>,
) -> Result<Arc<Mark>, String> {
    let id = Id::new(("workbench mark source", key));
    let read = match ctx.data(|data| data.get_temp::<Read>(id)) {
        Some(kept) => kept,
        None => {
            let made: Read = Arc::new(read().map(Arc::new));
            ctx.data_mut(|data| data.insert_temp(id, Arc::clone(&made)));
            made
        }
    };
    (*read).clone()
}

#[cfg(test)]
pub fn read_into(ctx: &Context, key: &str) -> bool {
    ctx.data(|data| data.get_temp::<Read>(Id::new(("workbench mark source", key))))
        .is_some()
}

#[derive(Clone)]
struct Shown {
    pixels: usize,
    texture: TextureHandle,
}

pub fn texture(ctx: &Context, key: &str, mark: &Arc<Mark>, height: f32) -> (TextureId, Vec2) {
    let scale = ctx.pixels_per_point();
    let pixels = (height * scale).round().max(1.0) as usize;
    let id = Id::new(("workbench mark", key));
    let kept = ctx.data(|data| data.get_temp::<Shown>(id));
    let shown = match kept {
        Some(shown) if shown.pixels == pixels => shown,
        _ => {
            let image = mark.image(pixels);
            let texture = ctx.load_texture(format!("mark {key}"), image, TextureOptions::NEAREST);
            let shown = Shown { pixels, texture };
            ctx.data_mut(|data| data.insert_temp(id, shown.clone()));
            shown
        }
    };
    let [width, height] = shown.texture.size();
    (
        shown.texture.id(),
        Vec2::new(width as f32, height as f32) / scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(size: [u32; 2], pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, size[0], size[1]);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&pixels.iter().flatten().copied().collect::<Vec<u8>>())
            .unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn pfx_s_mark_is_trimmed_to_its_ink_and_drawn_at_the_asked_height() {
        let mark = Mark::read(PFX).unwrap();
        let [width, height] = mark.size;
        assert!(width < 512 && height < 512, "{width}x{height}");
        assert!(width > 256 && height > 256, "{width}x{height}");
        for pixels in [19, 30, 48] {
            let image = mark.image(pixels);
            assert_eq!(image.size, [mark.width_for(pixels), pixels]);
        }
    }

    #[test]
    fn a_shrunk_mark_keeps_its_average_colour_and_its_clear_ground() {
        let red = [255, 0, 0, 255];
        let clear = [0, 0, 0, 0];
        let mut pixels = vec![clear; 12 * 6];
        for y in 1..5 {
            for x in 2..10 {
                pixels[y * 12 + x] = if (x + y) % 2 == 0 {
                    red
                } else {
                    [0, 0, 255, 255]
                };
            }
        }
        let mark = Mark::read(&png([12, 6], &pixels)).unwrap();
        assert_eq!(mark.size, [8, 4]);
        let image = mark.image(2);
        assert_eq!(image.size, [4, 2]);
        let half = Color32::from(Rgba::from_rgb(0.5, 0.0, 0.5));
        assert!(
            image.pixels.iter().all(|&pixel| pixel == half),
            "{:?}",
            image.pixels
        );
        let edge = Mark::read(&png([3, 1], &[clear, red, clear])).unwrap();
        assert_eq!(edge.size, [1, 1]);
    }

    #[test]
    fn a_picture_with_no_ink_is_refused() {
        let clear = [0, 0, 0, 0];
        assert!(Mark::read(&png([2, 2], &[clear; 4])).is_err());
        assert!(Mark::read(b"not a png").is_err());
    }
}
