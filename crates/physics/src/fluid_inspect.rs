use super::{Fluid, FluidState, Particle};
use std::collections::BTreeMap;

impl Fluid {
    pub fn inspect(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> FluidState {
        self.inspect_with_row(device, queue, None)
    }

    pub fn inspect_row(&self, device: &wgpu::Device, queue: &wgpu::Queue, row: u32) -> FluidState {
        self.inspect_with_row(device, queue, Some(row))
    }

    fn inspect_with_row(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        row: Option<u32>,
    ) -> FluidState {
        let particle_bytes = self.high as u64 * std::mem::size_of::<Particle>() as u64;
        let padded = (self.width * 8).div_ceil(256) * 256;
        let height_bytes = padded as u64 * self.height as u64;
        let particles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fluid particle inspection"),
            size: particle_bytes.max(4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let height = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fluid height inspection"),
            size: height_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Fluid inspection"),
        });
        if particle_bytes > 0 {
            encoder.copy_buffer_to_buffer(&self.particles, 0, &particles, 0, particle_bytes);
        }
        encoder.copy_texture_to_buffer(
            self.height_texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &height,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.height),
                },
            },
            self.height_texture.size(),
        );
        queue.submit(Some(encoder.finish()));
        let particle_data = if particle_bytes > 0 {
            mapped(device, &particles)
        } else {
            Vec::new()
        };
        let height_data = mapped(device, &height);
        let mut state = FluidState {
            live: 0,
            asleep: self.asleep(),
            bounds: None,
            height_min: f32::INFINITY,
            height_max: f32::NEG_INFINITY,
            height_mean: 0.0,
            channel_histogram: std::array::from_fn(|_| BTreeMap::new()),
            resting: self.still > self.settle,
            row: row
                .filter(|r| *r < self.height)
                .map(|_| Vec::with_capacity(self.width as usize)),
        };
        for particle in bytemuck::cast_slice::<u8, Particle>(&particle_data) {
            if particle.born < 0.0 || !particle.pos.iter().all(|v| v.is_finite()) {
                continue;
            }
            state.live += 1;
            match &mut state.bounds {
                Some((lo, hi)) => {
                    for axis in 0..2 {
                        lo[axis] = lo[axis].min(particle.pos[axis]);
                        hi[axis] = hi[axis].max(particle.pos[axis]);
                    }
                }
                None => state.bounds = Some((particle.pos, particle.pos)),
            }
        }
        let mut sum = 0.0f64;
        for y in 0..self.height as usize {
            for x in 0..self.width as usize {
                let at = y * padded as usize + x * 8;
                let channels = std::array::from_fn::<_, 4, _>(|channel| {
                    half(u16::from_le_bytes([
                        height_data[at + channel * 2],
                        height_data[at + channel * 2 + 1],
                    ]))
                });
                let h = channels[0];
                state.height_min = state.height_min.min(h);
                state.height_max = state.height_max.max(h);
                sum += f64::from(h);
                if row == Some(y as u32)
                    && let Some(samples) = &mut state.row
                {
                    samples.push(h);
                }
                if h > 0.01 {
                    for channel in 0..3 {
                        let value = (channels[channel + 1] / h * 16.0).round() as i32;
                        *state.channel_histogram[channel].entry(value).or_default() += 1;
                    }
                }
            }
        }
        state.height_mean = (sum / f64::from(self.width * self.height)) as f32;
        state
    }
}

pub(super) fn mapped(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    let slice = buffer.slice(..);
    let (send, recv) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("fluid inspection poll");
    recv.recv()
        .expect("fluid inspection callback")
        .expect("fluid inspection map");
    slice.get_mapped_range().to_vec()
}

fn half(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if man == 0 {
            sign << 31
        } else {
            let mut frac = man;
            let mut exponent = 113u32;
            while frac & 0x400 == 0 {
                frac <<= 1;
                exponent -= 1;
            }
            frac &= 0x3ff;
            (sign << 31) | (exponent << 23) | (frac << 13)
        }
    } else if exp == 31 {
        (sign << 31) | (255 << 23) | (man << 13)
    } else {
        (sign << 31) | ((exp + 112) << 23) | (man << 13)
    };
    f32::from_bits(bits)
}
