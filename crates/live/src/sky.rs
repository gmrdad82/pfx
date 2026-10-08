use bytemuck::{Pod, Zeroable};
#[cfg(test)]
use pfx_core::daylight::Daylight;
use pfx_core::daylight::{self, Probe};
pub use pfx_core::sky::AnalyticSky;
use pfx_core::sky::SKY_MODEL_WGSL;
use pfx_load::Sky as HdrSky;
use std::f32::consts::{PI, TAU};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::thread::JoinHandle;

const SH_SAMPLES: usize = 4096;

#[derive(Clone, Debug)]
pub enum SkySource {
    Analytic(AnalyticSky),
    Hdr(HdrSky),
}

impl SkySource {
    pub fn from_probes(hour: f64, skies: &[HdrSky], probes: &[Probe]) -> Result<Self, String> {
        let first = skies.first().ok_or("sky probes need at least one HDR")?;
        if skies.iter().any(|s| {
            s.width != first.width
                || s.height != first.height
                || s.texels.len() != first.texels.len()
        }) {
            return Err("sky probes need matching HDR dimensions".into());
        }
        let weights = daylight::probe_weights(hour, probes, skies.len());
        if weights.iter().sum::<f64>() <= 0.0 {
            return Err("sky probes have no valid index".into());
        }
        let mut texels = vec![[0.0; 4]; first.texels.len()];
        for (sky, weight) in skies.iter().zip(weights) {
            for (out, input) in texels.iter_mut().zip(&sky.texels) {
                for (out_channel, input_channel) in out[..3].iter_mut().zip(&input[..3]) {
                    *out_channel += *input_channel * weight as f32;
                }
            }
        }
        for pixel in &mut texels {
            pixel[3] = 1.0;
        }
        Ok(Self::Hdr(HdrSky {
            width: first.width,
            height: first.height,
            texels,
        }))
    }

    pub fn radiance(&self, direction: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Analytic(model) => model.radiance(direction),
            Self::Hdr(hdr) => {
                let d = unit(direction);
                let u = (d[0].atan2(-d[2]) / TAU + 0.5).rem_euclid(1.0);
                let v = d[1].clamp(-1.0, 1.0).acos() / PI;
                let x = u * hdr.width as f32 - 0.5;
                let y = v * hdr.height as f32 - 0.5;
                let x0 = x.floor();
                let y0 = y.floor();
                let tx = x - x0;
                let ty = y - y0;
                let at = |ix: f32, iy: f32| {
                    let col = (ix as i32).rem_euclid(hdr.width as i32) as usize;
                    let row = (iy as i32).clamp(0, hdr.height as i32 - 1) as usize;
                    hdr.texels[row * hdr.width as usize + col]
                };
                let a = at(x0, y0);
                let b = at(x0 + 1.0, y0);
                let c = at(x0, y0 + 1.0);
                let d = at(x0 + 1.0, y0 + 1.0);
                [0, 1, 2].map(|i| {
                    (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty)
                        + (c[i] * (1.0 - tx) + d[i] * tx) * ty
                })
            }
        }
    }

    fn diffuse(&self, direction: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Analytic(model) => model.diffuse(direction),
            Self::Hdr(_) => self.radiance(direction),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SkyLighting {
    pub sh: [[f32; 3]; 9],
    pub cube: Vec<Vec<[f32; 4]>>,
    pub edge: u32,
    pub mip_count: u32,
    source: SkySource,
    sh_next: usize,
    cube_next: u32,
}

impl SkyLighting {
    pub fn new(source: SkySource, edge: u32) -> Self {
        let edge = edge.max(1).next_power_of_two();
        let mip_count = edge.ilog2() + 1;
        let cube = (0..mip_count)
            .map(|mip| vec![[0.0; 4]; (6 * (edge >> mip).pow(2)) as usize])
            .collect();
        Self {
            sh: [[0.0; 3]; 9],
            cube,
            edge,
            mip_count,
            source,
            sh_next: 0,
            cube_next: 0,
        }
    }

    pub fn set_source(&mut self, source: SkySource) {
        self.source = source;
        self.sh = [[0.0; 3]; 9];
        self.sh_next = 0;
        self.cube_next = 0;
    }

    pub fn source(&self) -> &SkySource {
        &self.source
    }

    pub fn step(&mut self) -> Option<(u32, u32)> {
        if self.sh_next < SH_SAMPLES {
            self.sh_chunk();
        }
        if self.cube_next >= self.mip_count * 6 {
            return None;
        }
        let mip = self.cube_next / 6;
        let face = self.cube_next % 6;
        let texels = self.face(mip, face);
        let size = self.edge >> mip;
        let at = (face * size * size) as usize;
        self.cube[mip as usize][at..at + texels.len()].copy_from_slice(&texels);
        self.cube_next += 1;
        Some((mip, face))
    }

    pub fn settle(&mut self) {
        while self.sh_next < SH_SAMPLES {
            self.sh_chunk();
        }
        let first = self.cube_next;
        let mip_count = self.mip_count;
        let edge = self.edge;
        let lighting = &*self;
        let faces: Vec<Vec<(u32, Vec<[f32; 4]>)>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..6)
                .map(|face| {
                    scope.spawn(move || {
                        (0..mip_count)
                            .filter(|mip| mip * 6 + face >= first)
                            .map(|mip| (mip, lighting.face(mip, face)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().expect("sky face worker"))
                .collect()
        });
        for (face, mips) in faces.into_iter().enumerate() {
            for (mip, texels) in mips {
                let size = edge >> mip;
                let at = face * (size * size) as usize;
                self.cube[mip as usize][at..at + texels.len()].copy_from_slice(&texels);
            }
        }
        self.cube_next = self.mip_count * 6;
    }

    fn sh_chunk(&mut self) {
        for index in self.sh_next..(self.sh_next + 256).min(SH_SAMPLES) {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / SH_SAMPLES as f32;
            let angle = index as f32 * 2.399_963_1;
            let radius = (1.0 - z * z).sqrt();
            let d = [radius * angle.cos(), z, radius * angle.sin()];
            let color = self.source.diffuse(d);
            let basis = sh_basis(d);
            let scale = 4.0 * PI / SH_SAMPLES as f32;
            for (l, basis_value) in basis.iter().enumerate() {
                let convolution = if l == 0 {
                    PI
                } else if l < 4 {
                    2.0 * PI / 3.0
                } else {
                    PI / 4.0
                };
                for (coefficient, value) in self.sh[l].iter_mut().zip(color) {
                    *coefficient += value * basis_value * scale * convolution;
                }
            }
        }
        self.sh_next = (self.sh_next + 256).min(SH_SAMPLES);
    }

    fn face(&self, mip: u32, face: u32) -> Vec<[f32; 4]> {
        let size = self.edge >> mip;
        let roughness = mip as f32 / (self.mip_count - 1).max(1) as f32;
        let mut texels = Vec::with_capacity((size * size) as usize);
        for y in 0..size {
            for x in 0..size {
                let n = cube_direction(
                    face,
                    (x as f32 + 0.5) / size as f32,
                    (y as f32 + 0.5) / size as f32,
                );
                let color = prefilter(&self.source, n, roughness);
                texels.push([color[0], color[1], color[2], 1.0]);
            }
        }
        texels
    }

    pub fn complete(&self) -> bool {
        self.sh_next == SH_SAMPLES && self.cube_next == self.mip_count * 6
    }

    pub fn irradiance(&self, normal: [f32; 3]) -> [f32; 3] {
        let basis = sh_basis(unit(normal));
        [0, 1, 2].map(|channel| {
            (0..9)
                .map(|i| self.sh[i][channel] * basis[i])
                .sum::<f32>()
                .max(0.0)
        })
    }
}

pub struct SettledSky {
    pub request: u64,
    pub lighting: SkyLighting,
    textures: Option<(wgpu::Texture, wgpu::Texture)>,
}

struct SettlerState {
    request: Option<(u64, SkySource)>,
    result: Option<SettledSky>,
    busy: bool,
    closed: bool,
    built: u64,
}

struct SettlerShared {
    state: Mutex<SettlerState>,
    wake: Condvar,
}

impl SettlerShared {
    fn lock(&self) -> MutexGuard<'_, SettlerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct SkySettler {
    edge: u32,
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    shared: Arc<SettlerShared>,
    worker: Option<JoinHandle<()>>,
}

impl SkySettler {
    pub fn new(edge: u32, gpu: Option<(wgpu::Device, wgpu::Queue)>) -> Self {
        Self {
            edge,
            gpu,
            shared: Arc::new(SettlerShared {
                state: Mutex::new(SettlerState {
                    request: None,
                    result: None,
                    busy: false,
                    closed: false,
                    built: 0,
                }),
                wake: Condvar::new(),
            }),
            worker: None,
        }
    }

    pub fn request(&mut self, request: u64, source: SkySource) {
        if self.worker.is_none() {
            let shared = self.shared.clone();
            let edge = self.edge;
            let gpu = self.gpu.clone();
            self.worker = Some(
                std::thread::Builder::new()
                    .name("pito sky settle".into())
                    .spawn(move || settle_worker(&shared, edge, gpu))
                    .expect("spawn the sky settle thread"),
            );
        }
        self.shared.lock().request = Some((request, source));
        self.shared.wake.notify_all();
    }

    pub fn cancel(&self) {
        let mut state = self.shared.lock();
        state.request = None;
        state.result = None;
    }

    pub fn take(&self) -> Option<SettledSky> {
        self.shared.lock().result.take()
    }

    pub fn idle(&self) -> bool {
        let state = self.shared.lock();
        state.request.is_none() && !state.busy
    }

    pub fn built(&self) -> u64 {
        self.shared.lock().built
    }

    pub fn wait(&self) {
        let mut state = self.shared.lock();
        while state.request.is_some() || state.busy {
            state = self
                .shared
                .wake
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

impl Drop for SkySettler {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn settle_worker(shared: &SettlerShared, edge: u32, gpu: Option<(wgpu::Device, wgpu::Queue)>) {
    loop {
        let (request, source) = {
            let mut state = shared.lock();
            loop {
                if state.closed {
                    return;
                }
                if let Some(next) = state.request.take() {
                    state.busy = true;
                    break next;
                }
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        let mut lighting = SkyLighting::new(source, edge);
        lighting.settle();
        let textures = gpu.as_ref().map(|(device, queue)| {
            (
                sky_hdr(device, queue, &lighting.source),
                sky_cube(device, queue, &lighting),
            )
        });
        let mut state = shared.lock();
        state.busy = false;
        state.built += 1;
        if state
            .result
            .as_ref()
            .is_none_or(|result| result.request < request)
        {
            state.result = Some(SettledSky {
                request,
                lighting,
                textures,
            });
        }
        drop(state);
        shared.wake.notify_all();
    }
}

pub struct SkySteps {
    settler: SkySettler,
    pending: Option<(u64, SkySource)>,
    next: u64,
    shown: u64,
}

impl SkySteps {
    pub fn new(edge: u32, gpu: Option<(wgpu::Device, wgpu::Queue)>) -> Self {
        Self {
            settler: SkySettler::new(edge, gpu),
            pending: None,
            next: 0,
            shown: 0,
        }
    }

    pub fn request(&mut self, source: SkySource) -> u64 {
        self.next += 1;
        self.pending = Some((self.next, source));
        self.next
    }

    pub fn settled(&mut self) -> u64 {
        self.next += 1;
        self.shown = self.next;
        self.pending = None;
        self.settler.cancel();
        self.next
    }

    pub fn dispatch(&mut self) {
        if let Some((request, source)) = self.pending.take() {
            self.settler.request(request, source);
        }
    }

    pub fn take(&mut self) -> Option<SettledSky> {
        let settled = self.settler.take()?;
        if settled.request <= self.shown {
            return None;
        }
        self.shown = settled.request;
        Some(settled)
    }

    pub fn shown(&self) -> u64 {
        self.shown
    }

    pub fn requested(&self) -> u64 {
        self.next
    }

    pub fn settling(&self) -> bool {
        self.pending.is_some() || !self.settler.idle() || self.shown < self.next
    }

    pub fn built(&self) -> u64 {
        self.settler.built()
    }

    pub fn wait(&mut self) {
        self.dispatch();
        self.settler.wait();
    }
}

fn f16_bytes(texels: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(texels.len() * 8);
    for pixel in texels {
        for channel in pixel {
            bytes.extend_from_slice(&half::f16::from_f32(*channel).to_bits().to_le_bytes());
        }
    }
    bytes
}

fn hdr_texture(device: &wgpu::Device, source: &SkySource) -> wgpu::Texture {
    let (width, height) = match source {
        SkySource::Hdr(hdr) => (hdr.width, hdr.height),
        _ => (1, 1),
    };
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky HDR"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn sky_hdr(device: &wgpu::Device, queue: &wgpu::Queue, source: &SkySource) -> wgpu::Texture {
    let texture = hdr_texture(device, source);
    if let SkySource::Hdr(hdr) = source {
        queue.write_texture(
            texture.as_image_copy(),
            &f16_bytes(&hdr.texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8 * hdr.width),
                rows_per_image: Some(hdr.height),
            },
            texture.size(),
        );
    }
    texture
}

fn cube_texture(device: &wgpu::Device, lighting: &SkyLighting) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky GGX cube"),
        size: wgpu::Extent3d {
            width: lighting.edge,
            height: lighting.edge,
            depth_or_array_layers: 6,
        },
        mip_level_count: lighting.mip_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn sky_cube(device: &wgpu::Device, queue: &wgpu::Queue, lighting: &SkyLighting) -> wgpu::Texture {
    let texture = cube_texture(device, lighting);
    for (mip, texels) in lighting.cube.iter().enumerate() {
        let size = lighting.edge >> mip;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &f16_bytes(texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 8),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 6,
            },
        );
    }
    texture
}

fn sh_basis(d: [f32; 3]) -> [f32; 9] {
    [
        0.282095,
        0.488603 * d[1],
        0.488603 * d[2],
        0.488603 * d[0],
        1.092548 * d[0] * d[1],
        1.092548 * d[1] * d[2],
        0.315392 * (3.0 * d[2] * d[2] - 1.0),
        1.092548 * d[0] * d[2],
        0.546274 * (d[0] * d[0] - d[1] * d[1]),
    ]
}

fn prefilter(source: &SkySource, n: [f32; 3], roughness: f32) -> [f32; 3] {
    if roughness == 0.0 {
        return source.radiance(n);
    }
    let up = if n[1].abs() < 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let right = unit(cross(up, n));
    let up = cross(n, right);
    let alpha = roughness * roughness;
    let mut sum = [0.0; 3];
    let mut weight = 0.0;
    for i in 0..64_u32 {
        let xi = (i as f32 + 0.5) / 64.0;
        let phi = TAU * radical_inverse(i);
        let cos_theta = ((1.0 - xi) / (1.0 + (alpha * alpha - 1.0) * xi)).sqrt();
        let sin_theta = (1.0 - cos_theta * cos_theta).sqrt();
        let h = add(
            add(
                mul(right, sin_theta * phi.cos()),
                mul(up, sin_theta * phi.sin()),
            ),
            mul(n, cos_theta),
        );
        let l = unit(add(mul(h, 2.0 * dot(n, h)), mul(n, -1.0)));
        let ndotl = dot(n, l).max(0.0);
        if ndotl > 0.0 {
            let value = source.radiance(l);
            for (channel, incoming) in sum.iter_mut().zip(value) {
                *channel += incoming * ndotl;
            }
            weight += ndotl;
        }
    }
    sum.map(|v| v / weight.max(1e-6))
}

fn radical_inverse(value: u32) -> f32 {
    value.reverse_bits() as f32 * 2.328_306_4e-10
}

fn cube_direction(face: u32, u: f32, v: f32) -> [f32; 3] {
    let x = 2.0 * u - 1.0;
    let y = 1.0 - 2.0 * v;
    unit(match face {
        0 => [1.0, y, -x],
        1 => [-1.0, y, x],
        2 => [x, 1.0, -y],
        3 => [x, -1.0, y],
        4 => [x, y, 1.0],
        _ => [-x, y, -1.0],
    })
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: [f32; 3], s: f32) -> [f32; 3] {
    a.map(|v| v * s)
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f32; 3]) -> [f32; 3] {
    mul(a, 1.0 / dot(a, a).sqrt().max(1e-8))
}

pub static SKY_WGSL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{}\n{}",
        SKY_MODEL_WGSL.replace("@group(1)", "@group(0)"),
        SKY_PASS_WGSL
    )
});

const SKY_PASS_WGSL: &str = r#"
struct SkyVertex {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
};
@vertex
fn sky_vs(@builtin(vertex_index) index: u32) -> SkyVertex {
    var positions = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    var output: SkyVertex;
    output.position = vec4f(positions[index], 1.0, 1.0);
    output.uv = positions[index]*vec2f(0.5, -0.5)+vec2f(0.5, 0.5);
    return output;
}
@fragment
fn sky_fs(input: SkyVertex) -> @location(0) vec4f {
    let direction = normalize(sky.forward.xyz + sky.right.xyz*(input.uv.x*2.0-1.0) + sky.up.xyz*(1.0-input.uv.y*2.0));
    return vec4f(sky_radiance(direction), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyUniform {
    sun: [f32; 4],
    sun_colour_intensity: [f32; 4],
    ambient_turbidity_albedo: [f32; 4],
    albedo_mode: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

pub struct SkyPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    hdr: wgpu::Texture,
    sampler: wgpu::Sampler,
    cube: wgpu::Texture,
    device: wgpu::Device,
    steps: SkySteps,
    pub lighting: SkyLighting,
}

impl SkyPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: wgpu::TextureFormat,
        depth: Option<wgpu::TextureFormat>,
        source: SkySource,
        cube_edge: u32,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky shader"),
            source: wgpu::ShaderSource::Wgsl(SKY_WGSL.as_str().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky pass"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("sky_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: depth.map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("sky_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky uniform"),
            size: std::mem::size_of::<SkyUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let hdr = hdr_texture(device, &source);
        let bind = Self::bind(device, &layout, &uniform, &hdr, &sampler);
        let lighting = SkyLighting::new(source, cube_edge);
        let cube = cube_texture(device, &lighting);
        let steps = SkySteps::new(lighting.edge, Some((device.clone(), queue.clone())));
        let mut pass = Self {
            pipeline,
            layout,
            uniform,
            bind,
            hdr,
            sampler,
            cube,
            device: device.clone(),
            steps,
            lighting,
        };
        pass.set_source(device, queue, pass.lighting.source.clone());
        pass
    }

    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        hdr: &wgpu::Texture,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky binding"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &hdr.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, source: SkySource) {
        self.steps.settled();
        self.hdr = sky_hdr(device, queue, &source);
        self.bind = Self::bind(
            device,
            &self.layout,
            &self.uniform,
            &self.hdr,
            &self.sampler,
        );
        self.lighting.set_source(source);
    }

    pub fn request_source(&mut self, source: SkySource) -> u64 {
        self.steps.request(source)
    }

    pub fn swap_settled(&mut self) -> Option<u64> {
        self.steps.dispatch();
        let settled = self.steps.take()?;
        let (hdr, cube) = settled.textures?;
        self.hdr = hdr;
        self.cube = cube;
        self.lighting = settled.lighting;
        self.bind = Self::bind(
            &self.device,
            &self.layout,
            &self.uniform,
            &self.hdr,
            &self.sampler,
        );
        Some(settled.request)
    }

    pub fn settling(&self) -> bool {
        self.steps.settling()
    }

    pub fn shown(&self) -> u64 {
        self.steps.shown()
    }

    pub fn wait_settled(&mut self) -> Option<u64> {
        self.steps.wait();
        self.swap_settled()
    }

    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
    ) {
        self.swap_settled();
        let (model, mode) = match self.lighting.source() {
            SkySource::Analytic(model) => (*model, 0.0),
            SkySource::Hdr(_) => (
                AnalyticSky {
                    sun: [0.0, 1.0, 0.0],
                    sun_colour: [1.0; 3],
                    sun_intensity: 0.0,
                    ambient: 0.0,
                    turbidity: 2.0,
                    ground_albedo: [0.0; 3],
                },
                1.0,
            ),
        };
        let data = SkyUniform {
            sun: [model.sun[0], model.sun[1], model.sun[2], 0.0],
            sun_colour_intensity: [
                model.sun_colour[0],
                model.sun_colour[1],
                model.sun_colour[2],
                model.sun_intensity,
            ],
            ambient_turbidity_albedo: [
                model.ambient,
                model.turbidity,
                model.ground_albedo[0],
                model.ground_albedo[1],
            ],
            albedo_mode: [model.ground_albedo[2], mode, 0.0, 0.0],
            forward: [forward[0], forward[1], forward[2], 0.0],
            right: [right[0], right[1], right[2], 0.0],
            up: [up[0], up[1], up[2], 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&data));
        if let Some((mip, face)) = self.lighting.step() {
            let size = self.lighting.edge >> mip;
            let start = (face * size * size) as usize;
            let end = start + (size * size) as usize;
            let bytes = f16_bytes(&self.lighting.cube[mip as usize][start..end]);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.cube,
                    mip_level: mip,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: face,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 8),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    pub fn cube_view(&self) -> wgpu::TextureView {
        self.cube.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        })
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daylight(hour: f64) -> Daylight {
        Daylight {
            hour,
            day: 172.0,
            latitude: 45.0,
            heading: 180.0,
        }
    }

    fn model(hour: f64) -> AnalyticSky {
        AnalyticSky::new(
            daylight(hour),
            daylight::REFERENCE_HOUR,
            2.5,
            [0.2, 0.25, 0.2],
        )
    }

    #[test]
    fn sun_is_brightest_and_night_is_dark() {
        let noon = model(12.0);
        let near = noon.radiance(noon.sun);
        let away = noon.radiance([-noon.sun[0], noon.sun[1], -noon.sun[2]]);
        assert!(near[0] > away[0] * 100.0);
        let night = model(0.0);
        let above = [0.0, 1.0, 0.0];
        assert!(noon.radiance(above)[2] > night.radiance(above)[2] * 5.0);
    }

    #[test]
    fn sh_reconstructs_diffuse_irradiance() {
        let source = SkySource::Analytic(model(12.0));
        let mut lighting = SkyLighting::new(source.clone(), 2);
        while !lighting.complete() {
            lighting.step();
        }
        for normal in [[0.0, 1.0, 0.0], [0.8, 0.6, 0.0], [0.0, 0.5, 0.866_025_4]] {
            let mut reference = [0.0; 3];
            let count = 16_384;
            for i in 0..count {
                let y = 1.0 - 2.0 * (i as f32 + 0.5) / count as f32;
                let phi = i as f32 * 2.399_963_1;
                let r = (1.0 - y * y).sqrt();
                let dir = [r * phi.cos(), y, r * phi.sin()];
                let weight = dot(dir, normal).max(0.0) * 4.0 * PI / count as f32;
                let color = source.diffuse(dir);
                for (channel, incoming) in reference.iter_mut().zip(color) {
                    *channel += incoming * weight;
                }
            }
            let sh = lighting.irradiance(normal);
            for c in 0..3 {
                let error = (sh[c] - reference[c]).abs() / reference[c].max(0.01);
                assert!(
                    error < 0.18,
                    "{normal:?} channel {c}: {sh:?} versus {reference:?}"
                );
            }
        }
    }

    #[test]
    fn hdr_blend_and_ggx_cube() {
        let red = HdrSky {
            width: 2,
            height: 1,
            texels: vec![[2.0, 0.0, 0.0, 1.0]; 2],
        };
        let blue = HdrSky {
            width: 2,
            height: 1,
            texels: vec![[0.0, 0.0, 4.0, 1.0]; 2],
        };
        let probes = [
            Probe {
                hour: 6.0,
                index: 0,
            },
            Probe {
                hour: 18.0,
                index: 1,
            },
        ];
        let source = SkySource::from_probes(12.0, &[red, blue], &probes).unwrap();
        let color = source.radiance([0.0, 1.0, 0.0]);
        assert!((color[0] - 1.0).abs() < 1e-5 && (color[2] - 2.0).abs() < 1e-5);
        let mut lighting = SkyLighting::new(source, 4);
        while !lighting.complete() {
            lighting.step();
        }
        for mip in &lighting.cube {
            for texel in mip {
                assert!((texel[0] - 1.0).abs() < 1e-3 && (texel[2] - 2.0).abs() < 1e-3);
            }
        }
    }

    fn same_bits(a: &SkyLighting, b: &SkyLighting) -> bool {
        let sh = |l: &SkyLighting| {
            l.sh.iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        };
        let cube = |l: &SkyLighting| {
            l.cube
                .iter()
                .flatten()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        };
        a.complete() && b.complete() && sh(a) == sh(b) && cube(a) == cube(b)
    }

    fn stepped(source: SkySource, edge: u32) -> SkyLighting {
        let mut lighting = SkyLighting::new(source, edge);
        while !lighting.complete() {
            lighting.step();
        }
        lighting
    }

    fn banded_hdr() -> HdrSky {
        let (width, height) = (32, 16);
        HdrSky {
            width,
            height,
            texels: (0..width * height)
                .map(|i| {
                    let (x, y) = (i % width, i / width);
                    [
                        0.2 + x as f32 * 0.05,
                        0.1 + y as f32 * 0.3,
                        if (x + y) % 5 == 0 { 6.0 } else { 0.4 },
                        1.0,
                    ]
                })
                .collect(),
        }
    }

    #[test]
    fn settling_off_thread_equals_stepping_bit_for_bit() {
        for source in [
            SkySource::Analytic(model(9.5)),
            SkySource::Hdr(banded_hdr()),
        ] {
            let reference = stepped(source.clone(), 16);
            let mut settled = SkyLighting::new(source.clone(), 16);
            settled.settle();
            assert!(same_bits(&reference, &settled));
            let mut partly = SkyLighting::new(source.clone(), 16);
            for _ in 0..9 {
                partly.step();
            }
            partly.settle();
            assert!(same_bits(&reference, &partly));
            let mut steps = SkySteps::new(16, None);
            let request = steps.request(source);
            steps.wait();
            let result = steps.take().unwrap();
            assert_eq!(result.request, request);
            assert!(same_bits(&reference, &result.lighting));
            assert!(!steps.settling());
        }
    }

    #[test]
    fn twenty_requests_in_one_frame_build_only_the_last() {
        let mut steps = SkySteps::new(16, None);
        let mut calls = Vec::new();
        let mut last = 0;
        for index in 0..20 {
            let source = SkySource::Analytic(model(6.0 + index as f64 * 0.5));
            let started = std::time::Instant::now();
            last = steps.request(source);
            calls.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let started = std::time::Instant::now();
        steps.dispatch();
        let dispatch = started.elapsed().as_secs_f64() * 1000.0;
        assert!(steps.settling());
        steps.wait();
        let result = steps.take().unwrap();
        assert_eq!(result.request, last);
        assert_eq!(steps.built(), 1);
        assert!(steps.take().is_none());
        assert!(same_bits(
            &stepped(SkySource::Analytic(model(6.0 + 19.0 * 0.5)), 16),
            &result.lighting
        ));
        let mean = calls.iter().sum::<f64>() / calls.len() as f64;
        let most = calls.iter().copied().fold(0.0, f64::max);
        println!(
            "set_sky on the calling thread: mean {mean:.4} ms, max {most:.4} ms over 20 calls; dispatch {dispatch:.4} ms"
        );
        assert!(mean < 1.0);
        assert!(dispatch < 50.0);
    }

    #[test]
    fn a_scrubber_ends_on_its_newest_hour_and_old_results_never_return() {
        let mut steps = SkySteps::new(8, None);
        let mut newest = 0;
        for frame in 0..30 {
            newest = steps.request(SkySource::Analytic(model(8.0 + frame as f64 * 0.25)));
            steps.dispatch();
            if let Some(result) = steps.take() {
                assert!(result.request <= newest);
            }
        }
        steps.wait();
        let shown = steps.take().map(|result| result.request);
        assert!(shown == Some(newest) || steps.shown() == newest);
        assert!(steps.built() <= 30);
        let settled = steps.settled();
        steps.request(SkySource::Analytic(model(13.0)));
        steps.dispatch();
        steps.settled();
        steps.wait();
        assert!(steps.take().is_none());
        assert!(settled < steps.shown());
    }

    #[test]
    fn wgsl_parses() {
        let module = naga::front::wgsl::parse_str(&SKY_WGSL).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_sky_matches_cpu_at_morning_noon_and_evening() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky test target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky readback"),
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for hour in [8.0, 12.0, 18.0] {
            let started = std::time::Instant::now();
            let model = model(hour);
            let mut pass = SkyPass::new(
                &gpu.device,
                &gpu.queue,
                wgpu::TextureFormat::Rgba16Float,
                None,
                SkySource::Analytic(model),
                2,
            );
            for direction in [
                unit([0.5, 0.7, 0.3]),
                [0.0, 1.0, 0.0],
                unit([-0.6, 0.4, 0.6]),
            ] {
                pass.update(&gpu.queue, direction, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                {
                    let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("sky test"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.draw(&mut render);
                }
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(1),
                        },
                    },
                    texture.size(),
                );
                gpu.queue.submit(Some(encoder.finish()));
                let (send, receive) = std::sync::mpsc::channel();
                let slice = readback.slice(..);
                slice.map_async(wgpu::MapMode::Read, move |result| {
                    let _ = send.send(result);
                });
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                receive.recv().unwrap().unwrap();
                let mapped = slice.get_mapped_range();
                let actual = [0, 1, 2].map(|i| {
                    half::f16::from_bits(u16::from_le_bytes([mapped[2 * i], mapped[2 * i + 1]]))
                        .to_f32()
                });
                drop(mapped);
                readback.unmap();
                let expected = model.radiance(direction);
                for c in 0..3 {
                    assert!(
                        (actual[c] - expected[c]).abs() <= expected[c].max(0.01) * 0.04,
                        "{hour}: {actual:?} versus {expected:?}"
                    );
                }
                println!("sky GPU {hour:02.0}:00 {direction:?}: {actual:?}");
            }
            println!(
                "sky GPU {hour:02.0}:00 headless test wall time: {:.3} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
