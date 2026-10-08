use std::cell::Cell;
use std::path::Path;

use bytemuck::{Pod, Zeroable};
use pfx_bake::{
    AnchorBlend, Artifact, Grid, Manifest, blend_anchors, read_artifact, with_emitters,
};
use wgpu::util::DeviceExt;

pub const PROBE_CORNER_FLOOR: f32 = 0.05;
pub const PROBE_LOCALS: usize = 4;
pub const PROBE_WGSL: &str = r#"
override use_probes: bool = true;
struct ProbeLocal {
    min_spacing: vec4f,
    dims: vec4u,
}
struct ProbeGridInfo {
    min_spacing: vec4f,
    dims: vec4u,
    fallback: vec4f,
    blend: vec4f,
    shade: vec4f,
    locals: array<ProbeLocal, 4>,
}
@group(3) @binding(6) var<storage, read> probe_lower: array<u32>;
@group(3) @binding(7) var<storage, read> probe_upper: array<u32>;
@group(3) @binding(8) var<uniform> probe_grid: ProbeGridInfo;

fn probe_word(anchor: u32, index: u32) -> u32 {
    if (anchor == 0u) { return probe_lower[index]; }
    return probe_upper[index];
}

fn probe_block(anchor: u32, offset: u32) -> array<u32, 18> {
    var block: array<u32, 18>;
    if (anchor == 0u) {
        for (var k = 0u; k < 18u; k++) { block[k] = probe_lower[offset + k]; }
    } else {
        for (var k = 0u; k < 18u; k++) { block[k] = probe_upper[offset + k]; }
    }
    return block;
}

fn probe_index(xyz: vec3u, dims: vec4u) -> u32 {
    return (xyz.z * dims.y + xyz.y) * dims.x + xyz.x;
}

fn sample_probe_volume(position: vec3f, n: vec3f, point: vec3f, anchor: u32, origin: vec4f, dims: vec4u, words: u32, toward: vec3f, seen: ptr<function, f32>) -> vec4f {
    let last = vec3f(dims.xyz - vec3u(1u));
    let surface = (position - origin.xyz) / origin.w;
    let shifted = (point - origin.xyz) / origin.w;
    let surface_out = any(surface < vec3f(0.0)) || any(surface > last);
    let shifted_out = any(shifted < vec3f(0.0)) || any(shifted > last);
    if (surface_out && shifted_out) { return vec4f(0.0); }
    let coord = clamp(shifted, vec3f(0.0), last);
    let base = min(vec3u(floor(coord)), max(dims.xyz, vec3u(2u)) - vec3u(2u));
    var t = clamp(coord - vec3f(base), vec3f(0.0), vec3f(1.0));
    for (var axis = 0u; axis < 3u; axis++) {
        if (dims[axis] == 1u) { t[axis] = 0.0; }
    }
    let version_two = dims.w == 2u;
    var color = vec3f(0.0);
    var toward_luma = 0.0;
    var total = 0.0;
    for (var corner = 0u; corner < 8u; corner++) {
        var xyz = base;
        var weight = 1.0;
        for (var axis = 0u; axis < 3u; axis++) {
            let high = (corner & (1u << axis)) != 0u;
            if (high && dims[axis] > 1u) { xyz[axis] += 1u; }
            weight *= select(1.0 - t[axis], t[axis], high);
        }
        weight = max(weight - probe_grid.blend.z, 0.0);
        if (weight == 0.0) { continue; }
        let offset = words + probe_index(xyz, dims) * select(15u, 20u, version_two);
        var center = origin.xyz + vec3f(xyz) * origin.w;
        if (version_two) {
            let last = probe_word(anchor, offset + 19u);
            if (((last >> 16u) & 1u) == 0u) { continue; }
            center += vec3f(unpack2x16float(probe_word(anchor, offset + 18u)), unpack2x16float(last).x) * origin.w;
            let toward_probe = center - position + n * 0.0001;
            let facing = clamp((dot(toward_probe / max(length(toward_probe), 0.000001), n) + 1.0) * 0.5, 0.0, 1.0);
            weight *= facing * facing * facing;
            if (weight == 0.0) { continue; }
            let block = probe_block(anchor, offset);
            let toward_point = point - center;
            let distance = length(toward_point);
            let direction = toward_point / max(distance, 0.000001);
            var irradiance = vec3f(0.0);
            var toward_irradiance = 0.0;
            var mean = 0.0;
            var second = 0.0;
            for (var axis = 0u; axis < 3u; axis++) {
                let normal_component = n[axis];
                let up = normal_component >= 0.0;
                let rg = unpack2x16float(select(block[axis * 4u + 2u], block[axis * 4u], up));
                let b = unpack2x16float(select(block[axis * 4u + 3u], block[axis * 4u + 1u], up)).x;
                irradiance += vec3f(rg, b) * normal_component * normal_component;
                let toward_component = toward[axis];
                let toward_up = toward_component >= 0.0;
                let toward_rg = unpack2x16float(select(block[axis * 4u + 2u], block[axis * 4u], toward_up));
                let toward_b = unpack2x16float(select(block[axis * 4u + 3u], block[axis * 4u + 1u], toward_up)).x;
                toward_irradiance += dot(vec3f(toward_rg, toward_b), vec3f(0.2126, 0.7152, 0.0722)) * toward_component * toward_component;
                let ray_component = direction[axis];
                let pair = unpack2x16float(block[12u + axis]);
                let depth = select(pair.y, pair.x, ray_component >= 0.0);
                let ray_weight = ray_component * ray_component;
                let squared = unpack2x16float(block[15u + axis]);
                mean += ray_weight * depth * origin.w;
                second += ray_weight * select(squared.y, squared.x, ray_component >= 0.0) * origin.w * origin.w;
            }
            let variance = max(second - mean * mean, origin.w * origin.w * 0.01);
            let delta = max(distance - mean - origin.w * 0.1, 0.0);
            let chance = variance / (variance + delta * delta);
            weight *= chance * chance;
            color += irradiance * weight;
            toward_luma += toward_irradiance * weight;
            total += weight;
            continue;
        }
        let toward_probe = center - position + n * 0.0001;
        let facing = clamp((dot(toward_probe / max(length(toward_probe), 0.000001), n) + 1.0) * 0.5, 0.0, 1.0);
        weight *= facing * facing * facing;
        if (weight == 0.0) { continue; }
        let toward_point = point - center;
        let distance = length(toward_point);
        let direction = toward_point / max(distance, 0.000001);
        var irradiance = vec3f(0.0);
        var toward_irradiance = 0.0;
        var visibility = 0.0;
        for (var axis = 0u; axis < 3u; axis++) {
            let normal_component = n[axis];
            let normal_lobe = axis * 2u + select(1u, 0u, normal_component >= 0.0);
            let rg = unpack2x16float(probe_word(anchor, offset + normal_lobe * 2u));
            let b = unpack2x16float(probe_word(anchor, offset + normal_lobe * 2u + 1u)).x;
            irradiance += vec3f(rg, b) * normal_component * normal_component;
            let toward_component = toward[axis];
            if (toward_component != 0.0) {
                let toward_lobe = axis * 2u + select(1u, 0u, toward_component >= 0.0);
                let toward_rg = unpack2x16float(probe_word(anchor, offset + toward_lobe * 2u));
                let toward_b = unpack2x16float(probe_word(anchor, offset + toward_lobe * 2u + 1u)).x;
                toward_irradiance += dot(vec3f(toward_rg, toward_b), vec3f(0.2126, 0.7152, 0.0722)) * toward_component * toward_component;
            }
            let ray_component = direction[axis];
            let pair = unpack2x16float(probe_word(anchor, offset + 12u + axis));
            let depth = select(pair.y, pair.x, ray_component >= 0.0);
            let ray_weight = ray_component * ray_component;
            let reach = depth + origin.w * 0.25;
            visibility += ray_weight * clamp((reach - distance) / origin.w, 0.0, 1.0);
        }
        weight *= select(1.0, visibility, distance > 0.000001);
        color += irradiance * weight;
        toward_luma += toward_irradiance * weight;
        total += weight;
    }
    if (total <= 0.000001) { return vec4f(0.0); }
    *seen = toward_luma / total;
    return vec4f(color / total, 1.0);
}

fn probe_local_fade(point: vec3f, local: ProbeLocal) -> f32 {
    let far = local.min_spacing.xyz + vec3f(local.dims.xyz - vec3u(1u)) * local.min_spacing.w;
    let inside = min(point - local.min_spacing.xyz, far - point);
    return clamp(min(inside.x, min(inside.y, inside.z)) / local.min_spacing.w, 0.0, 1.0);
}

fn sample_probe_anchor(position: vec3f, normal: vec3f, anchor: u32) -> vec3f {
    var unused = 0.0;
    return sample_probe_anchor_toward(position, normal, vec3f(0.0), anchor, &unused);
}

fn sample_probe_anchor_toward(position: vec3f, normal: vec3f, toward: vec3f, anchor: u32, seen: ptr<function, f32>) -> vec3f {
    let normal_length = length(normal);
    if (normal_length < 0.000001 || normal_length != normal_length || normal_length > 1e20 || any(position != position) || any(abs(position) > vec3f(1e20))) { return probe_grid.fallback.xyz; }
    let n = normal / normal_length;
    let point = position + n * 0.006;
    var local = vec4f(0.0);
    var local_seen = 0.0;
    var fade = 0.0;
    let count = min(u32(probe_grid.blend.w), 4u);
    for (var k = 0u; k < count; k++) {
        let inner = probe_local_fade(point, probe_grid.locals[k]);
        if (inner <= 0.0) { continue; }
        let sampled = sample_probe_volume(position, n, point, anchor, probe_grid.locals[k].min_spacing, vec4u(probe_grid.locals[k].dims.xyz, 2u), probe_grid.locals[k].dims.w, toward, &local_seen);
        if (sampled.w > 0.0) {
            local = sampled;
            fade = inner;
            break;
        }
    }
    if (fade >= 1.0) {
        *seen = local_seen;
        return local.xyz;
    }
    var main_seen = 0.0;
    let main = sample_probe_volume(position, n, point, anchor, probe_grid.min_spacing, probe_grid.dims, 0u, toward, &main_seen);
    if (main.w <= 0.0) {
        if (fade > 0.0) {
            *seen = local_seen;
            return local.xyz;
        }
        return probe_grid.fallback.xyz;
    }
    if (fade <= 0.0) {
        *seen = main_seen;
        return main.xyz;
    }
    *seen = mix(main_seen, local_seen, fade);
    return mix(main.xyz, local.xyz, fade);
}

fn probe_reflection_occlusion(grid: vec3f, reference: vec3f) -> f32 {
    let threshold = probe_grid.shade.x;
    if (threshold <= 0.0 || grid.x < 0.0) { return 1.0; }
    let open = dot(reference, vec3f(0.2126, 0.7152, 0.0722)) * threshold;
    if (!(open > 0.000001)) { return 1.0; }
    return clamp(dot(grid, vec3f(0.2126, 0.7152, 0.0722)) / open, 0.0, 1.0);
}

fn sample_live_irradiance(position: vec3f, normal: vec3f) -> vec3f {
    if (!use_probes || probe_grid.blend.y < 0.5) { return probe_grid.fallback.xyz; }
    let lower = sample_probe_anchor(position, normal, 0u);
    if (probe_grid.blend.x <= 0.0) { return lower; }
    let upper = sample_probe_anchor(position, normal, 1u);
    return mix(lower, upper, probe_grid.blend.x);
}

fn sample_live_irradiance_toward(position: vec3f, normal: vec3f, toward: vec3f, seen: ptr<function, f32>) -> vec3f {
    if (!use_probes || probe_grid.blend.y < 0.5) { return probe_grid.fallback.xyz; }
    var lower_seen = -1.0;
    let lower = sample_probe_anchor_toward(position, normal, toward, 0u, &lower_seen);
    if (probe_grid.blend.x <= 0.0) {
        *seen = lower_seen;
        return lower;
    }
    var upper_seen = -1.0;
    let upper = sample_probe_anchor_toward(position, normal, toward, 1u, &upper_seen);
    if (lower_seen >= 0.0 && upper_seen >= 0.0) {
        *seen = mix(lower_seen, upper_seen, probe_grid.blend.x);
    }
    return mix(lower, upper, probe_grid.blend.x);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LocalUniform {
    min_spacing: [f32; 4],
    dims: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridUniform {
    min_spacing: [f32; 4],
    dims: [u32; 4],
    fallback: [f32; 4],
    blend: [f32; 4],
    shade: [f32; 4],
    locals: [LocalUniform; PROBE_LOCALS],
}

pub fn verified_artifact(path: &Path) -> Result<Artifact, String> {
    read_artifact(path)
}

const BAND_ZERO: f32 = 0.282095;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ambient {
    #[default]
    Sky,
    Constant([f32; 3]),
    Sh([[f32; 3]; 9]),
}

impl Ambient {
    pub fn valid(&self) -> bool {
        match self {
            Self::Sky => true,
            Self::Constant(irradiance) => irradiance.iter().all(|v| v.is_finite() && *v >= 0.0),
            Self::Sh(sh) => sh.iter().flatten().all(|v| v.is_finite()),
        }
    }

    pub fn sh(&self, sky: [[f32; 3]; 9]) -> [[f32; 3]; 9] {
        match self {
            Self::Sky => sky,
            Self::Constant(irradiance) => {
                let mut sh = [[0.0; 3]; 9];
                sh[0] = irradiance.map(|v| v / BAND_ZERO);
                sh
            }
            Self::Sh(sh) => *sh,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactDesc {
    pub size: [u32; 2],
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub tint: [f32; 3],
}

impl ContactDesc {
    fn valid(&self) -> bool {
        self.size.iter().all(|&n| n > 0)
            && self.min.iter().chain(&self.max).all(|v| v.is_finite())
            && self.max[0] > self.min[0]
            && self.max[1] > self.min[1]
            && self.tint.iter().all(|v| v.is_finite() && *v >= 0.0)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct ContactUniform {
    rect: [f32; 4],
    tint: [f32; 4],
}

impl ContactUniform {
    pub(crate) fn off() -> Self {
        Self::zeroed()
    }

    pub(crate) fn on(desc: &ContactDesc) -> Self {
        Self {
            rect: [
                desc.min[0],
                desc.min[1],
                1.0 / (desc.max[0] - desc.min[0]),
                1.0 / (desc.max[1] - desc.min[1]),
            ],
            tint: [desc.tint[0], desc.tint[1], desc.tint[2], 1.0],
        }
    }
}

pub struct ContactField {
    desc: ContactDesc,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl ContactField {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        desc: ContactDesc,
    ) -> Result<Self, String> {
        if !desc.valid() {
            return Err("a contact field needs a nonzero size, a finite box with min below max and a finite nonnegative tint".into());
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live contact field"),
            size: wgpu::Extent3d {
                width: desc.size[0],
                height: desc.size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let field = Self {
            view: texture.create_view(&Default::default()),
            texture,
            desc,
        };
        field.write(queue, &vec![1.0; (desc.size[0] * desc.size[1]) as usize])?;
        Ok(field)
    }

    pub fn desc(&self) -> &ContactDesc {
        &self.desc
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn write(&self, queue: &wgpu::Queue, values: &[f32]) -> Result<(), String> {
        let [width, height] = self.desc.size;
        if values.len() != (width * height) as usize {
            return Err(format!(
                "a {width}x{height} contact field takes {} values, not {}",
                width * height,
                values.len()
            ));
        }
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|v| half::f16::from_f32(v.clamp(0.0, 1.0)).to_le_bytes())
            .collect();
        queue.write_texture(
            self.texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 2),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }
}

pub struct ProbeLighting {
    pub manifest: Option<Manifest>,
    layout: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    buffers: Vec<wgpu::Buffer>,
    selected: Option<AnchorBlend>,
    data: GridUniform,
    floor: Cell<f32>,
    fading: Option<Fading>,
    emission: f32,
}

struct Fading {
    grids: Vec<Grid>,
    emitters: Grid,
    scales: Vec<f32>,
}

impl ProbeLighting {
    pub fn fallback(device: &wgpu::Device, queue: &wgpu::Queue, fallback: [f32; 3]) -> Self {
        let layout = Self::make_layout(device);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live probe uniform"),
            size: std::mem::size_of::<GridUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dummy = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live probe fallback"),
            contents: &[0; 60],
            usage: wgpu::BufferUsages::STORAGE,
        });
        let data = GridUniform {
            min_spacing: [0.0, 0.0, 0.0, 1.0],
            dims: [1, 1, 1, 0],
            fallback: [fallback[0], fallback[1], fallback[2], 0.0],
            blend: [0.0; 4],
            shade: [0.0, 1.0, 0.0, 0.0],
            locals: [LocalUniform::zeroed(); PROBE_LOCALS],
        };
        queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&data));
        let group = Self::make_group(device, &layout, &dummy, &dummy, &uniform);
        Self {
            manifest: None,
            layout,
            group,
            uniform,
            buffers: vec![dummy],
            selected: None,
            data,
            floor: Cell::new(PROBE_CORNER_FLOOR),
            fading: None,
            emission: 0.0,
        }
    }

    pub fn load(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
        hour: f32,
        fallback: [f32; 3],
    ) -> Result<Self, String> {
        Self::from_artifact(device, queue, &verified_artifact(path)?, hour, fallback)
    }

    pub fn from_artifact(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        artifact: &Artifact,
        hour: f32,
        fallback: [f32; 3],
    ) -> Result<Self, String> {
        Self::from_artifacts(device, queue, artifact, &[], hour, fallback)
    }

    pub fn from_artifacts(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        artifact: &Artifact,
        locals: &[Artifact],
        hour: f32,
        fallback: [f32; 3],
    ) -> Result<Self, String> {
        check_locals(artifact, locals)?;
        let mut lighting = Self::fallback(device, queue, fallback);
        lighting.data.min_spacing = [
            artifact.manifest.grid.min[0],
            artifact.manifest.grid.min[1],
            artifact.manifest.grid.min[2],
            artifact.manifest.grid.spacing,
        ];
        lighting.data.dims = [
            artifact.manifest.dimensions[0],
            artifact.manifest.dimensions[1],
            artifact.manifest.dimensions[2],
            artifact.manifest.schema_version,
        ];
        let mut words = (artifact.grids[0].bytes().len() - 64) / 4;
        for (slot, local) in lighting.data.locals.iter_mut().zip(locals) {
            let grid = &local.manifest;
            *slot = LocalUniform {
                min_spacing: [
                    grid.grid.min[0],
                    grid.grid.min[1],
                    grid.grid.min[2],
                    grid.grid.spacing,
                ],
                dims: [
                    grid.dimensions[0],
                    grid.dimensions[1],
                    grid.dimensions[2],
                    words as u32,
                ],
            };
            words += (local.grids[0].bytes().len() - 64) / 4;
        }
        lighting.data.blend[3] = locals.len() as f32;
        lighting.buffers = artifact
            .grids
            .iter()
            .enumerate()
            .map(|(anchor, grid)| {
                let mut bytes = grid.bytes().split_off(64);
                for local in locals {
                    bytes.extend_from_slice(&local.grids[anchor].bytes()[64..]);
                }
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("live probe anchor"),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                })
            })
            .collect();
        if let (Some(emitters), Some(entry)) = (&artifact.emitters, &artifact.manifest.emitters) {
            lighting.fading = Some(Fading {
                grids: artifact.grids.clone(),
                emitters: emitters.clone(),
                scales: entry.scales.clone(),
            });
            lighting.set_emission(queue, 0.0)?;
        }
        lighting.manifest = Some(artifact.manifest.clone());
        lighting.set_hour(device, queue, hour)?;
        Ok(lighting)
    }

    pub fn set_hour(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hour: f32,
    ) -> Result<(), String> {
        let Some(manifest) = &self.manifest else {
            return Err("no probe artifact is loaded".into());
        };
        let selection = blend_anchors(&manifest.anchors, hour)?;
        if self
            .selected
            .is_none_or(|old| old.lower != selection.lower || old.upper != selection.upper)
        {
            self.group = Self::make_group(
                device,
                &self.layout,
                &self.buffers[selection.lower],
                &self.buffers[selection.upper],
                &self.uniform,
            );
        }
        self.data.blend = [
            selection.upper_weight,
            1.0,
            self.floor.get(),
            self.data.blend[3],
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&self.data));
        self.selected = Some(selection);
        Ok(())
    }

    pub fn set_emission(&mut self, queue: &wgpu::Queue, scale: f32) -> Result<(), String> {
        if !scale.is_finite() || scale < 0.0 {
            return Err("emission scale must be finite and nonnegative".into());
        }
        if let Some(fading) = &self.fading {
            for ((grid, baked), buffer) in
                fading.grids.iter().zip(&fading.scales).zip(&self.buffers)
            {
                let faded = with_emitters(grid, &fading.emitters, scale - baked)?;
                queue.write_buffer(buffer, 0, &faded.bytes()[64..]);
            }
        }
        self.emission = scale;
        Ok(())
    }

    pub fn emission(&self) -> f32 {
        self.emission
    }

    pub fn has_emitters(&self) -> bool {
        self.fading.is_some()
    }

    pub fn set_fallback(&mut self, queue: &wgpu::Queue, fallback: [f32; 3]) {
        self.data.fallback = [fallback[0], fallback[1], fallback[2], 0.0];
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&self.data));
    }

    pub fn set_corner_floor(&self, queue: &wgpu::Queue, floor: f32) -> Result<(), String> {
        if !floor.is_finite() || !(0.0..=0.25).contains(&floor) {
            return Err("the probe corner floor must be between 0 and 0.25".into());
        }
        self.floor.set(floor);
        let blend = [
            self.data.blend[0],
            self.data.blend[1],
            floor,
            self.data.blend[3],
        ];
        queue.write_buffer(&self.uniform, 48, bytemuck::bytes_of(&blend));
        Ok(())
    }

    pub fn set_reflection_occlusion(
        &mut self,
        queue: &wgpu::Queue,
        threshold: f32,
    ) -> Result<(), String> {
        if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
            return Err("the reflection occlusion threshold must be between 0 and 1".into());
        }
        self.data.shade[0] = threshold;
        queue.write_buffer(&self.uniform, 64, bytemuck::bytes_of(&self.data.shade));
        Ok(())
    }

    pub fn reflection_occlusion(&self) -> f32 {
        self.data.shade[0]
    }

    pub fn set_sky_visibility(&mut self, queue: &wgpu::Queue, on: bool) {
        self.data.shade[1] = if on { 1.0 } else { 0.0 };
        queue.write_buffer(&self.uniform, 64, bytemuck::bytes_of(&self.data.shade));
    }

    pub fn sky_visibility(&self) -> bool {
        self.data.shade[1] > 0.0
    }

    pub fn corner_floor(&self) -> f32 {
        self.floor.get()
    }

    pub fn local_volumes(&self) -> usize {
        self.data.blend[3] as usize
    }

    pub fn live(&self) -> bool {
        let off = self.data.blend[1] < 0.5;
        !off
    }

    pub fn selection(&self) -> Option<AnchorBlend> {
        self.selected
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn group(&self) -> &wgpu::BindGroup {
        &self.group
    }

    pub fn bindings(&self) -> [&wgpu::Buffer; 3] {
        match self.selected {
            Some(selection) => [
                &self.buffers[selection.lower],
                &self.buffers[selection.upper],
                &self.uniform,
            ],
            None => [&self.buffers[0], &self.buffers[0], &self.uniform],
        }
    }

    fn make_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live probe layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    fn make_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        lower: &wgpu::Buffer,
        upper: &wgpu::Buffer,
        uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live probe binding"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: lower.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: upper.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }
}

fn check_locals(artifact: &Artifact, locals: &[Artifact]) -> Result<(), String> {
    if artifact.grids.is_empty() {
        return Err("a probe artifact needs at least one anchor".into());
    }
    if locals.is_empty() {
        return Ok(());
    }
    if locals.len() > PROBE_LOCALS {
        return Err(format!(
            "at most {PROBE_LOCALS} local probe volumes, not {}",
            locals.len()
        ));
    }
    if artifact.emitters.is_some() || locals.iter().any(|local| local.emitters.is_some()) {
        return Err("local probe volumes take no emitter layer".into());
    }
    for local in locals {
        if local.manifest.schema_version != 2 {
            return Err("a local probe volume needs schema 2, with depth moments".into());
        }
        if local.manifest.anchors != artifact.manifest.anchors
            || local.grids.len() != artifact.grids.len()
        {
            return Err("a local probe volume needs the main volume's anchors".into());
        }
        if local.manifest.dimensions.iter().any(|&count| count < 3) {
            return Err("a local probe volume needs at least three probes on every axis".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use half::f16;
    use pfx_bake::{Anchor, BakeScene, FileEntry, Grid, GridSpec, Probe, bake, write_artifact};
    use pfx_gpu::Gpu;
    use pfx_load::Sky;
    use pfx_materials::Material;
    use pfx_trace::bvh::Triangle;
    use pfx_trace::{Camera, Scene, Sun, Trace};
    use sha2::{Digest, Sha256};
    use std::fs;

    fn test_root(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(name)
    }

    fn digest(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn loader_rejects_bad_hash_and_version() {
        let path = test_root("live-probe-loader-test");
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
        fs::create_dir_all(&path).unwrap();
        let spec = GridSpec {
            min: [0.0; 3],
            max: [0.0; 3],
            spacing: 1.0,
        };
        let grid = Grid::new(spec, vec![Probe::default()]).unwrap();
        let bytes = grid.bytes();
        fs::write(path.join("probe-000.bin"), &bytes).unwrap();
        let mut manifest = Manifest {
            schema_version: 1,
            scene_hash: "test".into(),
            anchors: vec![12.0],
            sample_count: 1,
            seed: 0,
            grid: spec,
            dimensions: [1; 3],
            format: "ambient_cube_rgb16f_visibility_f16_le_v1".into(),
            color_space: "linear_rec709_irradiance".into(),
            coordinates: "right_handed_y_up_xyz_x_fastest".into(),
            files: vec![FileEntry {
                path: "probe-000.bin".into(),
                bytes: bytes.len() as u64,
                sha256: digest(&bytes),
            }],
            emitters: None,
        };
        let save = |manifest: &Manifest| {
            let json = serde_json::to_vec(manifest).unwrap();
            fs::write(path.join("manifest.json"), &json).unwrap();
            fs::write(path.join("manifest.sha256"), digest(&json)).unwrap();
        };
        save(&manifest);
        assert!(verified_artifact(&path).is_ok());
        let mut corrupt = bytes.clone();
        corrupt[64] ^= 1;
        fs::write(path.join("probe-000.bin"), &corrupt).unwrap();
        assert!(verified_artifact(&path).err().unwrap().contains("checksum"));
        fs::write(path.join("probe-000.bin"), &bytes).unwrap();
        manifest.schema_version = 2;
        save(&manifest);
        assert!(verified_artifact(&path).err().unwrap().contains("manifest"));
        fs::remove_dir_all(path).unwrap();
    }

    fn artifact(spacing: f32, count: u32, anchors: &[f32], version_two: bool) -> Artifact {
        let spec = GridSpec {
            min: [0.0; 3],
            max: [spacing * (count - 1) as f32; 3],
            spacing,
        };
        let probes = vec![Probe::default(); (count * count * count) as usize];
        let grid = if version_two {
            let extra = pfx_bake::ProbeExtra {
                second: [0.0; 6],
                offset: [0.0; 3],
                enabled: true,
                backfaces: 0,
            };
            Grid::with_extras(spec, probes.clone(), vec![extra; probes.len()]).unwrap()
        } else {
            Grid::new(spec, probes).unwrap()
        };
        Artifact {
            manifest: Manifest {
                schema_version: grid.version(),
                scene_hash: "test".into(),
                anchors: anchors.to_vec(),
                sample_count: 1,
                seed: 0,
                grid: spec,
                dimensions: [count; 3],
                format: String::new(),
                color_space: String::new(),
                coordinates: String::new(),
                files: Vec::new(),
                emitters: None,
            },
            grids: vec![grid; anchors.len()],
            emitters: None,
            rounds: None,
        }
    }

    #[test]
    fn local_volumes_need_the_main_anchors_and_depth_moments() {
        let main = artifact(0.05, 4, &[12.0, 16.0], true);
        let fine = || artifact(0.01, 3, &[12.0, 16.0], true);
        let local = fine();
        assert!(check_locals(&main, &[]).is_ok());
        assert!(check_locals(&main, std::slice::from_ref(&local)).is_ok());
        assert!(
            check_locals(
                &main,
                &(0..PROBE_LOCALS).map(|_| fine()).collect::<Vec<_>>()
            )
            .is_ok()
        );
        assert!(
            check_locals(
                &main,
                &(0..=PROBE_LOCALS).map(|_| fine()).collect::<Vec<_>>()
            )
            .unwrap_err()
            .contains("at most")
        );
        let other_hours = artifact(0.01, 3, &[12.0, 19.0], true);
        assert!(
            check_locals(&main, &[other_hours])
                .unwrap_err()
                .contains("anchors")
        );
        let flat = artifact(0.01, 3, &[12.0, 16.0], false);
        assert!(
            check_locals(&main, &[flat])
                .unwrap_err()
                .contains("schema 2")
        );
        let thin = artifact(0.01, 2, &[12.0, 16.0], true);
        assert!(
            check_locals(&main, &[thin])
                .unwrap_err()
                .contains("three probes")
        );
    }

    #[test]
    fn anchor_selection_tracks_live_hour() {
        let hours = [8.0, 12.0, 16.0];
        assert_eq!(blend_anchors(&hours, 10.0).unwrap().upper_weight, 0.5);
        assert_eq!(blend_anchors(&hours, 14.0).unwrap().lower, 1);
        assert_eq!(blend_anchors(&hours, 17.0).unwrap().upper, 2);
        assert!(blend_anchors(&hours, f32::NAN).is_err());
    }

    #[test]
    fn probe_shader_validates() {
        let source = format!(
            "{PROBE_WGSL}\n@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {{ var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0)); return vec4f(p[i], 0.0, 1.0); }}\n@fragment fn fs() -> @location(0) vec4f {{ return vec4f(sample_live_irradiance(vec3f(0.0), vec3f(0.0, 1.0, 0.0)), 1.0); }}"
        );
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    fn quad(
        triangles: &mut Vec<Triangle>,
        a: [f32; 3],
        b: [f32; 3],
        c: [f32; 3],
        d: [f32; 3],
        material: u32,
    ) {
        triangles.push(Triangle {
            vertices: [a, b, c],
            material,
        });
        triangles.push(Triangle {
            vertices: [a, c, d],
            material,
        });
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn sunlit_floor_agrees_with_path_trace() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut triangles = Vec::new();
        quad(
            &mut triangles,
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 0.0, 1.0],
            [-1.0, 0.0, 1.0],
            0,
        );
        quad(
            &mut triangles,
            [-1.0, 0.0, -1.0],
            [-1.0, 1.5, -1.0],
            [1.0, 1.5, -1.0],
            [1.0, 0.0, -1.0],
            1,
        );
        quad(
            &mut triangles,
            [1.0, 0.0, -1.0],
            [1.0, 1.5, -1.0],
            [1.0, 1.5, 1.0],
            [1.0, 0.0, 1.0],
            1,
        );
        quad(
            &mut triangles,
            [1.0, 0.0, 1.0],
            [1.0, 1.5, 1.0],
            [-1.0, 1.5, 1.0],
            [-1.0, 0.0, 1.0],
            1,
        );
        quad(
            &mut triangles,
            [-1.0, 0.0, 1.0],
            [-1.0, 1.5, 1.0],
            [-1.0, 1.5, -1.0],
            [-1.0, 0.0, -1.0],
            1,
        );
        let material = Material {
            base: [0.8; 3],
            specular: 0.0,
            roughness: 1.0,
            ..Material::default()
        };
        let wall = Material {
            base: [0.2; 3],
            specular: 0.0,
            roughness: 1.0,
            ..Material::default()
        };
        let anchor = Anchor {
            hour: 12.0,
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.5, 0.5, 0.5, 1.0]],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [1.0; 3],
                intensity: 0.1,
            },
        };
        let scene = BakeScene {
            triangles: triangles.clone(),
            shapes: Vec::new(),
            materials: vec![material, wall],
            anchors: vec![anchor.clone()],
        };
        let spec = GridSpec {
            min: [0.0, 0.01, 0.0],
            max: [0.0, 0.01, 0.0],
            spacing: 1.0,
        };
        let grids = bake(&gpu, &scene, spec, 16, 73).unwrap();
        let root = test_root("live-probe-gpu-test");
        let path = root.join("artifact");
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        write_artifact(
            root.parent().unwrap(),
            Path::new("live-probe-gpu-test/artifact"),
            &scene,
            &grids,
            16,
            73,
        )
        .unwrap();
        let lighting = ProbeLighting::load(&gpu.device, &gpu.queue, &path, 12.0, [0.0; 3]).unwrap();
        let empty = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[],
            });
        let empty_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &empty,
            entries: &[],
        });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&empty, &empty, &empty, lighting.layout()],
                push_constant_ranges: &[],
            });
        let source = format!(
            "{PROBE_WGSL}\n@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {{ var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0)); return vec4f(p[i], 0.0, 1.0); }}\n@fragment fn fs() -> @location(0) vec4f {{ let indirect = sample_live_irradiance(vec3f(0.0, 0.01, 0.0), vec3f(0.0, 1.0, 0.0)) * 0.8 / 3.14159265; return vec4f(indirect + vec3f(0.1 * 0.8 / 3.14159265), 1.0); }}"
        );
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });
        let target = gpu
            .offscreen(1, 1, wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &empty_group, &[]);
            pass.set_bind_group(1, &empty_group, &[]);
            pass.set_bind_group(2, &empty_group, &[]);
            pass.set_bind_group(3, lighting.group(), &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(encoder.finish()));
        let live = f16::from_bits(gpu.readback_rgba16(&target).unwrap()[0]).to_f32();
        let reference_scene = Scene {
            triangles,
            shapes: Vec::new(),
            materials: vec![material, scene.materials[1]],
            sky: anchor.sky,
            camera: Camera {
                origin: [0.0, 1.0, 0.0],
                forward: [0.0, -1.0, 0.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 0.0, 1.0],
            },
            sun: anchor.sun,
        };
        let mut trace = Trace::new(&gpu, &reference_scene, 1, 1).unwrap();
        trace.sample(&gpu, 4096, 74).unwrap();
        let bytes = trace.readback(&gpu).unwrap().color;
        let reference = f32::from_le_bytes(bytes[..4].try_into().unwrap());
        let error = (live - reference).abs() / reference.max(0.01);
        println!("sunlit floor live={live:.4} path={reference:.4} relative_error={error:.3}");
        assert!(
            error < 0.3,
            "live={live} reference={reference} error={error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn irradiance_pixel(gpu: &Gpu, lighting: &ProbeLighting) -> Vec<u16> {
        irradiance_at(gpu, lighting, [0.0, 0.01, 0.0], [0.0, 1.0, 0.0])
    }

    fn irradiance_at(
        gpu: &Gpu,
        lighting: &ProbeLighting,
        position: [f32; 3],
        normal: [f32; 3],
    ) -> Vec<u16> {
        let empty = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[],
            });
        let empty_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &empty,
            entries: &[],
        });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&empty, &empty, &empty, lighting.layout()],
                push_constant_ranges: &[],
            });
        let source = format!(
            "{PROBE_WGSL}\n@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {{ var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0)); return vec4f(p[i], 0.0, 1.0); }}\n@fragment fn fs() -> @location(0) vec4f {{ return vec4f(sample_live_irradiance(vec3f({:?}, {:?}, {:?}), vec3f({:?}, {:?}, {:?})), 1.0); }}",
            position[0], position[1], position[2], normal[0], normal[1], normal[2]
        );
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });
        let target = gpu
            .offscreen(1, 1, wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &empty_group, &[]);
            pass.set_bind_group(1, &empty_group, &[]);
            pass.set_bind_group(2, &empty_group, &[]);
            pass.set_bind_group(3, lighting.group(), &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(encoder.finish()));
        gpu.readback_rgba16(&target).unwrap()
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn from_artifact_lights_a_scene_as_load_does() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let anchors: Vec<Anchor> = [12.0, 18.0]
            .into_iter()
            .map(|hour| Anchor {
                hour,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[0.5, 0.5, 0.5, 1.0]],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 0.1,
                },
            })
            .collect();
        let scene = BakeScene {
            triangles: Vec::new(),
            shapes: Vec::new(),
            materials: vec![Material::default()],
            anchors,
        };
        let spec = GridSpec {
            min: [0.0, 0.01, 0.0],
            max: [0.0, 0.01, 0.0],
            spacing: 1.0,
        };
        let grid = |level: f32| {
            let mut probe = Probe::default();
            for (index, lobe) in probe.lobes.iter_mut().enumerate() {
                *lobe = [level * (index + 1) as f32, level, level * 0.5];
            }
            Grid::new(spec, vec![probe]).unwrap()
        };
        let grids = [grid(0.25), grid(0.75)];
        let emitters = pfx_bake::Emitters {
            grid: grid(0.125),
            scales: vec![0.0, 0.0],
            direct: false,
        };
        let root = test_root("live-probe-bytes-test");
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        pfx_bake::write_artifact_with_emitters(
            root.parent().unwrap(),
            Path::new("live-probe-bytes-test/artifact"),
            &scene,
            &grids,
            &emitters,
            4,
            3,
        )
        .unwrap();
        let path = root.join("artifact");
        let files: std::collections::HashMap<String, Vec<u8>> = fs::read_dir(&path)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect();
        let artifact = pfx_bake::read_artifact_from(&|name| files.get(name).cloned()).unwrap();
        let mut loaded =
            ProbeLighting::load(&gpu.device, &gpu.queue, &path, 15.0, [0.0; 3]).unwrap();
        let mut built =
            ProbeLighting::from_artifact(&gpu.device, &gpu.queue, &artifact, 15.0, [0.0; 3])
                .unwrap();
        let base = irradiance_pixel(&gpu, &loaded);
        assert!(f16::from_bits(base[0]).to_f32() > 0.0);
        assert_eq!(irradiance_pixel(&gpu, &built), base);
        for scale in [0.5, 1.0] {
            loaded.set_emission(&gpu.queue, scale).unwrap();
            built.set_emission(&gpu.queue, scale).unwrap();
            assert_eq!(
                irradiance_pixel(&gpu, &built),
                irradiance_pixel(&gpu, &loaded)
            );
        }
        assert_ne!(irradiance_pixel(&gpu, &built), base);
        fs::remove_dir_all(root).unwrap();
    }

    fn box_with_a_slot() -> Vec<Triangle> {
        let mut triangles = Vec::new();
        quad(
            &mut triangles,
            [-1.0, 0.0, -1.0],
            [-1.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 0.0, -1.0],
            0,
        );
        let (low, high, top) = (0.005, 0.045, 0.03);
        for (a, b) in [
            ([low, low], [high, low]),
            ([high, low], [high, high]),
            ([high, high], [low, high]),
            ([low, high], [low, low]),
        ] {
            quad(
                &mut triangles,
                [a[0], 0.0, a[1]],
                [b[0], 0.0, b[1]],
                [b[0], top, b[1]],
                [a[0], top, a[1]],
                0,
            );
        }
        quad(
            &mut triangles,
            [low, top, low],
            [low, top, high],
            [0.035, top, high],
            [0.035, top, low],
            0,
        );
        triangles
    }

    fn baked_artifact(
        gpu: &Gpu,
        scene: &BakeScene,
        spec: GridSpec,
        root: &Path,
        name: &str,
    ) -> Artifact {
        let grids = bake(gpu, scene, spec, 32, 11).unwrap();
        write_artifact(
            root.parent().unwrap(),
            &Path::new(root.file_name().unwrap()).join(name),
            scene,
            &grids,
            32,
            11,
        )
        .unwrap();
        verified_artifact(&root.join(name)).unwrap()
    }

    fn rgb(pixel: &[u16]) -> [f32; 3] {
        std::array::from_fn(|channel| f16::from_bits(pixel[channel]).to_f32())
    }

    fn luma(rgb: [f32; 3]) -> f32 {
        0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_local_volume_keeps_the_light_out_of_a_slotted_box() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let white = Material {
            base: [0.8; 3],
            specular: 0.0,
            roughness: 1.0,
            ..Material::default()
        };
        let scene = BakeScene {
            triangles: box_with_a_slot(),
            shapes: Vec::new(),
            materials: vec![white],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[1.0, 1.0, 1.0, 1.0]],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 0.0,
                },
            }],
        };
        let root = test_root("live-probe-slot-test");
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        let main = baked_artifact(
            &gpu,
            &scene,
            GridSpec {
                min: [-0.15, -0.025, -0.15],
                max: [0.15, 0.125, 0.15],
                spacing: 0.05,
            },
            &root,
            "main",
        );
        let local = baked_artifact(
            &gpu,
            &scene,
            GridSpec {
                min: [-0.01, -0.015, -0.01],
                max: [0.06, 0.045, 0.06],
                spacing: 0.01,
            },
            &root,
            "local",
        );
        let inside = [0.015, 0.0, 0.025];
        let up = [0.0, 1.0, 0.0];
        let traced = bake(
            &gpu,
            &scene,
            GridSpec {
                min: [inside[0], 0.002, inside[2]],
                max: [inside[0], 0.002, inside[2]],
                spacing: 0.01,
            },
            64,
            12,
        )
        .unwrap()[0]
            .probes[0]
            .lobes[2];
        let coarse =
            ProbeLighting::from_artifact(&gpu.device, &gpu.queue, &main, 12.0, [0.0; 3]).unwrap();
        let fine = ProbeLighting::from_artifacts(
            &gpu.device,
            &gpu.queue,
            &main,
            std::slice::from_ref(&local),
            12.0,
            [0.0; 3],
        )
        .unwrap();
        assert_eq!(fine.local_volumes(), 1);
        let leaked = luma(rgb(&irradiance_at(&gpu, &coarse, inside, up))) / luma(traced);
        let kept = luma(rgb(&irradiance_at(&gpu, &fine, inside, up))) / luma(traced);
        println!(
            "slotted box interior: traced {:.4}, main grid {leaked:.2}x, with the local volume {kept:.2}x",
            luma(traced)
        );
        assert!(leaked > 1.5, "the 5 cm grid alone reads {leaked}x");
        assert!(
            (1.0 / 1.5..1.5).contains(&kept),
            "the local volume reads {kept}x"
        );
        for (position, normal) in [
            ([-0.1, 0.0, -0.1], up),
            ([0.12, 0.05, 0.0], [-1.0, 0.0, 0.0]),
            ([0.0, 0.0, 0.1], up),
        ] {
            assert_eq!(
                irradiance_at(&gpu, &fine, position, normal),
                irradiance_at(&gpu, &coarse, position, normal),
                "{position:?} is outside the local volume"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
