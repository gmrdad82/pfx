use pfx::adapter::{About, Adapter, HdrImage, Image, TurnState, serve, turn_soon};
use pfx::protocol::{AudioFormat, Event, Opened};
use pfx_gpu::pace::{Pacer, Slice, Stats, Work};
use pfx_gpu::{Gpu, wgpu};
use serde_json::Value;
use std::path::Path;
use std::sync::mpsc;

const SHOTS: &str = r#"
[shots.slide]
line = "A checkerboard slides one square per second."
fixtures = "none"
seconds = 1.0
fps = [60]
size = ["720p"]
closure = { crossfade = 0.2 }
warmup = 8
per_frame = 2

[[shots.slide.inputs]]
at = 0.5
cue = "flip"
"#;

const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> params: array<u32>;
@group(0) @binding(1) var<storage, read_write> pixels: array<vec2<u32>>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let width = params[0];
    let height = params[1];
    let start = params[2];
    let rows = params[3];
    if id.x >= width || id.y >= rows { return; }
    let y = start + id.y;
    if y >= height { return; }
    let square = bitcast<f32>(params[4]);
    let shift = bitcast<f32>(params[5]);
    let on = (u32(floor((f32(id.x) + shift) / square)) + u32(floor(f32(y) / square))) % 2u == 0u;
    let bright = select(on, !on, params[8] != 0u);
    let value = select(9000u, 52000u, bright);
    var hash = params[6] ^ (id.x * 73856093u) ^ (y * 19349663u);
    for (var i = 0u; i < 768u; i = i + 1u) {
        hash = (hash ^ (hash >> 13u)) * 1664525u + 1013904223u + i;
    }
    let jitter = (hash % 512u) / max(params[7], 1u);
    pixels[y * width + id.x] = vec2<u32>(
        (value + jitter) | (value << 16u),
        value | (65535u << 16u)
    );
}
"#;

struct Checker {
    width: u32,
    height: u32,
    time: f64,
    samples: u32,
    noise: u64,
    flipped: bool,
    gpu: Option<Gpu>,
    pacer: Pacer,
    turns: TurnState,
}

impl Checker {
    fn cpu_frame(&self) -> Vec<u16> {
        let square = 60.0 * self.width as f64 / 1920.0;
        let shift = self.time * square;
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height {
            for x in 0..self.width {
                let on = (((x as f64 + shift) / square).floor() as i64
                    + (y as f64 / square).floor() as i64)
                    % 2
                    == 0;
                let value = if on != self.flipped {
                    52000u16
                } else {
                    9000u16
                };
                let jitter = ((self.noise ^ (x as u64 * 73_856_093) ^ (y as u64 * 19_349_663))
                    % 512) as u16
                    / (self.samples.max(1) as u16);
                rgba.extend_from_slice(&[value.saturating_add(jitter), value, value, 65535]);
            }
        }
        rgba
    }

    fn render(&mut self, fixed_rows: Option<u32>) -> Result<(Vec<u16>, Stats), String> {
        let gpu = self.gpu.as_ref().ok_or("GPU is not open")?;
        let size = u64::from(self.width) * u64::from(self.height) * 8;
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("checker pixels"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("checker readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let params = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("checker params"),
            size: 36,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("checker heavy frame"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("checker heavy frame"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("checker frame"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let square = 60.0 * self.width as f32 / 1920.0;
        let shift = self.time as f32 * square;
        let base = [
            self.width,
            self.height,
            0,
            0,
            square.to_bits(),
            shift.to_bits(),
            self.noise as u32,
            self.samples,
            u32::from(self.flipped),
        ];
        self.pacer.set_fixed_units(fixed_rows);
        let stats = self.pacer.run(
            "checker frame",
            &gpu.device,
            &gpu.queue,
            Work::Bands {
                width: self.width,
                height: self.height,
            },
            |encoder, slice| {
                let Slice::Pixels { y, height, .. } = slice else {
                    unreachable!()
                };
                let mut data = base;
                data[2] = y;
                data[3] = height;
                let bytes: Vec<u8> = data.iter().flat_map(|value| value.to_le_bytes()).collect();
                gpu.queue.write_buffer(&params, 0, &bytes);
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("checker band"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(self.width.div_ceil(8), height.div_ceil(8), 1);
            },
            |milliseconds| {
                turn_soon(&mut self.turns, milliseconds);
            },
        )?;
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&output, 0, &read, 0, size);
        gpu.queue.submit(Some(encoder.finish()));
        let (send, receive) = mpsc::channel();
        read.slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("{error:?}"))?;
        receive
            .recv()
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let data = read.slice(..).get_mapped_range();
        let rgba = data
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        drop(data);
        read.unmap();
        Ok((rgba, stats))
    }
}

impl Adapter for Checker {
    fn about(&self) -> About {
        About {
            app: "checker".into(),
            rev: "example".into(),
            depth: 16,
            cutouts: false,
        }
    }

    fn shots(&self) -> &str {
        SHOTS
    }

    fn open(
        &mut self,
        _shot: &str,
        _fixtures: &Path,
        scale: f32,
        _app: &Value,
    ) -> Result<Opened, String> {
        self.width = (1920.0 * scale).round() as u32;
        self.height = (1080.0 * scale).round() as u32;
        if std::env::var("PFX_DIRECT").ok().as_deref() == Some("1") {
            return Ok(Opened {
                size: [self.width, self.height],
                gpu: "none".into(),
                driver: "none".into(),
                backend: None,
            });
        }
        let gpu = pollster::block_on(Gpu::headless())?;
        let opened = Opened {
            size: [self.width, self.height],
            gpu: gpu.info.name.clone(),
            driver: gpu.info.driver.clone(),
            backend: Some(format!("{:?}", gpu.info.backend)),
        };
        self.gpu = Some(gpu);
        Ok(opened)
    }

    fn input(&mut self, event: &Event) -> Result<(), String> {
        if *event == (Event::Cue { cue: "flip".into() }) {
            self.flipped = !self.flipped;
        }
        Ok(())
    }

    fn advance(&mut self, to: f64) -> Result<(), String> {
        self.time = to;
        self.samples = 0;
        Ok(())
    }

    fn sample(&mut self, spp: u32, _moving: u32, seed: u64) -> Result<(), String> {
        self.samples += spp;
        self.noise = seed;
        Ok(())
    }

    fn frame(&mut self, _ground: Option<[f32; 3]>, _linear: bool) -> Result<Image, String> {
        let rgba = if self.gpu.is_some() {
            self.render(None)?.0
        } else {
            self.cpu_frame()
        };
        self.turns.reset();
        Ok(Image {
            width: self.width,
            height: self.height,
            rgba,
            ground: None,
        })
    }

    fn audio_format(&self) -> Option<AudioFormat> {
        match std::env::var("CHECKER_AUDIO").ok().as_deref() {
            Some("f32") => Some(AudioFormat::F32),
            Some("s16") => Some(AudioFormat::S16),
            _ => None,
        }
    }

    fn audio(&mut self, seconds: f64) -> Result<Vec<f32>, String> {
        let frames = (seconds * 48_000.0).round() as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f64 / 48_000.0;
            samples.push((0.5 * (std::f64::consts::TAU * 440.0 * t).sin()) as f32);
            samples.push((0.5 * (std::f64::consts::TAU * 660.0 * t).sin()) as f32);
        }
        Ok(samples)
    }

    fn supports_hdr(&self) -> bool {
        true
    }

    fn hdr(&mut self) -> Result<HdrImage, String> {
        let ramp = [0x0000, 0x3400, 0x3800, 0x3c00];
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height {
            for x in 0..self.width {
                rgba.extend_from_slice(&[
                    ramp[(x % 4) as usize],
                    ramp[(y % 4) as usize],
                    0x3c00,
                    0x3c00,
                ]);
            }
        }
        Ok(HdrImage {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}

fn main() {
    let mut adapter = Checker {
        width: 0,
        height: 0,
        time: 0.0,
        samples: 0,
        noise: 0,
        flipped: false,
        gpu: None,
        pacer: Pacer::default(),
        turns: TurnState::default(),
    };
    if let Err(error) = serve(&mut adapter) {
        eprintln!("checker: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdr_gradient_is_half_float_in_top_down_row_order() {
        let mut checker = Checker {
            width: 4,
            height: 3,
            time: 0.0,
            samples: 0,
            noise: 0,
            flipped: false,
            gpu: None,
            pacer: Pacer::default(),
            turns: TurnState::default(),
        };
        assert!(checker.supports_hdr());
        let image = checker.hdr().unwrap();
        assert_eq!([image.width, image.height], [4, 3]);
        assert_eq!(&image.rgba[0..4], &[0x0000, 0x0000, 0x3c00, 0x3c00]);
        assert_eq!(&image.rgba[12..16], &[0x3c00, 0x0000, 0x3c00, 0x3c00]);
        assert_eq!(&image.rgba[16..20], &[0x0000, 0x3400, 0x3c00, 0x3c00]);
        assert_eq!(&image.rgba[32..36], &[0x0000, 0x3800, 0x3c00, 0x3c00]);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn heavy_frame_is_identical_at_every_band_size() {
        let mut checker = Checker {
            width: 3840,
            height: 2160,
            time: 0.5,
            samples: 4,
            noise: 37,
            flipped: false,
            gpu: Some(pollster::block_on(Gpu::headless()).unwrap()),
            pacer: Pacer::default(),
            turns: TurnState::default(),
        };
        let (reference, unsliced) = checker.render(Some(2160)).unwrap();
        for rows in [Some(1), Some(7), Some(64), None] {
            let (frame, stats) = checker.render(rows).unwrap();
            assert_eq!(frame, reference);
            if rows.is_none() {
                let pixels = f64::from(checker.width) * f64::from(checker.height);
                let unsliced_throughput = pixels / (unsliced.wall_ms * 1000.0);
                let paced_throughput = pixels / (stats.wall_ms * 1000.0);
                eprintln!(
                    "checker slices={} longest_ms={:.3} median_ms={:.3} unsliced_mpix_s={:.3} paced_mpix_s={:.3} throughput_ratio={:.3}",
                    stats.count(),
                    stats.longest_ms(),
                    stats.median_ms(),
                    unsliced_throughput,
                    paced_throughput,
                    paced_throughput / unsliced_throughput,
                );
                assert!(
                    stats.longest_ms() <= 6.0,
                    "longest slice: {} ms",
                    stats.longest_ms()
                );
            }
        }
    }
}
