use std::io::{Read, Write};

use pfx_gpu::Gpu;

use crate::adaptive::{Round, mapped};
use crate::gpu::Trace;

const MAGIC: &[u8; 8] = b"PFXCKPT1";
const ACCUM: u64 = 48;
const TEXEL: u64 = 16;
const SNAP: u64 = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub size: [u32; 2],
    pub samples: u32,
    pub accum: Vec<u8>,
    pub color: Vec<u8>,
    pub albedo: Vec<u8>,
    pub normal: Vec<u8>,
    pub adaptive: Option<Rounds>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rounds {
    pub reached: Vec<Round>,
    pub snaps: Vec<u8>,
}

pub trait Keeper {
    fn resume(&mut self) -> Option<Saved>;
    fn due(&mut self, samples: u32) -> bool;
    fn keep(&mut self, saved: Saved) -> Result<(), String>;
}

fn pixels(size: [u32; 2]) -> u64 {
    u64::from(size[0]) * u64::from(size[1])
}

fn put(out: &mut impl Write, bytes: &[u8]) -> Result<(), String> {
    out.write_all(bytes).map_err(|e| e.to_string())
}

fn word(input: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    input
        .read_exact(&mut bytes)
        .map_err(|e| format!("the checkpoint is cut short: {e}"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn long(input: &mut impl Read) -> Result<u64, String> {
    let mut bytes = [0u8; 8];
    input
        .read_exact(&mut bytes)
        .map_err(|e| format!("the checkpoint is cut short: {e}"))?;
    Ok(u64::from_le_bytes(bytes))
}

fn blob(input: &mut impl Read, len: u64) -> Result<Vec<u8>, String> {
    let len = usize::try_from(len).map_err(|_| "the checkpoint is too large".to_string())?;
    let mut out = pfx_core::memory::filled(len, 0u8, "a checkpoint")?;
    input
        .read_exact(&mut out)
        .map_err(|e| format!("the checkpoint is cut short: {e}"))?;
    Ok(out)
}

impl Saved {
    pub fn fits(&self) -> bool {
        let pixels = pixels(self.size);
        self.accum.len() as u64 == pixels * ACCUM
            && [&self.color, &self.albedo, &self.normal]
                .iter()
                .all(|texels| texels.len() as u64 == pixels * TEXEL)
            && self
                .adaptive
                .as_ref()
                .is_none_or(|rounds| rounds.snaps.len() as u64 == pixels * SNAP)
    }

    pub fn write_to(&self, key: &str, out: &mut impl Write) -> Result<(), String> {
        if !self.fits() {
            return Err("the checkpoint's buffers do not match its size".into());
        }
        put(out, MAGIC)?;
        put(out, &(key.len() as u64).to_le_bytes())?;
        put(out, key.as_bytes())?;
        for value in [self.size[0], self.size[1], self.samples] {
            put(out, &value.to_le_bytes())?;
        }
        for texels in [&self.accum, &self.color, &self.albedo, &self.normal] {
            put(out, texels)?;
        }
        match &self.adaptive {
            None => put(out, &u32::MAX.to_le_bytes()),
            Some(rounds) => {
                put(out, &(rounds.reached.len() as u32).to_le_bytes())?;
                for round in &rounds.reached {
                    put(out, &round.samples.to_le_bytes())?;
                    put(out, &round.active.to_le_bytes())?;
                }
                put(out, &rounds.snaps)
            }
        }
    }

    pub fn read_from(key: &str, input: &mut impl Read) -> Result<Saved, String> {
        let mut magic = [0u8; 8];
        input
            .read_exact(&mut magic)
            .map_err(|e| format!("the checkpoint is cut short: {e}"))?;
        if &magic != MAGIC {
            return Err("not a pfx checkpoint".into());
        }
        let len = long(input)?;
        if len != key.len() as u64 {
            return Err("the checkpoint was made by another render".into());
        }
        if blob(input, len)? != key.as_bytes() {
            return Err("the checkpoint was made by another render".into());
        }
        let size = [word(input)?, word(input)?];
        let samples = word(input)?;
        let pixels = pixels(size);
        let accum = blob(input, pixels * ACCUM)?;
        let color = blob(input, pixels * TEXEL)?;
        let albedo = blob(input, pixels * TEXEL)?;
        let normal = blob(input, pixels * TEXEL)?;
        let adaptive = match word(input)? {
            u32::MAX => None,
            count => {
                let mut reached = Vec::new();
                for _ in 0..count {
                    reached.push(Round {
                        samples: word(input)?,
                        active: long(input)?,
                    });
                }
                Some(Rounds {
                    reached,
                    snaps: blob(input, pixels * SNAP)?,
                })
            }
        };
        let mut rest = [0u8; 1];
        if input.read(&mut rest).map_err(|e| e.to_string())? != 0 {
            return Err("the checkpoint runs on past its end".into());
        }
        Ok(Saved {
            size,
            samples,
            accum,
            color,
            albedo,
            normal,
            adaptive,
        })
    }
}

impl Trace {
    pub fn save(&self, gpu: &Gpu) -> Result<Saved, String> {
        Ok(Saved {
            size: [self.width, self.height],
            samples: self.samples,
            accum: read_buffer(gpu, &self.accum)?,
            color: gpu.readback_bytes(&self.color)?,
            albedo: gpu.readback_bytes(&self.albedo)?,
            normal: gpu.readback_bytes(&self.normal)?,
            adaptive: None,
        })
    }

    pub fn restore(&mut self, gpu: &Gpu, saved: &Saved) -> Result<(), String> {
        if saved.size != [self.width, self.height] || !saved.fits() {
            return Err(format!(
                "the checkpoint is {}x{} and the trace {}x{}",
                saved.size[0], saved.size[1], self.width, self.height
            ));
        }
        gpu.queue.write_buffer(&self.accum, 0, &saved.accum);
        for (target, texels) in [
            (&self.color, &saved.color),
            (&self.albedo, &saved.albedo),
            (&self.normal, &saved.normal),
        ] {
            gpu.queue.write_texture(
                target.texture.as_image_copy(),
                texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * TEXEL as u32),
                    rows_per_image: Some(self.height),
                },
                target.texture.size(),
            );
        }
        gpu.queue.submit(None);
        self.samples = saved.samples;
        Ok(())
    }
}

pub(crate) fn read_buffer(gpu: &Gpu, buffer: &wgpu::Buffer) -> Result<Vec<u8>, String> {
    let size = buffer.size();
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("trace checkpoint readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("trace checkpoint readback"),
        });
    encoder.copy_buffer_to_buffer(buffer, 0, &read, 0, size);
    gpu.queue.submit(Some(encoder.finish()));
    mapped(gpu, &read)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(adaptive: bool) -> Saved {
        let size = [3, 2];
        let fill = |per: u64, seed: u8| -> Vec<u8> {
            (0..pixels(size) * per)
                .map(|n| (n as u8).wrapping_mul(7).wrapping_add(seed))
                .collect()
        };
        Saved {
            size,
            samples: 21,
            accum: fill(ACCUM, 1),
            color: fill(TEXEL, 2),
            albedo: fill(TEXEL, 3),
            normal: fill(TEXEL, 4),
            adaptive: adaptive.then(|| Rounds {
                reached: vec![
                    Round {
                        samples: 8,
                        active: 6,
                    },
                    Round {
                        samples: 16,
                        active: 2,
                    },
                ],
                snaps: fill(SNAP, 5),
            }),
        }
    }

    #[test]
    fn a_checkpoint_reads_back_as_written_only_under_its_own_key() {
        for adaptive in [false, true] {
            let saved = saved(adaptive);
            let mut bytes = Vec::new();
            saved.write_to("clip frame 4", &mut bytes).unwrap();
            assert_eq!(
                Saved::read_from("clip frame 4", &mut bytes.as_slice()).unwrap(),
                saved
            );
            let other = Saved::read_from("clip frame 5", &mut bytes.as_slice()).unwrap_err();
            assert!(other.contains("another render"), "{other}");
            let cut = Saved::read_from("clip frame 4", &mut &bytes[..bytes.len() - 1]);
            assert!(cut.unwrap_err().contains("cut short"));
            let mut long = bytes.clone();
            long.push(0);
            assert!(Saved::read_from("clip frame 4", &mut long.as_slice()).is_err());
        }
        let mut wrong = saved(false);
        wrong.color.pop();
        assert!(wrong.write_to("k", &mut Vec::new()).is_err());
        assert!(Saved::read_from("k", &mut &b"PNG....."[..]).is_err());
    }
}
