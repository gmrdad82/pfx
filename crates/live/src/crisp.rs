use pfx_gpu::{GpuProfiler, wgpu};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const WORKGROUP: u32 = 8;

pub const SHARPEN_WGSL: &str = r#"
struct Params {
    width: u32,
    height: u32,
    strength: f32,
    exposure: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var display: texture_storage_2d<rgba16float, write>;

fn tap(p: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(i32(params.width), i32(params.height));
    let c = max(textureLoad(source, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0).rgb * params.exposure, vec3<f32>(0.0));
    return c / (1.0 + max(c.r, max(c.g, c.b)));
}

@compute @workgroup_size(8, 8)
fn sharpen(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    let p = vec2<i32>(gid.xy);
    let centre = textureLoad(source, p, 0);
    let a = tap(p + vec2<i32>(-1, -1));
    let b = tap(p + vec2<i32>(0, -1));
    let c = tap(p + vec2<i32>(1, -1));
    let d = tap(p + vec2<i32>(-1, 0));
    let e = tap(p);
    let f = tap(p + vec2<i32>(1, 0));
    let g = tap(p + vec2<i32>(-1, 1));
    let h = tap(p + vec2<i32>(0, 1));
    let i = tap(p + vec2<i32>(1, 1));
    let cross_lo = min(min(min(d, e), min(f, b)), h);
    let cross_hi = max(max(max(d, e), max(f, b)), h);
    let lo = cross_lo + min(cross_lo, min(min(a, c), min(g, i)));
    let hi = cross_hi + max(cross_hi, max(max(a, c), max(g, i)));
    let amp = sqrt(clamp(min(lo, vec3<f32>(2.0) - hi) / max(hi, vec3<f32>(1.0e-5)), vec3<f32>(0.0), vec3<f32>(1.0)));
    let w = amp * (-params.strength / mix(8.0, 5.0, params.strength));
    let t = clamp((b * w + d * w + f * w + h * w + e) / (vec3<f32>(1.0) + 4.0 * w), vec3<f32>(0.0), vec3<f32>(0.999));
    let back = t / (1.0 - max(t.r, max(t.g, t.b))) / params.exposure;
    textureStore(display, p, vec4<f32>(back, centre.a));
}

@compute @workgroup_size(8, 8)
fn copy(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    let p = vec2<i32>(gid.xy);
    textureStore(display, p, textureLoad(source, p, 0));
}
"#;

pub fn check_strength(strength: f32) -> Result<f32, String> {
    if strength.is_finite() && (0.0..=1.0).contains(&strength) {
        Ok(strength)
    } else {
        Err("sharpen strength must be between 0 and 1".into())
    }
}

fn squash(c: [f32; 3], exposure: f32) -> [f32; 3] {
    let c = c.map(|v| (v * exposure).max(0.0));
    let peak = c[0].max(c[1]).max(c[2]);
    c.map(|v| v / (1.0 + peak))
}

pub fn sharpen(pixels: &[[f32; 4]], width: usize, strength: f32, exposure: f32) -> Vec<[f32; 4]> {
    if strength == 0.0 || width == 0 {
        return pixels.to_vec();
    }
    let height = pixels.len() / width;
    let tap = |x: isize, y: isize| {
        let x = x.clamp(0, width as isize - 1) as usize;
        let y = y.clamp(0, height as isize - 1) as usize;
        let p = pixels[y * width + x];
        squash([p[0], p[1], p[2]], exposure)
    };
    let peak = -strength / (8.0 + (5.0 - 8.0) * strength);
    let mut out = Vec::with_capacity(pixels.len());
    for y in 0..height as isize {
        for x in 0..width as isize {
            let taps: [[f32; 3]; 9] =
                std::array::from_fn(|k| tap(x + k as isize % 3 - 1, y + k as isize / 3 - 1));
            let mut back = [0.0; 3];
            for (channel, slot) in back.iter_mut().enumerate() {
                let v: [f32; 9] = std::array::from_fn(|k| taps[k][channel]);
                let cross = [v[1], v[3], v[4], v[5], v[7]];
                let corners = [v[0], v[2], v[6], v[8]];
                let cross_lo = cross.iter().copied().fold(f32::INFINITY, f32::min);
                let cross_hi = cross.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let lo = cross_lo + corners.iter().copied().fold(cross_lo, f32::min);
                let hi = cross_hi + corners.iter().copied().fold(cross_hi, f32::max);
                let amp = ((lo.min(2.0 - hi) / hi.max(1.0e-5)).clamp(0.0, 1.0)).sqrt();
                let w = amp * peak;
                *slot = ((v[1] * w + v[3] * w + v[5] * w + v[7] * w + v[4]) / (1.0 + 4.0 * w))
                    .clamp(0.0, 0.999);
            }
            let top = back[0].max(back[1]).max(back[2]);
            let centre = pixels[y as usize * width + x as usize];
            out.push([
                back[0] / (1.0 - top) / exposure,
                back[1] / (1.0 - top) / exposure,
                back[2] / (1.0 - top) / exposure,
                centre[3],
            ]);
        }
    }
    out
}

pub struct Pass {
    layout: wgpu::BindGroupLayout,
    sharpen: wgpu::ComputePipeline,
    copy: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    target: Option<(wgpu::Texture, wgpu::TextureView)>,
    width: u32,
    height: u32,
    viewport: (u32, u32),
}

impl Pass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("crisp"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crisp"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crisp"),
            source: wgpu::ShaderSource::Wgsl(SHARPEN_WGSL.into()),
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            sharpen: pipeline("sharpen"),
            copy: pipeline("copy"),
            layout,
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("crisp params"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            target: None,
            width,
            height,
            viewport: (width, height),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) != (self.width, self.height) {
            self.target = None;
            self.width = width;
            self.height = height;
        }
        self.viewport = (width, height);
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        crate::viewport::check([width, height], [self.width, self.height])?;
        self.viewport = (width, height);
        Ok(())
    }

    pub fn target(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(|(texture, _)| texture)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        strength: f32,
        exposure: f32,
        profiler: Option<&mut GpuProfiler>,
    ) -> &wgpu::TextureView {
        let (capacity_width, capacity_height) = (self.width, self.height);
        let (width, height) = self.viewport;
        let (_, target) = self.target.get_or_insert_with(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("crisp display"),
                size: wgpu::Extent3d {
                    width: capacity_width,
                    height: capacity_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            (texture, view)
        });
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&strength.to_le_bytes());
        bytes.extend_from_slice(&exposure.max(1.0e-8).to_le_bytes());
        queue.write_buffer(&self.params, 0, &bytes);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crisp"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(target),
                },
            ],
        });
        let sharpening = strength > 0.0;
        let mut profiler = profiler;
        let timing = profiler
            .as_deref_mut()
            .and_then(|timer| timer.pass(if sharpening { "sharpen" } else { "crisp copy" }));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("crisp"),
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|timer| timer.compute_writes(timing)),
            });
            pass.set_pipeline(if sharpening {
                &self.sharpen
            } else {
                &self.copy
            });
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(WORKGROUP), height.div_ceil(WORKGROUP), 1);
        }
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(width: usize, height: usize) -> Vec<[f32; 4]> {
        (0..width * height)
            .map(|k| {
                let x = k % width;
                let v = match x {
                    0..=3 => 0.05,
                    4 => 0.3,
                    5 => 0.6,
                    _ => 0.85,
                };
                [v, v * 0.9, v * 0.8, 1.0]
            })
            .collect()
    }

    #[test]
    fn strength_zero_is_the_input() {
        let pixels = edge(10, 4);
        assert_eq!(sharpen(&pixels, 10, 0.0, 1.0), pixels);
    }

    #[test]
    fn a_flat_image_stays_flat() {
        let pixels = vec![[0.4, 0.2, 0.1, 0.7]; 36];
        for strength in [0.25, 1.0] {
            for p in sharpen(&pixels, 6, strength, 1.3) {
                for c in 0..3 {
                    assert!((p[c] - pixels[0][c]).abs() < 1e-5, "{p:?}");
                }
                assert_eq!(p[3], 0.7);
            }
        }
    }

    #[test]
    fn an_edge_gets_steeper_with_strength_and_stays_finite() {
        let pixels = edge(10, 4);
        let slope = |image: &[[f32; 4]]| image[10 + 5][0] - image[10 + 4][0];
        let soft = slope(&pixels);
        let half = sharpen(&pixels, 10, 0.5, 1.0);
        let full = sharpen(&pixels, 10, 1.0, 1.0);
        assert!(slope(&half) > soft, "{} against {soft}", slope(&half));
        assert!(slope(&full) > slope(&half));
        for p in full.iter().chain(&half) {
            assert!(p.iter().all(|v| v.is_finite() && *v >= 0.0), "{p:?}");
        }
    }

    #[test]
    fn hdr_values_round_trip_through_the_squash() {
        let pixels = vec![[12.0, 3.0, 0.5, 1.0]; 25];
        for p in sharpen(&pixels, 5, 1.0, 0.5) {
            for c in 0..3 {
                assert!((p[c] - pixels[0][c]).abs() / pixels[0][c] < 1e-3, "{p:?}");
            }
        }
    }

    #[test]
    fn strength_outside_zero_to_one_is_refused() {
        assert!(check_strength(-0.1).is_err());
        assert!(check_strength(1.5).is_err());
        assert!(check_strength(f32::NAN).is_err());
        assert_eq!(check_strength(0.3), Ok(0.3));
    }

    #[test]
    fn the_shader_validates() {
        let module = naga::front::wgsl::parse_str(SHARPEN_WGSL).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
