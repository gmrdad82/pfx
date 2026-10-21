mod aux;
mod net;
pub mod pfm;
mod setup;
mod tile;
mod tza;
mod wgsl;

use bytemuck::{Pod, Zeroable};
use pfx_gpu::Gpu;
use wgpu::util::DeviceExt;

use crate::aux::Filter;
use crate::net::Net;
use crate::tile::Plan;

pub use crate::aux::Aux;

pub const OIDN_VERSION: &str = "2.5.1";
const MACS_PER_SUBMISSION: u64 = 24_000_000_000;
const WEIGHTS: &[u8] = include_bytes!("../weights/rt_hdr_alb_nrm.tza");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Job {
    src: [i32; 2],
    dst: [i32; 2],
    size: [i32; 2],
    out_src: [i32; 2],
    out_dst: [i32; 2],
    out_size: [i32; 2],
    dims: [u32; 2],
    image: [u32; 2],
    stride: u32,
    norm: f32,
    rcp_norm: f32,
    pad: u32,
}

struct Step {
    pipeline: usize,
    bindings: wgpu::BindGroup,
    groups: [u32; 3],
    macs: u64,
}

struct Images {
    color: wgpu::Buffer,
    albedo: wgpu::Buffer,
    normal: wgpu::Buffer,
    output: wgpu::Buffer,
    clean_albedo: Option<wgpu::Buffer>,
    clean_normal: Option<wgpu::Buffer>,
}

struct Setup {
    width: u32,
    height: u32,
    stride: u32,
    plan: Plan,
    images: Images,
    pipelines: Vec<wgpu::ComputePipeline>,
    steps: Vec<Step>,
}

struct Model {
    filter: Filter,
    net: Net,
    weights: Vec<wgpu::Buffer>,
    biases: Vec<wgpu::Buffer>,
}

pub struct Denoiser {
    aux: Aux,
    models: Vec<Model>,
    max_tile_pixels: u64,
    setup: Option<Setup>,
}

fn norm_scales() -> (f32, f32) {
    let max = 1.926_532_5e-1 * (65504.0f32 + 6.260_261e-3).ln() + 9.986_201_5e-1;
    ((1.0f64 / f64::from(max)) as f32, max)
}

impl Denoiser {
    pub fn new(gpu: &Gpu) -> Result<Self, String> {
        Self::with_aux(gpu, Aux::Noisy)
    }

    pub fn with_aux(gpu: &Gpu, aux: Aux) -> Result<Self, String> {
        let models = aux
            .filters()
            .iter()
            .map(|&filter| model(gpu, filter, aux))
            .collect::<Result<_, String>>()?;
        Ok(Self {
            aux,
            models,
            max_tile_pixels: tile::DEFAULT_MAX_TILE_PIXELS,
            setup: None,
        })
    }

    pub fn aux(&self) -> Aux {
        self.aux
    }

    pub fn tiles(&self) -> Option<(u32, u32, usize)> {
        self.setup
            .as_ref()
            .map(|setup| (setup.plan.tile_w, setup.plan.tile_h, setup.plan.tiles.len()))
    }

    pub fn denoise(
        &mut self,
        gpu: &Gpu,
        width: u32,
        height: u32,
        color: &[[f32; 4]],
        albedo: &[[f32; 4]],
        normal: &[[f32; 4]],
    ) -> Result<Vec<[f32; 4]>, String> {
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or("image dimensions overflow")?;
        if color.len() != pixels || albedo.len() != pixels || normal.len() != pixels {
            return Err("image size mismatch".into());
        }
        self.prepare(gpu, width, height)?;
        let setup = self.setup.as_ref().unwrap();
        for (buffer, pixels) in [
            (&setup.images.color, color),
            (&setup.images.albedo, albedo),
            (&setup.images.normal, normal),
        ] {
            upload(gpu, buffer, width, setup.stride, pixels);
        }
        self.run(gpu)?;
        self.read_output(gpu)
    }

    pub fn denoise_textures(
        &mut self,
        gpu: &Gpu,
        color: &wgpu::Texture,
        albedo: &wgpu::Texture,
        normal: &wgpu::Texture,
    ) -> Result<Vec<[f32; 4]>, String> {
        self.load_textures(gpu, color, albedo, normal)?;
        self.run(gpu)?;
        self.read_output(gpu)
    }

    pub fn denoise_textures_into(
        &mut self,
        gpu: &Gpu,
        color: &wgpu::Texture,
        albedo: &wgpu::Texture,
        normal: &wgpu::Texture,
        output: &wgpu::Texture,
    ) -> Result<(), String> {
        check_texture(output, color.width(), color.height())?;
        if !output.usage().contains(wgpu::TextureUsages::COPY_DST) {
            return Err("the output texture needs COPY_DST".into());
        }
        self.load_textures(gpu, color, albedo, normal)?;
        self.run(gpu)?;
        let setup = self.setup.as_ref().unwrap();
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("denoise output"),
            });
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &setup.images.output,
                layout: image_layout(setup.stride),
            },
            output.as_image_copy(),
            extent(setup.width, setup.height),
        );
        gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    fn load_textures(
        &mut self,
        gpu: &Gpu,
        color: &wgpu::Texture,
        albedo: &wgpu::Texture,
        normal: &wgpu::Texture,
    ) -> Result<(), String> {
        let (width, height) = (color.width(), color.height());
        for texture in [color, albedo, normal] {
            check_texture(texture, width, height)?;
            if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
                return Err("the input textures need COPY_SRC".into());
            }
        }
        self.prepare(gpu, width, height)?;
        let setup = self.setup.as_ref().unwrap();
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("denoise inputs"),
            });
        for (texture, buffer) in [
            (color, &setup.images.color),
            (albedo, &setup.images.albedo),
            (normal, &setup.images.normal),
        ] {
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: image_layout(setup.stride),
                },
                extent(width, height),
            );
        }
        gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    fn run(&self, gpu: &Gpu) -> Result<(), String> {
        let setup = self.setup.as_ref().ok_or("the denoiser has no image")?;
        let mut at = 0;
        while at < setup.steps.len() {
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("denoise"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("denoise"),
                    timestamp_writes: None,
                });
                let mut spent = 0u64;
                while at < setup.steps.len() {
                    let step = &setup.steps[at];
                    if spent > 0 && spent + step.macs > MACS_PER_SUBMISSION {
                        break;
                    }
                    pass.set_pipeline(&setup.pipelines[step.pipeline]);
                    pass.set_bind_group(0, &step.bindings, &[]);
                    pass.dispatch_workgroups(step.groups[0], step.groups[1], step.groups[2]);
                    spent += step.macs;
                    at += 1;
                }
            }
            gpu.queue.submit(Some(encoder.finish()));
        }
        Ok(())
    }

    fn read_output(&self, gpu: &Gpu) -> Result<Vec<[f32; 4]>, String> {
        let setup = self.setup.as_ref().ok_or("the denoiser has no image")?;
        let size = setup.images.output.size();
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("denoise readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("denoise readback"),
            });
        encoder.copy_buffer_to_buffer(&setup.images.output, 0, &read, 0, size);
        gpu.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        let slice = read.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("{error:?}"))?;
        receive
            .recv()
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let pixels = {
            let bytes = slice.get_mapped_range().expect("a mapped range");
            let all: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
            let count = setup.width as usize * setup.height as usize;
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(count).map_err(|error| {
                format!(
                    "a {}x{} denoised frame needs {} bytes: {error}",
                    setup.width,
                    setup.height,
                    count as u128 * 16
                )
            })?;
            for row in all.chunks_exact(setup.stride as usize) {
                pixels.extend_from_slice(&row[..setup.width as usize]);
            }
            pixels
        };
        read.unmap();
        Ok(pixels)
    }

    fn prepare(&mut self, gpu: &Gpu, width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Err("image dimensions must be nonzero".into());
        }
        if self
            .setup
            .as_ref()
            .is_some_and(|setup| setup.width == width && setup.height == height)
        {
            return Ok(());
        }
        self.setup = None;
        self.setup = Some(setup::build(
            gpu,
            &self.models,
            self.max_tile_pixels,
            width,
            height,
        )?);
        Ok(())
    }
}

fn model(gpu: &Gpu, filter: Filter, aux: Aux) -> Result<Model, String> {
    let net = net::build(&tza::parse(filter.weights(aux))?, filter.input_channels())?;
    let upload = |label: &str, data: &[f32]| {
        gpu.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE,
            })
    };
    let weights = net
        .layers
        .iter()
        .map(|layer| upload(&layer.name, &layer.weights))
        .collect();
    let biases = net
        .layers
        .iter()
        .map(|layer| upload(&layer.name, &layer.bias))
        .collect();
    Ok(Model {
        filter,
        net,
        weights,
        biases,
    })
}

fn check_texture(texture: &wgpu::Texture, width: u32, height: u32) -> Result<(), String> {
    if texture.format() != wgpu::TextureFormat::Rgba32Float {
        return Err("the denoiser reads and writes Rgba32Float textures".into());
    }
    if texture.width() != width || texture.height() != height {
        return Err("image size mismatch".into());
    }
    Ok(())
}

fn image_layout(stride: u32) -> wgpu::TexelCopyBufferLayout {
    wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(stride * 16),
        rows_per_image: None,
    }
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

fn upload(gpu: &Gpu, buffer: &wgpu::Buffer, width: u32, stride: u32, pixels: &[[f32; 4]]) {
    if width == stride {
        gpu.queue
            .write_buffer(buffer, 0, bytemuck::cast_slice(pixels));
        return;
    }
    let mut padded = vec![[0.0f32; 4]; (buffer.size() / 16) as usize];
    for (row, line) in pixels.chunks_exact(width as usize).enumerate() {
        padded[row * stride as usize..][..width as usize].copy_from_slice(line);
    }
    gpu.queue
        .write_buffer(buffer, 0, bytemuck::cast_slice(&padded));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(x: u32, y: u32, salt: u32) -> f32 {
        let mut h = x.wrapping_mul(0x8da6_b343)
            ^ y.wrapping_mul(0xd816_3841)
            ^ salt.wrapping_mul(0xcb1a_b31f);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        h = h.wrapping_mul(0x297a_2d39);
        h ^= h >> 15;
        h as f32 / u32::MAX as f32
    }

    fn scene(size: u32) -> [Vec<[f32; 4]>; 3] {
        let mut color = Vec::new();
        let mut albedo = Vec::new();
        let mut normal = Vec::new();
        for y in 0..size {
            for x in 0..size {
                let cell = (x / 37 + y / 53) % 4;
                let a = [
                    [0.8, 0.2, 0.1],
                    [0.1, 0.6, 0.3],
                    [0.2, 0.3, 0.9],
                    [0.9, 0.9, 0.8],
                ][cell as usize];
                let n: [f32; 3] = match (x / 91 + 2 * (y / 71)) % 3 {
                    0 => [0.0, 0.0, 1.0],
                    1 => [0.6, 0.0, 0.8],
                    _ => [0.0, -0.8, 0.6],
                };
                let light = if (x + y) % 300 < 150 { 4.0 } else { 0.5 };
                let noise = |salt: u32| 2.0 * hash(x, y, salt);
                color.push([
                    a[0] * light * noise(0),
                    a[1] * light * noise(1),
                    a[2] * light * noise(2),
                    1.0,
                ]);
                let jitter = 0.2 * (hash(x, y, 3) - 0.5);
                albedo.push([
                    (a[0] + jitter).clamp(0.0, 1.0),
                    (a[1] + jitter).clamp(0.0, 1.0),
                    (a[2] + jitter).clamp(0.0, 1.0),
                    1.0,
                ]);
                normal.push([n[0] + jitter, n[1] - jitter, n[2], 0.0]);
            }
        }
        [color, albedo, normal]
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn tiled_denoising_matches_one_tile_in_every_mode() {
        let size = 1200;
        let [color, albedo, normal] = scene(size);
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        for aux in [Aux::Noisy, Aux::Prefiltered, Aux::Clean] {
            let mut whole = Denoiser::with_aux(&gpu, aux).unwrap();
            let mut split = Denoiser::with_aux(&gpu, aux).unwrap();
            split.max_tile_pixels = 768 * 768;
            let one = whole
                .denoise(&gpu, size, size, &color, &albedo, &normal)
                .unwrap();
            let tiled = split
                .denoise(&gpu, size, size, &color, &albedo, &normal)
                .unwrap();
            let whole_tiles = whole.tiles().unwrap().2;
            let split_tiles = split.tiles().unwrap().2;
            let mut worst = 0.0f32;
            for (a, b) in tiled.iter().zip(&one) {
                for c in 0..4 {
                    worst = worst.max((a[c] - b[c]).abs() / (1.0 + b[c].abs()));
                }
            }
            println!(
                "{aux:?} {size}x{size}: {split_tiles} tiles against {whole_tiles}, worst gap {worst:.2e}"
            );
            assert!(split_tiles >= 4, "{aux:?} {split_tiles}");
            assert!(whole_tiles < split_tiles, "{aux:?} {whole_tiles}");
            assert!(worst <= 1e-3, "{aux:?} {worst}");
        }
    }
}
