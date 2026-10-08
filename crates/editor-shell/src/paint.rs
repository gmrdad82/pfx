use egui::epaint::ClippedPrimitive;
use egui::{Color32, TexturesDelta};
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use pfx_gpu::{Gpu, wgpu};

pub struct Painter {
    renderer: Renderer,
}

pub fn linear(v: u8) -> f64 {
    let c = f64::from(v) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn clear(colour: Color32) -> wgpu::Color {
    wgpu::Color {
        r: linear(colour.r()),
        g: linear(colour.g()),
        b: linear(colour.b()),
        a: 1.0,
    }
}

impl Painter {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, options: RendererOptions) -> Painter {
        Painter {
            renderer: Renderer::new(&gpu.device, format, options),
        }
    }

    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    pub fn paint(
        &mut self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        screen: &ScreenDescriptor,
        textures: &TexturesDelta,
        primitives: &[ClippedPrimitive],
        ground: Color32,
    ) {
        for (id, delta) in &textures.set {
            self.renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("editor ui"),
            });
        let mut commands =
            self.renderer
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, primitives, screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("editor ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear(ground)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut pass = pass.forget_lifetime();
            self.renderer.render(&mut pass, primitives, screen);
        }
        commands.push(encoder.finish());
        gpu.queue.submit(commands);
        for id in &textures.free {
            self.renderer.free_texture(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clear_colour_is_linear() {
        let colour = clear(Color32::from_rgb(0, 255, 0x80));
        assert_eq!(colour.r, 0.0);
        assert_eq!(colour.g, 1.0);
        assert!((colour.b - 0.2158605).abs() < 1e-6);
    }
}
