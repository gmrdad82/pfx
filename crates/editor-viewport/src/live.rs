use std::collections::BTreeMap;

use egui::TextureId;
use pfx_gpu::{Gpu, wgpu};
use pfx_live::demand::{Limits, Need, Next};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::{Override, Staged};
use pfx_load::scene::{Camera, Scene, SceneDiff};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const SHOWN: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const IN_FLIGHT: u64 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    pub drew: bool,
    pub picked: Option<u32>,
    pub awaiting: bool,
}

struct Surface {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    shown: wgpu::TextureView,
}

fn surface(gpu: &Gpu, size: [u32; 2]) -> Surface {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("editor viewport"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[SHOWN],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let shown = texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(SHOWN),
        ..wgpu::TextureViewDescriptor::default()
    });
    Surface {
        texture,
        view,
        shown,
    }
}

pub struct Live {
    gpu: Gpu,
    renderer: Renderer,
    staged: Staged,
    scene: Scene,
    revision: u64,
    camera: Camera,
    surface: Surface,
    id: TextureId,
    size: [u32; 2],
    awaiting: bool,
    drawn: u64,
}

fn aspect(size: [u32; 2]) -> f32 {
    size[0].max(1) as f32 / size[1].max(1) as f32
}

impl Live {
    pub fn new(
        gpu: &Gpu,
        egui: &mut egui_wgpu::Renderer,
        scene: &Scene,
        revision: u64,
        camera: Camera,
        size: [u32; 2],
    ) -> Result<Live, String> {
        let mut renderer = Renderer::new_with_output_format(gpu.clone(), size[0], size[1], FORMAT)
            .map_err(|error| format!("the live renderer could not start: {error}"))?;
        renderer.set_exposure(Exposure::Fixed(1.0))?;
        renderer.set_frames_on_demand(Some(Limits::default()))?;
        let mut shown = scene.clone();
        shown.camera = Some(camera);
        let staged = renderer.stage(&shown)?;
        renderer.wait_sky()?;
        renderer.set_lens(staged.lens(aspect(size)));
        let surface = surface(gpu, size);
        let id =
            egui.register_native_texture(&gpu.device, &surface.shown, wgpu::FilterMode::Nearest);
        Ok(Live {
            gpu: gpu.clone(),
            renderer,
            staged,
            scene: shown,
            revision,
            camera,
            surface,
            id,
            size,
            awaiting: false,
            drawn: 0,
        })
    }

    pub fn texture(&self) -> TextureId {
        self.id
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn drawn(&self) -> u64 {
        self.drawn
    }

    pub fn staged(&self) -> &Staged {
        &self.staged
    }

    pub fn image(&self) -> &wgpu::Texture {
        &self.surface.texture
    }

    pub fn resize(
        &mut self,
        egui: &mut egui_wgpu::Renderer,
        size: [u32; 2],
    ) -> Result<bool, String> {
        if size == self.size {
            return Ok(false);
        }
        self.renderer.resize(size[0], size[1])?;
        self.surface = surface(&self.gpu, size);
        egui.update_egui_texture_from_wgpu_texture(
            &self.gpu.device,
            &self.surface.shown,
            wgpu::FilterMode::Nearest,
            self.id,
        );
        self.size = size;
        self.renderer.set_lens(self.staged.lens(aspect(size)));
        Ok(true)
    }

    pub fn show(&mut self, scene: &Scene, revision: u64) -> Result<(), String> {
        let mut next = scene.clone();
        next.camera = self.scene.camera;
        let diff = self.scene.diff(&next);
        self.staged.apply(&mut self.renderer, &next, &diff)?;
        self.scene = next;
        self.revision = revision;
        Ok(())
    }

    pub fn look(&mut self, camera: Camera) -> Result<(), String> {
        if self.camera == camera {
            return Ok(());
        }
        self.scene.camera = Some(camera);
        let diff = SceneDiff {
            camera: true,
            ..SceneDiff::default()
        };
        self.staged.apply(&mut self.renderer, &self.scene, &diff)?;
        self.renderer.set_lens(self.staged.lens(aspect(self.size)));
        self.camera = camera;
        Ok(())
    }

    pub fn overrides(&mut self, wanted: &BTreeMap<String, Override>) {
        let gone: Vec<String> = self
            .staged
            .overrides()
            .keys()
            .filter(|name| !wanted.contains_key(*name))
            .cloned()
            .collect();
        for name in gone {
            self.staged.clear_override(&name);
        }
        for (name, change) in wanted {
            if self.staged.overrides().get(name) != Some(change) {
                self.staged.set_override(name, *change);
            }
        }
    }

    pub fn gpu_ms(&self) -> Option<f64> {
        self.renderer
            .timings()
            .filter(|timings| !timings.passes.is_empty())
            .map(|timings| timings.passes.iter().map(|t| t.milliseconds).sum())
    }

    pub fn drops(&self) -> u64 {
        let drops = self.renderer.timing_drops();
        drops.ring + drops.late
    }

    pub fn draw(&mut self, pick: Option<[u32; 2]>) -> Result<Drawn, String> {
        let mut drawn = Drawn::default();
        if self.awaiting
            && let Some(picked) = self.renderer.picked()
        {
            self.awaiting = false;
            drawn.picked = Some(picked.id);
        }
        let size = self.size;
        let finish = self.staged.finish();
        let frame = self.staged.frame(aspect(size), 0.0, 0);
        let next = Next::new(&frame.scene, &frame.text, &frame.effects, finish);
        if let Some([x, y]) = pick {
            self.renderer.pick(x.min(size[0] - 1), y.min(size[1] - 1))?;
            self.renderer
                .render_needed(&next, &Need::Full, &self.surface.view)?;
            match self.renderer.picked() {
                Some(picked) => drawn.picked = Some(picked.id),
                None => self.awaiting = true,
            }
            drawn.drew = true;
        } else {
            let need = self.renderer.frame_needed(&next);
            if need.draws() {
                if self.renderer.frames_in_flight() >= IN_FLIGHT {
                    self.renderer.wait_frames(IN_FLIGHT - 1)?;
                }
                self.renderer
                    .submit_needed(&next, &need, &self.surface.view)?;
                drawn.drew = true;
            }
        }
        if drawn.drew {
            self.drawn += 1;
        }
        drawn.awaiting = self.awaiting;
        Ok(drawn)
    }
}
