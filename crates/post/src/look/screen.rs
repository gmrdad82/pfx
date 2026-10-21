use super::gpu::{LookGpu, Surface};
use super::{Crt, Curvature, Glow, Grain, Place, Scanlines, Vignette};
use pfx_gpu::GpuProfiler;

pub const CURVE_EDGE: f32 = 0.01;
pub const CURVE_MAX: f32 = 0.25;
pub const EDGE_MAX: f32 = 0.5;
pub const PERIOD_MIN: f32 = 1.0;
pub const LENGTH_MAX: f32 = 64.0;
pub const VIGNETTE_MAX: f32 = 4.0;
pub const BLOOM_MAX: f32 = 4.0;
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const SLOTS: [usize; 2] = [0, 1];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePart {
    pub on: bool,
    pub strength: f32,
    pub edge: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScanlinePart {
    pub on: bool,
    pub strength: f32,
    pub period: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrainPart {
    pub on: bool,
    pub strength: f32,
    pub size: f32,
    pub response: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VignettePart {
    pub on: bool,
    pub strength: f32,
    pub radius: f32,
    pub falloff: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomPart {
    pub on: bool,
    pub strength: f32,
    pub threshold: f32,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScreenPart {
    Curve,
    Scanlines,
    Grain,
    Vignette,
    Bloom,
}

impl ScreenPart {
    pub const ALL: [ScreenPart; 5] = [
        ScreenPart::Curve,
        ScreenPart::Scanlines,
        ScreenPart::Grain,
        ScreenPart::Vignette,
        ScreenPart::Bloom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ScreenPart::Curve => "curve",
            ScreenPart::Scanlines => "scanlines",
            ScreenPart::Grain => "grain",
            ScreenPart::Vignette => "vignette",
            ScreenPart::Bloom => "bloom",
        }
    }

    pub fn named(name: &str) -> Option<ScreenPart> {
        ScreenPart::ALL.into_iter().find(|part| part.name() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenLook {
    pub curve: CurvePart,
    pub scanlines: ScanlinePart,
    pub grain: GrainPart,
    pub vignette: VignettePart,
    pub bloom: BloomPart,
}

impl Default for ScreenLook {
    fn default() -> Self {
        Self::crt()
    }
}

fn within(value: f32, low: f32, high: f32) -> bool {
    value.is_finite() && (low..=high).contains(&value)
}

fn above(value: f32, low: f32, high: f32) -> bool {
    value.is_finite() && value > low && value <= high
}

impl ScreenLook {
    pub const fn crt() -> Self {
        Self {
            curve: CurvePart {
                on: true,
                strength: 0.02,
                edge: CURVE_EDGE,
            },
            scanlines: ScanlinePart {
                on: true,
                strength: 0.065,
                period: 2.25,
            },
            grain: GrainPart {
                on: true,
                strength: 0.04,
                size: 1.0,
                response: 0.68,
            },
            vignette: VignettePart {
                on: true,
                strength: 0.3,
                radius: 0.75,
                falloff: 0.8,
            },
            bloom: BloomPart {
                on: true,
                strength: 0.3,
                threshold: 0.7,
                radius: 4.0,
            },
        }
    }

    pub fn off() -> Self {
        let mut look = Self::crt();
        for part in ScreenPart::ALL {
            look.set(part, false);
        }
        look
    }

    pub fn only(part: ScreenPart) -> Self {
        Self::off().with(part, true)
    }

    pub fn with(mut self, part: ScreenPart, on: bool) -> Self {
        self.set(part, on);
        self
    }

    pub fn set(&mut self, part: ScreenPart, on: bool) {
        match part {
            ScreenPart::Curve => self.curve.on = on,
            ScreenPart::Scanlines => self.scanlines.on = on,
            ScreenPart::Grain => self.grain.on = on,
            ScreenPart::Vignette => self.vignette.on = on,
            ScreenPart::Bloom => self.bloom.on = on,
        }
    }

    pub fn strength(&self, part: ScreenPart) -> f32 {
        match part {
            ScreenPart::Curve => self.curve.strength,
            ScreenPart::Scanlines => self.scanlines.strength,
            ScreenPart::Grain => self.grain.strength,
            ScreenPart::Vignette => self.vignette.strength,
            ScreenPart::Bloom => self.bloom.strength,
        }
    }

    pub fn set_strength(&mut self, part: ScreenPart, strength: f32) {
        match part {
            ScreenPart::Curve => self.curve.strength = strength,
            ScreenPart::Scanlines => self.scanlines.strength = strength,
            ScreenPart::Grain => self.grain.strength = strength,
            ScreenPart::Vignette => self.vignette.strength = strength,
            ScreenPart::Bloom => self.bloom.strength = strength,
        }
    }

    pub fn is_on(&self, part: ScreenPart) -> bool {
        let on = match part {
            ScreenPart::Curve => self.curve.on,
            ScreenPart::Scanlines => self.scanlines.on,
            ScreenPart::Grain => self.grain.on,
            ScreenPart::Vignette => self.vignette.on,
            ScreenPart::Bloom => self.bloom.on,
        };
        on && self.strength(part) > 0.0
    }

    pub fn is_off(&self) -> bool {
        ScreenPart::ALL.into_iter().all(|part| !self.is_on(part))
    }

    pub fn validate(&self) -> Result<(), String> {
        let rules: [(bool, &str); 13] = [
            (
                within(self.curve.strength, 0.0, CURVE_MAX),
                "curve.strength is 0 to 0.25",
            ),
            (
                within(self.curve.edge, 0.0, EDGE_MAX),
                "curve.edge is 0 to 0.5",
            ),
            (
                within(self.scanlines.strength, 0.0, 1.0),
                "scanlines.strength is 0 to 1",
            ),
            (
                within(self.scanlines.period, PERIOD_MIN, LENGTH_MAX),
                "scanlines.period is 1 to 64 layout px",
            ),
            (
                within(self.grain.strength, 0.0, 1.0),
                "grain.strength is 0 to 1",
            ),
            (
                above(self.grain.size, 0.0, LENGTH_MAX),
                "grain.size is more than 0 and at most 64 layout px",
            ),
            (
                within(self.grain.response, 0.0, 1.0),
                "grain.response is 0 to 1",
            ),
            (
                within(self.vignette.strength, 0.0, 1.0),
                "vignette.strength is 0 to 1",
            ),
            (
                above(self.vignette.radius, 0.0, VIGNETTE_MAX),
                "vignette.radius is more than 0 and at most 4",
            ),
            (
                above(self.vignette.falloff, 0.0, VIGNETTE_MAX),
                "vignette.falloff is more than 0 and at most 4",
            ),
            (
                within(self.bloom.strength, 0.0, BLOOM_MAX),
                "bloom.strength is 0 to 4",
            ),
            (
                within(self.bloom.threshold, 0.0, 1.0),
                "bloom.threshold is 0 to 1",
            ),
            (
                above(self.bloom.radius, 0.0, LENGTH_MAX),
                "bloom.radius is more than 0 and at most 64 layout px",
            ),
        ];
        match rules.iter().find(|(ok, _)| !ok) {
            Some((_, rule)) => Err(format!("a screen look's {rule}")),
            None => Ok(()),
        }
    }

    pub fn grain_pass(&self) -> Option<Grain> {
        self.is_on(ScreenPart::Grain).then_some(Grain {
            amount: self.grain.strength,
            size: self.grain.size,
            response: self.grain.response,
        })
    }

    pub fn crt_pass(&self) -> Option<Crt> {
        let crt = Crt {
            scanlines: self.is_on(ScreenPart::Scanlines).then_some(Scanlines {
                period: self.scanlines.period,
                strength: self.scanlines.strength,
            }),
            glow: self.is_on(ScreenPart::Bloom).then_some(Glow {
                threshold: self.bloom.threshold,
                radius: self.bloom.radius,
                strength: self.bloom.strength,
                tint: [1.0; 4],
            }),
            curvature: self.is_on(ScreenPart::Curve).then_some(Curvature {
                amount: self.curve.strength,
                corner: self.curve.edge,
                bezel: BLACK,
            }),
            aberration: None,
        };
        (!crt.is_empty()).then_some(crt)
    }

    pub fn vignette_pass(&self, place: &Place) -> Option<Vignette> {
        let half = [place.layout[0] * 0.5, place.layout[1] * 0.5];
        self.is_on(ScreenPart::Vignette).then_some(Vignette {
            centre: half,
            radius: [
                half[0] * self.vignette.radius,
                half[1] * self.vignette.radius,
            ],
            falloff: self.vignette.falloff,
            floor: 0.0,
            colour: [0.0, 0.0, 0.0, self.vignette.strength],
        })
    }

    pub fn passes(&self) -> Vec<&'static str> {
        if self.is_off() {
            return Vec::new();
        }
        let mut passes = Vec::new();
        if self.grain_pass().is_some() {
            passes.push("look grain");
        }
        if let Some(crt) = self.crt_pass() {
            if crt.glow.is_some() {
                passes.extend(["look glow"; 3]);
            }
            passes.push("look crt");
        }
        if self.is_on(ScreenPart::Vignette) {
            passes.push("look vignette");
        }
        passes.push(PRESENT);
        passes
    }
}

pub const PRESENT: &str = "screen present";

pub const SCREEN_WGSL: &str = r#"
@group(0) @binding(0) var screen_source: texture_2d<f32>;

@vertex
fn screen_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(xy[index], 0.0, 1.0);
}

@fragment
fn screen_present(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return textureLoad(screen_source, vec2i(position.xy), 0);
}

@fragment
fn screen_present_decode(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let c = textureLoad(screen_source, vec2i(position.xy), 0);
    let x = max(c.rgb, vec3f(0.0));
    return vec4f(select(pow((x + 0.055) / 1.055, vec3f(2.4)), x / 12.92, x <= vec3f(0.04045)), c.a);
}
"#;

pub fn present_entry(format: wgpu::TextureFormat) -> &'static str {
    if format.is_srgb() {
        "screen_present_decode"
    } else {
        "screen_present"
    }
}

pub fn canvas_format(format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    format.remove_srgb_suffix()
}

pub fn present_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }]
}

pub struct Canvas {
    pub texture: wgpu::Texture,
    pub draw: wgpu::TextureView,
    pub read: wgpu::TextureView,
    pub size: [u32; 2],
    pub format: wgpu::TextureFormat,
}

impl Canvas {
    pub fn new(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> Self {
        let stored = canvas_format(format);
        let views = [format];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screen canvas"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: stored,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: if stored == format { &[] } else { &views },
        });
        let draw = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(format),
            ..Default::default()
        });
        let read = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(stored),
            ..Default::default()
        });
        Self {
            texture,
            draw,
            read,
            size,
            format,
        }
    }

    pub fn fits(&self, size: [u32; 2], format: wgpu::TextureFormat) -> bool {
        self.size == size && self.format == format
    }
}

pub struct ScreenGpu {
    look: LookGpu,
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    presents: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl ScreenGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("screen present"),
            entries: &present_entries(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("screen present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("screen present"),
            source: wgpu::ShaderSource::Wgsl(SCREEN_WGSL.into()),
        });
        Self {
            look: LookGpu::new(device, queue),
            module,
            layout,
            pipeline_layout,
            presents: Vec::new(),
        }
    }

    pub fn pipelines(&self) -> usize {
        self.look.pipelines() + self.presents.len()
    }

    pub fn warm(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.presents.iter().any(|(made, _)| *made == format) {
            return;
        }
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(PRESENT),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("screen_vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some(present_entry(format)),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        self.presents.push((format, pipeline));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: Surface<'_>,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        look: &ScreenLook,
        place: &Place,
        frame: u32,
        seed: u32,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<usize, String> {
        look.validate()?;
        if look.is_off() {
            return Ok(0);
        }
        self.warm(device, format);
        self.look.begin();
        let size = source.size;
        let mut current = source.view.clone();
        let mut slot = 0;
        let mut next = |look: &mut LookGpu| {
            let view = look.target(device, SLOTS[slot], size);
            slot ^= 1;
            view
        };
        if let Some(grain) = look.grain_pass() {
            let to = next(&mut self.look);
            self.look.grain(
                Surface {
                    view: &current,
                    size,
                },
                Surface { view: &to, size },
                &grain,
                place,
                frame,
                seed,
            );
            current = to;
        }
        if let Some(crt) = look.crt_pass() {
            let to = next(&mut self.look);
            self.look.crt(
                device,
                Surface {
                    view: &current,
                    size,
                },
                Surface { view: &to, size },
                &crt,
                place,
            );
            current = to;
        }
        if let Some(vignette) = look.vignette_pass(place) {
            let to = next(&mut self.look);
            self.look.vignette(
                Surface {
                    view: &current,
                    size,
                },
                Surface { view: &to, size },
                &vignette,
                place,
            );
            current = to;
        }
        let recorded = self.look.job_count() + 1;
        self.look
            .flush(device, queue, encoder, profiler.as_deref_mut());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(PRESENT),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&current),
            }],
        });
        let timing = profiler
            .as_deref_mut()
            .and_then(|profiler| profiler.pass(PRESENT));
        let writes = profiler
            .as_deref()
            .and_then(|profiler| profiler.render_writes(timing));
        let pipeline = &self
            .presents
            .iter()
            .find(|(made, _)| *made == format)
            .ok_or("the screen look has no present pipeline for this format")?
            .1;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some(PRESENT),
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
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
        Ok(recorded)
    }
}
