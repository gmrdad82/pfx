use bytemuck::{Pod, Zeroable};
use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Stats, Turned};
use wgpu::util::DeviceExt;

use crate::gpu::Trace;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adaptive {
    pub threshold: f32,
    pub min_samples: u32,
    pub max_samples: u32,
    pub growth: f32,
}

impl Default for Adaptive {
    fn default() -> Self {
        Self {
            threshold: 0.005,
            min_samples: 32,
            max_samples: 1024,
            growth: 1.5,
        }
    }
}

impl Adaptive {
    pub fn valid(&self) -> bool {
        self.threshold.is_finite()
            && self.threshold > 0.0
            && self.min_samples >= 2
            && self.max_samples >= self.min_samples
            && self.growth.is_finite()
            && self.growth > 1.0
    }

    pub fn rounds(&self) -> Vec<u32> {
        let mut rounds = vec![(self.min_samples / 2).max(1)];
        loop {
            let last = *rounds.last().unwrap();
            if last >= self.max_samples {
                return rounds;
            }
            let next = ((last as f32 * self.growth).ceil() as u32)
                .max(last + 1)
                .max(if rounds.len() == 1 {
                    self.min_samples
                } else {
                    0
                })
                .min(self.max_samples);
            rounds.push(next);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Converged {
    pub rounds: Vec<Round>,
    pub pixels: u64,
    pub stats: Stats,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Round {
    pub samples: u32,
    pub active: u64,
}

impl Converged {
    pub fn max_samples(&self) -> u32 {
        self.rounds.last().map_or(0, |round| round.samples)
    }

    pub fn mean_samples(&self) -> f64 {
        if self.pixels == 0 {
            return 0.0;
        }
        let mut total = 0.0;
        let mut before = 0;
        let mut running = self.pixels;
        for round in &self.rounds {
            total += f64::from(round.samples - before) * running as f64;
            before = round.samples;
            running = round.active;
        }
        total / self.pixels as f64
    }

    pub fn converged(&self) -> bool {
        self.rounds.last().is_some_and(|round| round.active == 0)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Check {
    size: [u32; 4],
    limits: [f32; 4],
}

impl Trace {
    pub fn sample_adaptive<T: Turned>(
        &mut self,
        gpu: &Gpu,
        adaptive: Adaptive,
        seed: u32,
        pacer: &mut Pacer,
        mut between: impl FnMut(f64) -> T,
    ) -> Result<Converged, String> {
        if !adaptive.valid() {
            return Err("adaptive sampling needs a positive threshold, a growth over 1 and at least 2 samples".into());
        }
        self.samples = 0;
        let pixels = u64::from(self.width) * u64::from(self.height);
        let check = Check {
            size: [self.width, self.height, adaptive.min_samples, 0],
            limits: [adaptive.threshold, 0.0, 0.0, 0.0],
        };
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("trace convergence"),
                source: wgpu::ShaderSource::Wgsl(include_str!("converge.wgsl").into()),
            });
        let pipeline = |entry: &'static str| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: None,
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let measure = pipeline("measure");
        let mark = pipeline("mark");
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("trace convergence check"),
                contents: bytemuck::bytes_of(&check),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let snaps = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace convergence snapshots"),
            size: pixels * 32,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let active = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace convergence active"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace convergence readback"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bind = |pipeline: &wgpu::ComputePipeline, with_active: bool| {
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.accum.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: snaps.as_entire_binding(),
                },
            ];
            if with_active {
                entries.push(wgpu::BindGroupEntry {
                    binding: 3,
                    resource: active.as_entire_binding(),
                });
            }
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("trace convergence bindings"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            })
        };
        let measure_binding = bind(&measure, false);
        let mark_binding = bind(&mark, true);
        let groups = (self.width.div_ceil(8), self.height.div_ceil(8));
        let mut converged = Converged {
            pixels,
            ..Converged::default()
        };
        for (step, target) in adaptive.rounds().into_iter().enumerate() {
            let count = target - self.samples;
            let stats = self.sample_paced(gpu, count, seed, pacer, &mut between)?;
            converged.stats.milliseconds.extend(stats.milliseconds);
            converged.stats.timings.extend(stats.timings);
            converged.stats.wall_ms += stats.wall_ms;
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("trace convergence"),
                });
            encoder.clear_buffer(&active, 0, None);
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("trace convergence measure"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&measure);
                pass.set_bind_group(0, &measure_binding, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
            if step > 0 {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("trace convergence mark"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&mark);
                pass.set_bind_group(0, &mark_binding, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
            encoder.copy_buffer_to_buffer(&active, 0, &read, 0, 16);
            gpu.queue.submit(Some(encoder.finish()));
            let running = if step > 0 {
                u64::from(read_count(gpu, &read)?)
            } else {
                pixels
            };
            converged.rounds.push(Round {
                samples: target,
                active: running,
            });
            if running == 0 {
                break;
            }
        }
        Ok(converged)
    }
}

impl Trace {
    pub fn sample_counts(&self, gpu: &Gpu) -> Result<Vec<u32>, String> {
        let size = self.accum.size();
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace sample counts"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("trace sample counts"),
            });
        encoder.copy_buffer_to_buffer(&self.accum, 0, &read, 0, size);
        gpu.queue.submit(Some(encoder.finish()));
        let bytes = mapped(gpu, &read)?;
        Ok(bytes
            .chunks_exact(48)
            .map(|accum| f32::from_le_bytes(accum[12..16].try_into().unwrap()).abs() as u32)
            .collect())
    }
}

fn mapped(gpu: &Gpu, buffer: &wgpu::Buffer) -> Result<Vec<u8>, String> {
    let (send, receive) = std::sync::mpsc::channel();
    let slice = buffer.slice(..);
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
    let bytes = slice.get_mapped_range().to_vec();
    buffer.unmap();
    Ok(bytes)
}

fn read_count(gpu: &Gpu, buffer: &wgpu::Buffer) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        mapped(gpu, buffer)?[..4].try_into().unwrap(),
    ))
}

pub fn error(prior: [f32; 4], prior_count: f32, now: [f32; 4], count: f32) -> f32 {
    if prior_count <= 0.0 || count <= prior_count {
        return 1e9;
    }
    let scale = (1.0 / count).sqrt() / (1.0 / prior_count + 1.0 / (count - prior_count)).sqrt();
    let gap: [f32; 4] = std::array::from_fn(|k| {
        let rest = (now[k] * count - prior[k] * prior_count) / (count - prior_count);
        (prior[k] - rest).abs() * scale
    });
    (gap[0] + gap[1] + gap[2]) / (1e-4 + (now[0] + now[1] + now[2]).max(0.0).sqrt()) + gap[3]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_grow_to_the_cap() {
        let adaptive = Adaptive {
            threshold: 0.01,
            min_samples: 16,
            max_samples: 100,
            growth: 1.5,
        };
        assert_eq!(adaptive.rounds(), vec![8, 16, 24, 36, 54, 81, 100]);
        assert!(adaptive.valid());
        assert!(
            !Adaptive {
                growth: 1.0,
                ..adaptive
            }
            .valid()
        );
        assert!(
            !Adaptive {
                max_samples: 8,
                ..adaptive
            }
            .valid()
        );
    }

    #[test]
    fn mean_samples_count_the_pixels_still_running() {
        let converged = Converged {
            rounds: vec![
                Round {
                    samples: 8,
                    active: 100,
                },
                Round {
                    samples: 16,
                    active: 40,
                },
                Round {
                    samples: 32,
                    active: 0,
                },
            ],
            pixels: 100,
            stats: Stats::default(),
        };
        assert!((converged.mean_samples() - (16.0 + 0.4 * 16.0)).abs() < 1e-9);
        assert!(converged.converged());
        assert_eq!(converged.max_samples(), 32);
    }

    #[test]
    fn the_error_is_cycles_half_buffer_at_a_half_split() {
        let even = [0.5, 0.4, 0.3, 1.0];
        let odd = [0.52, 0.38, 0.33, 1.0];
        let all: [f32; 4] = std::array::from_fn(|k| (even[k] + odd[k]) * 0.5);
        let got = error(even, 64.0, all, 128.0);
        let difference = (0..3).map(|k| (even[k] - odd[k]).abs()).sum::<f32>() * 0.5;
        let want = difference / (1e-4 + (all[0] + all[1] + all[2]).sqrt());
        assert!((got - want).abs() < 1e-6, "{got} against {want}");
        assert_eq!(error(even, 0.0, all, 8.0), 1e9);
        assert_eq!(error([0.2; 4], 10.0, [0.2; 4], 30.0), 0.0);
    }

    #[test]
    fn convergence_shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("converge.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
