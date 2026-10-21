use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::{FAILED, IDLE, MAPPING, READY};

pub const AUDIT_VAR: &str = "PFX_PRESENT_AUDIT";
pub const AUDIT_SLOTS: usize = 6;
pub const BLACK_LISTED: usize = 64;

pub const WRITE_EVERY: u64 = 120;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    frame: u64,
    held: Option<u64>,
    drawn: bool,
    pub acquired: u64,
    pub presents: u64,
    pub unrendered: u64,
    pub dropped: u64,
    pub reconfigured: u64,
}

impl Ledger {
    pub fn acquired(&mut self) -> u64 {
        if self.held.is_some() {
            self.dropped += 1;
        }
        self.frame += 1;
        self.acquired += 1;
        self.held = Some(self.frame);
        self.drawn = false;
        self.frame
    }

    pub fn drawn(&mut self) {
        if self.held.is_some() {
            self.drawn = true;
        }
    }

    pub fn presenting(&mut self) -> Option<(u64, bool)> {
        let frame = self.held.take()?;
        let rendered = std::mem::take(&mut self.drawn);
        self.presents += 1;
        if !rendered {
            self.unrendered += 1;
        }
        Some((frame, rendered))
    }

    pub fn reconfigured(&mut self) {
        self.reconfigured += 1;
        if self.held.take().is_some() {
            self.dropped += 1;
        }
        self.drawn = false;
    }

    pub fn holding(&self) -> Option<u64> {
        self.held
    }
}

pub fn black(bytes: &[u8], format: wgpu::TextureFormat) -> Option<bool> {
    match format {
        wgpu::TextureFormat::Bgra8Unorm
        | wgpu::TextureFormat::Bgra8UnormSrgb
        | wgpu::TextureFormat::Rgba8Unorm
        | wgpu::TextureFormat::Rgba8UnormSrgb => Some(
            bytes
                .chunks_exact(4)
                .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0),
        ),
        wgpu::TextureFormat::Rgb10a2Unorm => Some(bytes.chunks_exact(4).all(|pixel| {
            u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]) & 0x3fff_ffff == 0
        })),
        wgpu::TextureFormat::Rgba16Float => Some(bytes.chunks_exact(8).all(|pixel| {
            pixel[..6]
                .chunks_exact(2)
                .all(|half| u16::from_le_bytes([half[0], half[1]]) & 0x7fff == 0)
        })),
        _ => None,
    }
}

pub fn texel_bytes(format: wgpu::TextureFormat) -> Option<u32> {
    black(&[], format).map(|_| {
        if format == wgpu::TextureFormat::Rgba16Float {
            8
        } else {
            4
        }
    })
}

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    frame: u64,
    rendered: bool,
}

pub struct Readback {
    device: wgpu::Device,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    row: u32,
    slots: Vec<Slot>,
    pub checked: u64,
    pub black: u64,
    pub black_unrendered: u64,
    pub skipped: u64,
    pub failed: u64,
    pub black_frames: Vec<u64>,
}

impl Readback {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Option<Self> {
        texel_bytes(format)?;
        Some(Self {
            device: device.clone(),
            format,
            size: [0, 0],
            row: 0,
            slots: Vec::new(),
            checked: 0,
            black: 0,
            black_unrendered: 0,
            skipped: 0,
            failed: 0,
            black_frames: Vec::new(),
        })
    }

    fn fit(&mut self, size: [u32; 2]) {
        if self.size == size && !self.slots.is_empty() {
            return;
        }
        if self
            .slots
            .iter()
            .any(|slot| slot.state.load(Ordering::Acquire) == MAPPING)
        {
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            self.collect();
        }
        let texel = texel_bytes(self.format).unwrap_or(4);
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        self.row = (size[0] * texel).div_ceil(alignment) * alignment;
        self.size = size;
        let bytes = u64::from(self.row) * u64::from(size[1]);
        self.slots = (0..AUDIT_SLOTS)
            .map(|_| Slot {
                buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pfx present audit"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(IDLE)),
                frame: 0,
                rendered: true,
            })
            .collect();
    }

    pub fn capture(
        &mut self,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        frame: u64,
        rendered: bool,
    ) -> bool {
        if texture.format() != self.format {
            self.format = texture.format();
            self.slots.clear();
            if texel_bytes(self.format).is_none() {
                self.skipped += 1;
                return false;
            }
        }
        self.fit([texture.width(), texture.height()]);
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.state.load(Ordering::Acquire) == IDLE)
        else {
            self.skipped += 1;
            return false;
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx present audit"),
            });
        let slot = &mut self.slots[index];
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.row),
                    rows_per_image: Some(self.size[1]),
                },
            },
            wgpu::Extent3d {
                width: self.size[0],
                height: self.size[1],
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        slot.frame = frame;
        slot.rendered = rendered;
        slot.state.store(MAPPING, Ordering::Release);
        let state = slot.state.clone();
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(
                    if result.is_ok() { READY } else { FAILED },
                    Ordering::Release,
                );
            });
        true
    }

    pub fn collect(&mut self) {
        let _ = self.device.poll(wgpu::PollType::Poll);
        let raw = self.size[0] as usize * texel_bytes(self.format).unwrap_or(4) as usize;
        for slot in &mut self.slots {
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    let dark = {
                        let Ok(mapped) = slot.buffer.slice(..).get_mapped_range() else {
                            self.failed += 1;
                            slot.state.store(IDLE, Ordering::Release);
                            continue;
                        };
                        mapped
                            .chunks_exact(self.row as usize)
                            .all(|row| black(&row[..raw], self.format) == Some(true))
                    };
                    slot.buffer.unmap();
                    self.checked += 1;
                    if dark {
                        self.black += 1;
                        if !slot.rendered {
                            self.black_unrendered += 1;
                        }
                        if self.black_frames.len() < BLACK_LISTED {
                            self.black_frames.push(slot.frame);
                        }
                    }
                    slot.state.store(IDLE, Ordering::Release);
                }
                FAILED => {
                    self.failed += 1;
                    slot.buffer.unmap();
                    slot.state.store(IDLE, Ordering::Release);
                }
                _ => {}
            }
        }
    }

    pub fn finish(&mut self) {
        if self
            .slots
            .iter()
            .any(|slot| slot.state.load(Ordering::Acquire) == MAPPING)
        {
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        }
        self.collect();
    }
}

pub struct Audit {
    path: Option<PathBuf>,
    pub ledger: Ledger,
    pub readback: Option<Readback>,
    written: Option<u64>,
}

impl Audit {
    pub fn from_env(device: &wgpu::Device, format: wgpu::TextureFormat) -> Option<Self> {
        let path = std::env::var_os(AUDIT_VAR).filter(|path| !path.is_empty())?;
        Some(Self::to_file(Path::new(&path), device, format))
    }

    pub fn to_file(path: &Path, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            path: Some(path.to_path_buf()),
            ledger: Ledger::default(),
            readback: Readback::new(device, format),
            written: None,
        }
    }

    pub fn wanted() -> bool {
        std::env::var_os(AUDIT_VAR).is_some_and(|path| !path.is_empty())
    }

    pub fn presenting(&mut self, queue: &wgpu::Queue, texture: &wgpu::Texture) {
        let Some((frame, rendered)) = self.ledger.presenting() else {
            return;
        };
        if let Some(readback) = &mut self.readback {
            readback.capture(queue, texture, frame, rendered);
        }
    }

    pub fn presented(&mut self) {
        if let Some(readback) = &mut self.readback {
            readback.collect();
        }
        let presents = self.ledger.presents;
        let due = self
            .written
            .is_none_or(|written| presents.saturating_sub(written) >= WRITE_EVERY);
        if due {
            self.written = Some(presents);
            self.write();
        }
    }

    pub fn line(&self) -> String {
        let ledger = &self.ledger;
        let mut out = format!(
            "{{\"acquired\":{},\"presents\":{},\"unrendered\":{},\"dropped\":{},\"reconfigured\":{}",
            ledger.acquired,
            ledger.presents,
            ledger.unrendered,
            ledger.dropped,
            ledger.reconfigured
        );
        match &self.readback {
            Some(readback) => {
                let frames = readback
                    .black_frames
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let _ = write!(
                    out,
                    ",\"readback\":true,\"checked\":{},\"black\":{},\"black_unrendered\":{},\"skipped\":{},\"failed\":{},\"black_frames\":[{frames}]}}",
                    readback.checked,
                    readback.black,
                    readback.black_unrendered,
                    readback.skipped,
                    readback.failed
                );
            }
            None => out.push_str(",\"readback\":false}"),
        }
        out.push('\n');
        out
    }

    pub fn finish(&mut self) {
        if let Some(readback) = &mut self.readback {
            readback.finish();
        }
        self.write();
    }

    fn write(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut part = path.clone().into_os_string();
        part.push(".part");
        let part = PathBuf::from(part);
        let written =
            std::fs::write(&part, self.line()).and_then(|()| std::fs::rename(&part, path));
        if let Err(error) = written {
            eprintln!("{AUDIT_VAR}: cannot write {}: {error}", path.display());
        }
    }
}

impl Drop for Audit {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_drawn_after_its_acquire_presents_as_rendered() {
        let mut ledger = Ledger::default();
        let first = ledger.acquired();
        ledger.drawn();
        assert_eq!(ledger.presenting(), Some((first, true)));
        let second = ledger.acquired();
        assert_eq!(second, first + 1);
        ledger.drawn();
        assert_eq!(ledger.presenting(), Some((second, true)));
        assert_eq!(
            (ledger.presents, ledger.unrendered, ledger.dropped),
            (2, 0, 0)
        );
    }

    #[test]
    fn a_draw_before_the_acquire_does_not_count_for_the_image_acquired_after_it() {
        let mut ledger = Ledger::default();
        ledger.drawn();
        let frame = ledger.acquired();
        assert_eq!(ledger.presenting(), Some((frame, false)));
        assert_eq!(ledger.unrendered, 1);
        let next = ledger.acquired();
        ledger.drawn();
        ledger.presenting();
        ledger.drawn();
        assert_eq!(ledger.holding(), None);
        let last = ledger.acquired();
        assert!(last > next);
        assert_eq!(ledger.presenting(), Some((last, false)));
        assert_eq!((ledger.presents, ledger.unrendered), (3, 2));
    }

    #[test]
    fn a_present_with_nothing_acquired_is_not_a_present() {
        let mut ledger = Ledger::default();
        assert_eq!(ledger.presenting(), None);
        let frame = ledger.acquired();
        ledger.drawn();
        ledger.presenting();
        assert_eq!(ledger.presenting(), None);
        assert_eq!((ledger.presents, ledger.acquired, frame), (1, 1, 1));
    }

    #[test]
    fn an_image_acquired_and_never_presented_counts_as_dropped_on_the_next_acquire_or_a_reconfigure()
     {
        let mut ledger = Ledger::default();
        ledger.acquired();
        ledger.drawn();
        let next = ledger.acquired();
        assert_eq!(ledger.dropped, 1);
        assert_eq!(ledger.presenting(), Some((next, false)));
        ledger.acquired();
        ledger.drawn();
        ledger.reconfigured();
        assert_eq!((ledger.dropped, ledger.reconfigured), (2, 1));
        assert_eq!(ledger.presenting(), None);
        let after = ledger.acquired();
        ledger.drawn();
        assert_eq!(ledger.presenting(), Some((after, true)));
        ledger.reconfigured();
        assert_eq!(ledger.dropped, 2);
    }

    #[test]
    fn skips_and_resizes_never_leave_a_present_unrendered_when_each_frame_draws_after_acquire() {
        let mut ledger = Ledger::default();
        for frame in 0..600u64 {
            if frame % 7 == 3 {
                continue;
            }
            if frame % 50 == 10 {
                ledger.reconfigured();
            }
            ledger.acquired();
            ledger.drawn();
            assert!(ledger.presenting().is_some_and(|(_, rendered)| rendered));
        }
        assert_eq!((ledger.unrendered, ledger.dropped), (0, 0));
        assert_eq!(ledger.presents, ledger.acquired);
    }

    #[test]
    fn black_reads_only_the_colour_channels_of_the_formats_it_knows() {
        use wgpu::TextureFormat::*;
        let zero_alpha_one = [0, 0, 0, 255, 0, 0, 0, 255];
        for format in [Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba8UnormSrgb] {
            assert_eq!(black(&zero_alpha_one, format), Some(true));
            assert_eq!(black(&[0, 0, 0, 255, 0, 1, 0, 255], format), Some(false));
            assert_eq!(black(&[16, 24, 32, 255], format), Some(false));
        }
        assert_eq!(
            black(&0xc000_0000u32.to_le_bytes(), Rgb10a2Unorm),
            Some(true)
        );
        assert_eq!(
            black(&0xc000_0001u32.to_le_bytes(), Rgb10a2Unorm),
            Some(false)
        );
        let negative_zero = [0, 0x80, 0, 0, 0, 0x80, 0, 0x3c];
        assert_eq!(black(&negative_zero, Rgba16Float), Some(true));
        assert_eq!(
            black(&[0, 0, 0, 0x3c, 0, 0, 0, 0x3c], Rgba16Float),
            Some(false)
        );
        assert_eq!(black(&[0; 4], R8Unorm), None);
        assert_eq!(texel_bytes(R8Unorm), None);
        assert_eq!(texel_bytes(Rgba16Float), Some(8));
        assert_eq!(texel_bytes(Bgra8UnormSrgb), Some(4));
    }

    #[test]
    fn the_line_names_every_count_and_says_when_there_is_no_readback() {
        let mut audit = Audit {
            path: None,
            ledger: Ledger::default(),
            readback: None,
            written: None,
        };
        audit.ledger.acquired();
        audit.ledger.drawn();
        audit.ledger.presenting();
        audit.ledger.acquired();
        audit.ledger.presenting();
        assert_eq!(
            audit.line(),
            "{\"acquired\":2,\"presents\":2,\"unrendered\":1,\"dropped\":0,\"reconfigured\":0,\"readback\":false}\n"
        );
    }

    struct Chain {
        images: Vec<crate::OffscreenTarget>,
        next: usize,
    }

    impl Chain {
        fn new(gpu: &crate::Gpu, size: [u32; 2]) -> Self {
            Self {
                images: (0..3)
                    .map(|_| {
                        gpu.offscreen(size[0], size[1], wgpu::TextureFormat::Rgba8UnormSrgb)
                            .unwrap()
                    })
                    .collect(),
                next: 0,
            }
        }

        fn acquire(&mut self) -> usize {
            let index = self.next;
            self.next = (self.next + 1) % self.images.len();
            index
        }
    }

    fn draw(gpu: &crate::Gpu, image: &crate::OffscreenTarget, frame: u64) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &image.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.02 + (frame % 5) as f64 * 0.1,
                        g: 0.05,
                        b: 0.08,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        gpu.queue.submit([encoder.finish()]);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn every_image_presented_through_skips_and_resizes_was_rendered_that_frame_and_an_undrawn_one_reads_black()
     {
        let gpu = pollster::block_on(crate::Gpu::headless()).unwrap();
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut ledger = Ledger::default();
        let mut readback = Readback::new(&gpu.device, format).unwrap();
        let sizes = [[320, 180], [257, 143], [640, 360]];
        let mut chain = Chain::new(&gpu, sizes[0]);
        let mut skipped = 0;
        for frame in 0..90u64 {
            if frame % 7 == 3 {
                skipped += 1;
                continue;
            }
            if frame % 30 == 29 {
                chain = Chain::new(&gpu, sizes[(frame / 30 + 1) as usize % sizes.len()]);
                ledger.reconfigured();
            }
            let index = chain.acquire();
            let id = ledger.acquired();
            draw(&gpu, &chain.images[index], frame);
            ledger.drawn();
            let (presented, rendered) = ledger.presenting().unwrap();
            assert_eq!((presented, rendered), (id, true));
            assert!(readback.capture(&gpu.queue, &chain.images[index].texture, id, rendered));
            readback.finish();
        }
        let presents = 90 - skipped;
        assert_eq!(ledger.presents, presents);
        assert_eq!((ledger.unrendered, ledger.dropped), (0, 0));
        assert_eq!(
            (
                readback.checked,
                readback.black,
                readback.skipped,
                readback.failed
            ),
            (presents, 0, 0, 0)
        );
        let mut fresh = Chain::new(&gpu, [200, 120]);
        ledger.reconfigured();
        let index = fresh.acquire();
        let id = ledger.acquired();
        let (_, rendered) = ledger.presenting().unwrap();
        assert!(!rendered);
        readback.capture(&gpu.queue, &fresh.images[index].texture, id, rendered);
        readback.finish();
        assert_eq!(ledger.unrendered, 1);
        assert_eq!(
            (
                readback.black,
                readback.black_unrendered,
                readback.black_frames.clone()
            ),
            (1, 1, vec![id])
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_full_ring_skips_a_capture_instead_of_waiting() {
        let gpu = pollster::block_on(crate::Gpu::headless()).unwrap();
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let image = gpu.offscreen(64, 64, format).unwrap();
        let mut readback = Readback::new(&gpu.device, format).unwrap();
        draw(&gpu, &image, 1);
        let taken = (0..AUDIT_SLOTS as u64 + 2)
            .filter(|frame| readback.capture(&gpu.queue, &image.texture, *frame, true))
            .count();
        assert_eq!(taken, AUDIT_SLOTS);
        assert_eq!(readback.skipped, 2);
        readback.finish();
        assert_eq!((readback.checked, readback.black), (AUDIT_SLOTS as u64, 0));
        assert!(readback.capture(&gpu.queue, &image.texture, 99, true));
        readback.finish();
        assert_eq!(readback.checked, AUDIT_SLOTS as u64 + 1);
    }

    #[test]
    fn the_file_is_replaced_whole_and_holds_one_line() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/gpu-tests/audit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.json");
        let _ = std::fs::remove_file(&path);
        let mut audit = Audit {
            path: Some(path.clone()),
            ledger: Ledger::default(),
            readback: None,
            written: None,
        };
        audit.ledger.acquired();
        audit.ledger.drawn();
        audit.ledger.presenting();
        audit.presented();
        for _ in 1..WRITE_EVERY {
            audit.ledger.acquired();
            audit.ledger.drawn();
            audit.ledger.presenting();
            audit.presented();
        }
        let early = std::fs::read_to_string(&path).unwrap();
        assert!(early.starts_with("{\"acquired\":1,"));
        audit.ledger.acquired();
        audit.ledger.drawn();
        audit.ledger.presenting();
        audit.presented();
        let due = std::fs::read_to_string(&path).unwrap();
        assert!(due.starts_with(&format!("{{\"acquired\":{},", WRITE_EVERY + 1)));
        audit.ledger.acquired();
        drop(audit);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.starts_with(&format!(
            "{{\"acquired\":{},\"presents\":{},",
            WRITE_EVERY + 2,
            WRITE_EVERY + 1
        )));
        let mut part = path.into_os_string();
        part.push(".part");
        assert!(!Path::new(&part).exists());
    }
}
