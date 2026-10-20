use bytemuck::{Pod, Zeroable};
use pfx_gpu::GpuProfiler;

pub const SHARPNESS: f32 = 0.2;
pub const INTERMEDIATE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub const WGSL: &str = r#"
struct Fsr {
    con0: vec4f,
    last: vec2i,
    sharpness: f32,
    pad: f32,
}

@group(0) @binding(0) var fsr_source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> fsr: Fsr;

@vertex
fn fsr_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(xy[index], 0.0, 1.0);
}

fn prx_lo_rcp(a: f32) -> f32 {
    return bitcast<f32>(0x7ef07ebbu - bitcast<u32>(a));
}

fn prx_med_rcp(a: f32) -> f32 {
    let b = bitcast<f32>(0x7ef19fffu - bitcast<u32>(a));
    return b * (-b * a + 2.0);
}

fn prx_lo_rsq(a: f32) -> f32 {
    return bitcast<f32>(0x5f347d74u - (bitcast<u32>(a) >> 1u));
}

fn easu_load(p: vec2i) -> vec4f {
    return textureLoad(fsr_source, clamp(p, vec2i(0), fsr.last), 0);
}

fn easu_luma(c: vec4f) -> f32 {
    return c.b * 0.5 + (c.r * 0.5 + c.g);
}

fn easu_tap(
    acc: ptr<function, vec4f>,
    off: vec2f,
    dir: vec2f,
    len: vec2f,
    lob: f32,
    clp: f32,
    c: vec3f,
) {
    var v = vec2f(off.x * dir.x + off.y * dir.y, off.x * -dir.y + off.y * dir.x);
    v *= len;
    let d2 = min(v.x * v.x + v.y * v.y, clp);
    var wb = (2.0 / 5.0) * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb *= wb;
    wa *= wa;
    wb = (25.0 / 16.0) * wb - (25.0 / 16.0 - 1.0);
    let w = wb * wa;
    *acc += vec4f(c * w, w);
}

fn easu_set(
    dir: ptr<function, vec2f>,
    len: ptr<function, f32>,
    w: f32,
    la: f32,
    lb: f32,
    lc: f32,
    ld: f32,
    le: f32,
) {
    let dc = ld - lc;
    let cb = lc - lb;
    var len_x = prx_lo_rcp(max(abs(dc), abs(cb)));
    let dir_x = ld - lb;
    len_x = clamp(abs(dir_x) * len_x, 0.0, 1.0);
    len_x *= len_x;
    let ec = le - lc;
    let ca = lc - la;
    var len_y = prx_lo_rcp(max(abs(ec), abs(ca)));
    let dir_y = le - la;
    len_y = clamp(abs(dir_y) * len_y, 0.0, 1.0);
    len_y *= len_y;
    *dir += vec2f(dir_x, dir_y) * w;
    *len += (len_x + len_y) * w;
}

@fragment
fn easu_fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let ip = floor(position.xy);
    var pp = ip * fsr.con0.xy + fsr.con0.zw;
    let fp = floor(pp);
    pp -= fp;
    let f0 = vec2i(fp);
    let b = easu_load(f0 + vec2i(0, -1));
    let c = easu_load(f0 + vec2i(1, -1));
    let e = easu_load(f0 + vec2i(-1, 0));
    let f = easu_load(f0);
    let g = easu_load(f0 + vec2i(1, 0));
    let h = easu_load(f0 + vec2i(2, 0));
    let i = easu_load(f0 + vec2i(-1, 1));
    let j = easu_load(f0 + vec2i(0, 1));
    let k = easu_load(f0 + vec2i(1, 1));
    let l = easu_load(f0 + vec2i(2, 1));
    let n = easu_load(f0 + vec2i(0, 2));
    let o = easu_load(f0 + vec2i(1, 2));
    let bl = easu_luma(b);
    let cl = easu_luma(c);
    let el = easu_luma(e);
    let fl = easu_luma(f);
    let gl = easu_luma(g);
    let hl = easu_luma(h);
    let il = easu_luma(i);
    let jl = easu_luma(j);
    let kl = easu_luma(k);
    let ll = easu_luma(l);
    let nl = easu_luma(n);
    let ol = easu_luma(o);
    var dir = vec2f(0.0);
    var len = 0.0;
    easu_set(&dir, &len, (1.0 - pp.x) * (1.0 - pp.y), bl, el, fl, gl, jl);
    easu_set(&dir, &len, pp.x * (1.0 - pp.y), cl, fl, gl, hl, kl);
    easu_set(&dir, &len, (1.0 - pp.x) * pp.y, fl, il, jl, kl, nl);
    easu_set(&dir, &len, pp.x * pp.y, gl, jl, kl, ll, ol);
    let dir2 = dir * dir;
    var dir_r = dir2.x + dir2.y;
    let zero = dir_r < 1.0 / 32768.0;
    dir_r = prx_lo_rsq(dir_r);
    dir_r = select(dir_r, 1.0, zero);
    dir.x = select(dir.x, 1.0, zero);
    dir *= vec2f(dir_r);
    len = len * 0.5;
    len *= len;
    let stretch = (dir.x * dir.x + dir.y * dir.y) * prx_lo_rcp(max(abs(dir.x), abs(dir.y)));
    let len2 = vec2f(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    let lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
    let clp = prx_lo_rcp(lob);
    let min4 = min(min(f.rgb, g.rgb), min(j.rgb, k.rgb));
    let max4 = max(max(f.rgb, g.rgb), max(j.rgb, k.rgb));
    var acc = vec4f(0.0);
    easu_tap(&acc, vec2f(0.0, -1.0) - pp, dir, len2, lob, clp, b.rgb);
    easu_tap(&acc, vec2f(1.0, -1.0) - pp, dir, len2, lob, clp, c.rgb);
    easu_tap(&acc, vec2f(-1.0, 1.0) - pp, dir, len2, lob, clp, i.rgb);
    easu_tap(&acc, vec2f(0.0, 1.0) - pp, dir, len2, lob, clp, j.rgb);
    easu_tap(&acc, vec2f(0.0, 0.0) - pp, dir, len2, lob, clp, f.rgb);
    easu_tap(&acc, vec2f(-1.0, 0.0) - pp, dir, len2, lob, clp, e.rgb);
    easu_tap(&acc, vec2f(1.0, 1.0) - pp, dir, len2, lob, clp, k.rgb);
    easu_tap(&acc, vec2f(2.0, 1.0) - pp, dir, len2, lob, clp, l.rgb);
    easu_tap(&acc, vec2f(2.0, 0.0) - pp, dir, len2, lob, clp, h.rgb);
    easu_tap(&acc, vec2f(1.0, 0.0) - pp, dir, len2, lob, clp, g.rgb);
    easu_tap(&acc, vec2f(1.0, 2.0) - pp, dir, len2, lob, clp, o.rgb);
    easu_tap(&acc, vec2f(0.0, 2.0) - pp, dir, len2, lob, clp, n.rgb);
    let pix = min(max4, max(min4, acc.rgb / acc.w));
    let near = select(select(f, g, pp.x >= 0.5), select(j, k, pp.x >= 0.5), pp.y >= 0.5);
    return vec4f(pix, near.a);
}

fn rcas_load(p: vec2i) -> vec4f {
    return textureLoad(fsr_source, clamp(p, vec2i(0), fsr.last), 0);
}

fn min3(a: f32, b: f32, c: f32) -> f32 {
    return min(a, min(b, c));
}

fn max3(a: f32, b: f32, c: f32) -> f32 {
    return max(a, max(b, c));
}

@fragment
fn rcas_fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let sp = vec2i(floor(position.xy));
    let b = rcas_load(sp + vec2i(0, -1)).rgb;
    let d = rcas_load(sp + vec2i(-1, 0)).rgb;
    let ee = rcas_load(sp);
    let e = ee.rgb;
    let f = rcas_load(sp + vec2i(1, 0)).rgb;
    let h = rcas_load(sp + vec2i(0, 1)).rgb;
    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));
    let peak = vec2f(1.0, -4.0);
    let hit_min = min(mn4, e) / (4.0 * mx4);
    let hit_max = (peak.x - max(mx4, e)) / (4.0 * mn4 + peak.y);
    let lobe3 = max(-hit_min, hit_max);
    let limit = 0.25 - 1.0 / 16.0;
    let lobe = max(-limit, min(max3(lobe3.r, lobe3.g, lobe3.b), 0.0)) * fsr.sharpness;
    let rcp_l = prx_med_rcp(4.0 * lobe + 1.0);
    let pix = (lobe * b + lobe * d + lobe * h + lobe * f + e) * rcp_l;
    return vec4f(pix, ee.a);
}
"#;

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Uniform {
    con0: [f32; 4],
    last: [i32; 2],
    sharpness: f32,
    pad: f32,
}

pub struct Scaling<'a> {
    pub source: &'a wgpu::TextureView,
    pub source_size: [u32; 2],
    pub output: &'a wgpu::TextureView,
    pub output_format: wgpu::TextureFormat,
    pub output_size: [u32; 2],
    pub sharpness: f32,
}

struct Middle {
    view: wgpu::TextureView,
    size: [u32; 2],
}

pub struct Fsr {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    easu: Option<wgpu::RenderPipeline>,
    rcas: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    easu_uniform: wgpu::Buffer,
    rcas_uniform: wgpu::Buffer,
    easu_group: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    rcas_group: Option<wgpu::BindGroup>,
    middle: Option<Middle>,
}

pub fn easu_con0(source: [u32; 2], output: [u32; 2]) -> [f32; 4] {
    let x = source[0] as f32 / output[0].max(1) as f32;
    let y = source[1] as f32 / output[1].max(1) as f32;
    [x, y, 0.5 * x - 0.5, 0.5 * y - 0.5]
}

pub fn sharpness(stops: f32) -> f32 {
    (-stops.max(0.0)).exp2()
}

fn last(size: [u32; 2]) -> [i32; 2] {
    size.map(|side| side.max(1) as i32 - 1)
}

impl Fsr {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fsr"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fsr"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fsr"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let uniform = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            easu_uniform: uniform("fsr easu", std::mem::size_of::<Uniform>() as u64),
            rcas_uniform: uniform("fsr rcas", std::mem::size_of::<Uniform>() as u64),
            layout,
            pipeline_layout,
            module,
            easu: None,
            rcas: Vec::new(),
            easu_group: None,
            rcas_group: None,
            middle: None,
        }
    }

    fn pipeline(
        &self,
        device: &wgpu::Device,
        label: &str,
        entry: &str,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("fsr_vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some(entry),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn warm(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.easu.is_none() {
            self.easu = Some(self.pipeline(device, "fsr easu", "easu_fragment", INTERMEDIATE));
        }
        if !self.rcas.iter().any(|(made, _)| *made == format) {
            let pipeline = self.pipeline(device, "fsr rcas", "rcas_fragment", format);
            self.rcas.push((format, pipeline));
        }
    }

    pub fn pipelines_made(&self) -> usize {
        usize::from(self.easu.is_some()) + self.rcas.len()
    }

    fn middle(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        if let Some(middle) = self.middle.as_ref().filter(|middle| middle.size == size) {
            return middle.view.clone();
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fsr easu"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: INTERMEDIATE,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.rcas_group = None;
        self.middle = Some(Middle {
            view: view.clone(),
            size,
        });
        view
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scaling: &Scaling<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<(), String> {
        if scaling.source_size.contains(&0) || scaling.output_size.contains(&0) {
            return Err("the scaled sizes must be at least 1 by 1".into());
        }
        if scaling.output_format.is_srgb() {
            return Err("the scaled output must be a unorm view, not sRGB".into());
        }
        self.warm(device, scaling.output_format);
        let middle = self.middle(device, scaling.output_size);
        queue.write_buffer(
            &self.easu_uniform,
            0,
            bytemuck::bytes_of(&Uniform {
                con0: easu_con0(scaling.source_size, scaling.output_size),
                last: last(scaling.source_size),
                sharpness: 0.0,
                pad: 0.0,
            }),
        );
        queue.write_buffer(
            &self.rcas_uniform,
            0,
            bytemuck::bytes_of(&Uniform {
                con0: [0.0; 4],
                last: last(scaling.output_size),
                sharpness: sharpness(scaling.sharpness),
                pad: 0.0,
            }),
        );
        if self
            .easu_group
            .as_ref()
            .is_none_or(|(source, _)| source != scaling.source)
        {
            let group = self.group(device, "fsr easu", scaling.source, &self.easu_uniform);
            self.easu_group = Some((scaling.source.clone(), group));
        }
        if self.rcas_group.is_none() {
            self.rcas_group = Some(self.group(device, "fsr rcas", &middle, &self.rcas_uniform));
        }
        let rcas = self
            .rcas
            .iter()
            .find(|(made, _)| *made == scaling.output_format)
            .map(|(_, pipeline)| pipeline)
            .ok_or("the rcas pipeline is missing")?;
        let easu = self.easu.as_ref().ok_or("the easu pipeline is missing")?;
        let steps = [
            (
                "fsr easu",
                &middle,
                easu,
                self.easu_group.as_ref().map(|(_, group)| group),
            ),
            ("fsr rcas", scaling.output, rcas, self.rcas_group.as_ref()),
        ];
        for (label, output, pipeline, group) in steps {
            let timing = profiler
                .as_deref_mut()
                .and_then(|profiler| profiler.pass(label));
            let writes = profiler
                .as_deref()
                .and_then(|profiler| profiler.render_writes(timing));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: writes,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(())
    }

    fn group(
        &self,
        device: &wgpu::Device,
        label: &str,
        source: &wgpu::TextureView,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fsr_shader_validates() {
        let module = naga::front::wgsl::parse_str(WGSL).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn fsr_keeps_flat_colour_and_an_edge_between_its_sides() {
        use wgpu::util::DeviceExt;
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let (width, height) = (48u32, 27u32);
        let left = [200u8, 60, 30, 255];
        let right = [20u8, 90, 220, 255];
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|index| {
                if index % width < width / 2 {
                    left
                } else {
                    right
                }
            })
            .collect();
        let source = gpu.device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("fsr test source"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &pixels,
        );
        let view = source.create_view(&Default::default());
        let out = [width * 2, height * 2];
        let target = gpu
            .offscreen(out[0], out[1], wgpu::TextureFormat::Rgba8Unorm)
            .unwrap();
        let mut fsr = Fsr::new(&gpu.device);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        fsr.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &Scaling {
                source: &view,
                source_size: [width, height],
                output: &target.view,
                output_format: wgpu::TextureFormat::Rgba8Unorm,
                output_size: out,
                sharpness: SHARPNESS,
            },
            None,
        )
        .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let bytes = gpu.readback_bytes(&target).unwrap();
        let at = |x: u32, y: u32| {
            let index = ((y * out[0] + x) * 4) as usize;
            [
                bytes[index],
                bytes[index + 1],
                bytes[index + 2],
                bytes[index + 3],
            ]
        };
        let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1);
        for y in 0..out[1] {
            assert!(near(at(0, y), left) && near(at(out[0] / 4, y), left));
            assert!(near(at(out[0] - 1, y), right) && near(at(out[0] * 3 / 4, y), right));
            for x in 0..out[0] {
                let pixel = at(x, y);
                for channel in 0..3 {
                    let low = left[channel].min(right[channel]).saturating_sub(8);
                    let high = left[channel].max(right[channel]).saturating_add(8);
                    assert!((low..=high).contains(&pixel[channel]), "{x},{y}: {pixel:?}");
                }
            }
        }
    }

    #[test]
    fn easu_maps_output_pixel_centres_into_the_source() {
        let [x, y, ox, oy] = easu_con0([1920, 1080], [3840, 2160]);
        assert_eq!([x, y], [0.5, 0.5]);
        assert_eq!([ox, oy], [-0.25, -0.25]);
        assert_eq!(easu_con0([7, 5], [7, 5]), [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(sharpness(0.0), 1.0);
        assert_eq!(sharpness(1.0), 0.5);
    }
}
