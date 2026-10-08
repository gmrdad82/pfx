use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_post::gpu::GpuChain;
use pfx_post::{Chain, Pass, Tape};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SEED: u32 = 7;
const FRAME: u32 = 30;

fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(BufReader::new(File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("{other:?} is not RGB or RGBA"),
    };
    let rgba = buffer[..info.buffer_size()]
        .chunks_exact(channels)
        .flat_map(|p| [p[0], p[1], p[2], if channels == 4 { p[3] } else { 255 }])
        .collect();
    (info.width, info.height, rgba)
}

fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let mut encoder = png::Encoder::new(File::create(path).unwrap(), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
    println!("wrote {}", path.display());
}

fn decode(v: u8) -> f32 {
    pfx_materials::linear_channel(v as f32 / 255.0)
}

fn half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 255) as i32 - 127 + 15;
    let mantissa = bits & 0x7fffff;
    if exponent <= 0 {
        return sign;
    }
    let rounded = (mantissa >> 13) + ((mantissa >> 12) & 1);
    sign | (((exponent as u32) << 10) + rounded) as u16
}

fn float(value: u16) -> f32 {
    let exponent = ((value >> 10) & 31) as i32;
    let mantissa = (value & 1023) as f32;
    let magnitude = if exponent == 0 {
        mantissa * 2.0f32.powi(-24)
    } else {
        (1.0 + mantissa / 1024.0) * 2.0f32.powi(exponent - 15)
    };
    if value & 0x8000 != 0 {
        -magnitude
    } else {
        magnitude
    }
}

fn output_target(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tape still output"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    OffscreenTarget {
        texture,
        view,
        format: FORMAT,
        width,
        height,
    }
}

struct Still {
    gpu: Gpu,
    input: OffscreenTarget,
    output: OffscreenTarget,
    turns: Turns,
}

impl Still {
    fn render(&mut self, tape: Tape, frame: u32) -> Vec<u8> {
        let mut chain = Chain::new();
        chain.set_tape(tape);
        chain.passes.push(Pass::Encode);
        let post = GpuChain::new(chain, &self.gpu.device, self.input.width, self.input.height);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tape still"),
            });
        post.run(
            &mut encoder,
            &self.input.view,
            None,
            None,
            &self.output.view,
            frame,
            SEED,
        );
        let gpu = &self.gpu;
        let output = &self.output;
        self.turns.time(|| {
            gpu.queue.submit(Some(encoder.finish()));
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        });
        let bytes = self.turns.cpu(|| gpu.readback_rgba16(output).unwrap());
        bytes
            .chunks_exact(4)
            .flat_map(|p| {
                [
                    (float(p[0]).clamp(0.0, 1.0) * 255.0).round() as u8,
                    (float(p[1]).clamp(0.0, 1.0) * 255.0).round() as u8,
                    (float(p[2]).clamp(0.0, 1.0) * 255.0).round() as u8,
                    255,
                ]
            })
            .collect()
    }
}

fn shrink(rgba: &[u8], width: u32, height: u32, factor: u32) -> Vec<u8> {
    let (w, h) = (width / factor, height / factor);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for dy in 0..factor {
                for dx in 0..factor {
                    let i = (((y * factor + dy) * width + x * factor + dx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += rgba[i + c] as u32;
                    }
                }
            }
            out.extend(sum.map(|s| (s / (factor * factor)) as u8));
        }
    }
    out
}

fn crop(rgba: &[u8], width: u32, rect: [u32; 4]) -> Vec<u8> {
    let [x0, y0, x1, y1] = rect;
    let mut out = Vec::new();
    for y in y0..y1 {
        let start = ((y * width + x0) * 4) as usize;
        out.extend_from_slice(&rgba[start..start + ((x1 - x0) * 4) as usize]);
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(
        args.next()
            .expect("usage: tape_still <input.png> <out dir> [x0 y0 x1 y1]"),
    );
    let out = PathBuf::from(args.next().unwrap_or_else(|| "tmp/tape".into()));
    let rect: Vec<u32> = args.map(|v| v.parse().unwrap()).collect();
    std::fs::create_dir_all(&out).unwrap();
    let (width, height, rgba) = read_png(&source);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let input = gpu.offscreen(width, height, FORMAT).unwrap();
    let pixels: Vec<u16> = rgba
        .chunks_exact(4)
        .flat_map(|p| [decode(p[0]), decode(p[1]), decode(p[2]), 1.0].map(half))
        .collect();
    gpu.upload_rgba16(&input, &pixels).unwrap();
    let output = output_target(&gpu, width, height);
    let mut still = Still {
        gpu,
        input,
        output,
        turns: Turns::default(),
    };
    let mut shots = Vec::new();
    for (name, tape) in [
        ("before", Tape::OFF),
        ("forward", Tape::forward(1.0)),
        ("rewind", Tape::rewind(1.0)),
    ] {
        let image = still.render(tape, FRAME);
        save_png(&out.join(format!("{name}.png")), width, height, &image);
        shots.push((name, image));
    }
    if let [x0, y0, x1, y1] = rect[..] {
        for (name, image) in &shots {
            save_png(
                &out.join(format!("{name}-crop.png")),
                x1 - x0,
                y1 - y0,
                &crop(image, width, [x0, y0, x1, y1]),
            );
        }
    }
    let factor = (width / 960).max(1);
    let (w, h) = (width / factor, height / factor);
    let mut strip = Vec::new();
    for i in 0..8u32 {
        let frame = (i as f32 * 60.0 / 8.0).round() as u32;
        let image = still.render(Tape::rewind(1.0), frame);
        strip.extend(shrink(&image, width, height, factor));
    }
    save_png(&out.join("rewind-strip.png"), w, h * 8, &strip);
}
