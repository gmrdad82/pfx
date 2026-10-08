use wgpu::util::DeviceExt;

pub const MAX_RADIUS: f32 = 32.0;
pub const TILE: u32 = 16;
const REACH: i32 = 2;
const MAX_RINGS: u32 = 5;
const RING_SPACING: f32 = 6.0;
const RING_TURN: f32 = 2.399_963;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    pub focal_m: f32,
    pub sensor_m: f32,
    pub fstop: f32,
    pub aperture: f32,
    pub focus_m: f32,
    pub near_m: f32,
    pub near_blur: f32,
    pub far_m: f32,
    pub far_blur: f32,
    pub ground_m: Option<f32>,
}

impl Lens {
    pub fn enabled(self) -> bool {
        self.aperture > 0.0 && self.focus_m > self.focal_m && self.sensor_m > 0.0
    }

    pub fn sensor_for_view(focal_m: f32, focus_m: f32, tan_half_width: f32) -> f32 {
        2.0 * tan_half_width * focal_m * focus_m / (focus_m - focal_m).max(1.0e-6)
    }

    pub fn thin_lens(self, distance: f32, width: u32) -> f32 {
        if !self.enabled() || distance <= 0.0 {
            return 0.0;
        }
        let diameter = self.focal_m * self.aperture / self.fstop.max(0.01);
        let blur = diameter * self.focal_m / (self.focus_m - self.focal_m)
            * (distance - self.focus_m)
            / distance;
        blur * width as f32 / (2.0 * self.sensor_m.max(1.0e-4))
    }

    pub fn spreads(self, height: f32) -> bool {
        self.ground_m.is_none_or(|ground| height < ground)
    }

    pub fn opening(self, distance: f32, height: f32) -> f32 {
        let near = if self.spreads(height) {
            self.near_blur.max(0.0) * (self.near_m - distance).max(0.0) / self.near_m.max(0.001)
        } else {
            0.0
        };
        let far = self.far_blur.max(0.0) * (distance - self.far_m).max(0.0) / self.far_m.max(0.001);
        1.0 + near + far
    }

    pub fn circle_of_confusion(self, distance: f32, height: f32, width: u32) -> f32 {
        (self.thin_lens(distance, width) * self.opening(distance, height))
            .clamp(-MAX_RADIUS, MAX_RADIUS)
    }
}

pub fn height_plane(view: [[f32; 4]; 4], projection: [[f32; 4]; 4]) -> [f32; 4] {
    let row = [view[1][0], view[1][1], view[1][2]];
    let x = row[0] / projection[0][0];
    let y = row[1] / projection[1][1];
    let z = row[0] * projection[2][0] / projection[0][0]
        + row[1] * projection[2][1] / projection[1][1]
        - row[2];
    let w = -(row[0] * view[3][0] + row[1] * view[3][1] + row[2] * view[3][2]);
    [x, y, z, w]
}

fn slack(distance: f32) -> f32 {
    0.005 + 0.02 * distance
}

fn rings(radius: f32) -> u32 {
    ((radius / RING_SPACING).ceil() as u32).clamp(1, MAX_RINGS)
}

pub fn gather(
    image: &[[f32; 4]],
    depth: &[f32],
    heights: &[f32],
    width: usize,
    height: usize,
    lens: Lens,
) -> Vec<[f32; 4]> {
    let count = width * height;
    if !lens.enabled() || image.len() != count || depth.len() != count || heights.len() != count {
        return image.to_vec();
    }
    let coc: Vec<f32> = (0..count)
        .map(|i| lens.circle_of_confusion(depth[i], heights[i], width as u32))
        .collect();
    let tiles_x = width.div_ceil(TILE as usize);
    let tiles_y = height.div_ceil(TILE as usize);
    let mut tiles = vec![(0.0f32, f32::INFINITY); tiles_x * tiles_y];
    for y in 0..height {
        for x in 0..width {
            let tile = &mut tiles[(y / TILE as usize) * tiles_x + x / TILE as usize];
            tile.0 = tile.0.max(coc[y * width + x].abs());
            tile.1 = tile.1.min(depth[y * width + x]);
        }
    }
    let dilated: Vec<(f32, f32)> = (0..tiles_x * tiles_y)
        .map(|index| {
            let (tx, ty) = ((index % tiles_x) as i32, (index / tiles_x) as i32);
            let mut out = (0.0f32, f32::INFINITY);
            for dy in -REACH..=REACH {
                for dx in -REACH..=REACH {
                    let (x, y) = (tx + dx, ty + dy);
                    if x >= 0 && y >= 0 && (x as usize) < tiles_x && (y as usize) < tiles_y {
                        let tile = tiles[y as usize * tiles_x + x as usize];
                        out = (out.0.max(tile.0), out.1.min(tile.1));
                    }
                }
            }
            out
        })
        .collect();
    let mut out = image.to_vec();
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let d0 = depth[index];
            let c0 = coc[index];
            let surface = lens.spreads(heights[index]);
            let tile = dilated[(y / TILE as usize) * tiles_x + x / TILE as usize];
            let eps = slack(d0);
            let front = if tile.1 < d0 - eps { tile.0 } else { 0.0 };
            let radius = front
                .max(if surface { c0.abs() } else { 0.0 })
                .min(MAX_RADIUS);
            if radius < 1.0 {
                continue;
            }
            let ring_count = rings(radius);
            let share = radius * radius / (4 * ring_count * (ring_count + 1)) as f32;
            let centre = [x as f32 + 0.5, y as f32 + 0.5];
            let mut back = image[index];
            let mut back_weight = 1.0;
            let mut fore = [0.0f32; 4];
            let mut fore_weight = 0.0;
            for ring in 1..=ring_count {
                let reach = radius * ring as f32 / ring_count as f32;
                let taps = 8 * ring;
                for tap in 0..taps {
                    let angle =
                        RING_TURN * ring as f32 + std::f32::consts::TAU * tap as f32 / taps as f32;
                    let at = [
                        centre[0] + angle.cos() * reach,
                        centre[1] + angle.sin() * reach,
                    ];
                    let sx = (at[0].floor() as i64).clamp(0, width as i64 - 1) as usize;
                    let sy = (at[1].floor() as i64).clamp(0, height as i64 - 1) as usize;
                    let sample = sy * width + sx;
                    let ds = depth[sample];
                    let cs = coc[sample];
                    let colour = image[sample];
                    if ds < d0 - eps {
                        let cover = (cs.abs() - reach + 0.5).clamp(0.0, 1.0);
                        let weight = cover * share / (cs * cs).max(1.0);
                        for c in 0..4 {
                            fore[c] += colour[c] * weight;
                        }
                        fore_weight += weight;
                    } else if surface {
                        let limit = if ds <= d0 + eps {
                            c0.abs().min(cs.abs())
                        } else {
                            c0.abs()
                        };
                        let cover = (limit - reach + 0.5).clamp(0.0, 1.0);
                        for c in 0..4 {
                            back[c] += colour[c] * cover;
                        }
                        back_weight += cover;
                    }
                }
            }
            let alpha = fore_weight.min(1.0);
            for c in 0..4 {
                let behind = back[c] / back_weight;
                let ahead = fore[c] / fore_weight.max(1.0e-5);
                out[index][c] = behind + (ahead - behind) * alpha;
            }
        }
    }
    out
}

const COMMON: &str = r#"
struct Params {
    size: vec2u, tiles: vec2u,
    optics: vec4f, focus: vec4f, far: vec4f, height: vec4f,
}
@group(0) @binding(0) var<uniform> lens: Params;
fn world_height(pixel: vec2f, distance: f32) -> f32 {
    let ndc = vec2f(pixel.x / f32(lens.size.x) * 2.0 - 1.0, 1.0 - pixel.y / f32(lens.size.y) * 2.0);
    return dot(lens.height.xyz, vec3f(ndc.x * distance, ndc.y * distance, distance)) + lens.height.w;
}
fn spreads(height: f32) -> bool {
    return lens.far.z < 0.5 || height < lens.far.y;
}
fn coc(distance: f32, height: f32) -> f32 {
    let focal = lens.optics.x;
    let focus = lens.focus.x;
    if (distance <= 0.0 || focus <= focal) { return 0.0; }
    let diameter = focal * lens.optics.w / max(lens.optics.z, 0.01);
    let thin = diameter * focal / (focus - focal) * (distance - focus) / distance;
    let low = select(0.0, 1.0, spreads(height));
    let open = 1.0 + low * lens.focus.z * max(lens.focus.y - distance, 0.0) / max(lens.focus.y, 0.001)
        + lens.far.x * max(distance - lens.focus.w, 0.0) / max(lens.focus.w, 0.001);
    return clamp(thin * open * f32(lens.size.x) / (2.0 * max(lens.optics.y, 0.0001)), -lens.far.w, lens.far.w);
}
fn slack(distance: f32) -> f32 {
    return 0.005 + 0.02 * distance;
}
"#;

const TILE_WGSL: &str = r#"
@group(0) @binding(1) var depth: texture_2d<f32>;
@group(0) @binding(2) var tiles: texture_storage_2d<rgba16float, write>;
var<workgroup> spread: atomic<u32>;
var<workgroup> nearest: atomic<u32>;
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3u, @builtin(local_invocation_index) local: u32, @builtin(workgroup_id) group: vec3u) {
    if (local == 0u) {
        atomicStore(&spread, 0u);
        atomicStore(&nearest, bitcast<u32>(60000.0));
    }
    workgroupBarrier();
    if (all(id.xy < lens.size)) {
        let distance = textureLoad(depth, vec2i(id.xy), 0).x;
        let height = world_height(vec2f(id.xy) + vec2f(0.5), distance);
        atomicMax(&spread, bitcast<u32>(abs(coc(distance, height))));
        atomicMin(&nearest, bitcast<u32>(clamp(distance, 0.0, 60000.0)));
    }
    workgroupBarrier();
    if (local == 0u) {
        textureStore(tiles, vec2i(group.xy), vec4f(bitcast<f32>(atomicLoad(&spread)), bitcast<f32>(atomicLoad(&nearest)), 0.0, 0.0));
    }
}
"#;

const DILATE_WGSL: &str = r#"
@group(0) @binding(1) var tiles: texture_2d<f32>;
@group(0) @binding(2) var dilated: texture_storage_2d<rgba16float, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if (any(id.xy >= lens.tiles)) { return; }
    var spread = 0.0;
    var nearest = 60000.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let at = vec2i(id.xy) + vec2i(dx, dy);
            if (any(at < vec2i(0)) || any(at >= vec2i(lens.tiles))) { continue; }
            let tile = textureLoad(tiles, at, 0);
            spread = max(spread, tile.x);
            nearest = min(nearest, tile.y);
        }
    }
    textureStore(dilated, vec2i(id.xy), vec4f(spread, nearest, 0.0, 0.0));
}
"#;

const GATHER_WGSL: &str = r#"
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_2d<f32>;
@group(0) @binding(3) var dilated: texture_2d<f32>;
@group(0) @binding(4) var output_tex: texture_storage_2d<rgba16float, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if (any(id.xy >= lens.size)) { return; }
    let pixel = vec2i(id.xy);
    let sharp = textureLoad(source, pixel, 0);
    let d0 = textureLoad(depth, pixel, 0).x;
    let centre = vec2f(pixel) + vec2f(0.5);
    let c0 = coc(d0, world_height(centre, d0));
    let surface = spreads(world_height(centre, d0));
    let tile = textureLoad(dilated, pixel / 16, 0);
    let eps = slack(d0);
    let front = select(0.0, tile.x, tile.y < d0 - eps);
    let radius = min(max(front, select(0.0, abs(c0), surface)), lens.far.w);
    if (radius < 1.0) {
        textureStore(output_tex, pixel, sharp);
        return;
    }
    let rings = clamp(u32(ceil(radius / 6.0)), 1u, 5u);
    let share = radius * radius / f32(4u * rings * (rings + 1u));
    let last = vec2i(lens.size) - vec2i(1);
    var back = sharp;
    var back_weight = 1.0;
    var fore = vec4f(0.0);
    var fore_weight = 0.0;
    for (var ring = 1u; ring <= rings; ring++) {
        let reach = radius * f32(ring) / f32(rings);
        let taps = 8u * ring;
        for (var tap = 0u; tap < taps; tap++) {
            let angle = 2.399963 * f32(ring) + 6.2831853 * f32(tap) / f32(taps);
            let at = centre + vec2f(cos(angle), sin(angle)) * reach;
            let texel = clamp(vec2i(floor(at)), vec2i(0), last);
            let ds = textureLoad(depth, texel, 0).x;
            let cs = coc(ds, world_height(vec2f(texel) + vec2f(0.5), ds));
            let colour = textureLoad(source, texel, 0);
            if (ds < d0 - eps) {
                let cover = clamp(abs(cs) - reach + 0.5, 0.0, 1.0);
                let weight = cover * share / max(cs * cs, 1.0);
                fore += colour * weight;
                fore_weight += weight;
            } else if (surface) {
                let limit = select(abs(c0), min(abs(c0), abs(cs)), ds <= d0 + eps);
                let cover = clamp(limit - reach + 0.5, 0.0, 1.0);
                back += colour * cover;
                back_weight += cover;
            }
        }
    }
    let behind = back / back_weight;
    let ahead = fore / max(fore_weight, 0.00001);
    textureStore(output_tex, pixel, mix(behind, ahead, min(fore_weight, 1.0)));
}
"#;

fn shader(part: &str) -> String {
    format!("{COMMON}{part}")
}

fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn compute(
    device: &wgpu::Device,
    label: &'static str,
    source: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(shader(source).into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (layout, pipeline)
}

struct Targets {
    _output: wgpu::Texture,
    output: wgpu::TextureView,
    _tiles: wgpu::Texture,
    tiles: wgpu::TextureView,
    _dilated: wgpu::Texture,
    dilated: wgpu::TextureView,
}

impl Targets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = |label, width, height| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            (texture, view)
        };
        let (tiles_x, tiles_y) = (width.div_ceil(TILE), height.div_ceil(TILE));
        let (output_texture, output) = texture("depth of field target", width, height);
        let (tiles_texture, tiles) = texture("depth of field tiles", tiles_x, tiles_y);
        let (dilated_texture, dilated) = texture("depth of field dilated tiles", tiles_x, tiles_y);
        Self {
            _output: output_texture,
            output,
            _tiles: tiles_texture,
            tiles,
            _dilated: dilated_texture,
            dilated,
        }
    }
}

pub struct GpuDof {
    device: wgpu::Device,
    width: u32,
    height: u32,
    viewport: (u32, u32),
    targets: Targets,
    tile_layout: wgpu::BindGroupLayout,
    tile_pipeline: wgpu::ComputePipeline,
    dilate_layout: wgpu::BindGroupLayout,
    dilate_pipeline: wgpu::ComputePipeline,
    gather_layout: wgpu::BindGroupLayout,
    gather_pipeline: wgpu::ComputePipeline,
}

impl GpuDof {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (tile_layout, tile_pipeline) = compute(
            device,
            "depth of field tiles",
            TILE_WGSL,
            &[uniform_entry(), texture_entry(1), storage_entry(2)],
        );
        let (dilate_layout, dilate_pipeline) = compute(
            device,
            "depth of field dilate",
            DILATE_WGSL,
            &[uniform_entry(), texture_entry(1), storage_entry(2)],
        );
        let (gather_layout, gather_pipeline) = compute(
            device,
            "depth of field gather",
            GATHER_WGSL,
            &[
                uniform_entry(),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                storage_entry(4),
            ],
        );
        Self {
            device: device.clone(),
            width,
            height,
            viewport: (width, height),
            targets: Targets::new(device, width, height),
            tile_layout,
            tile_pipeline,
            dilate_layout,
            dilate_pipeline,
            gather_layout,
            gather_pipeline,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) != (self.width, self.height) {
            self.targets = Targets::new(&self.device, width, height);
            self.width = width;
            self.height = height;
        }
        self.viewport = (width, height);
    }

    pub fn capacity(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.viewport = crate::gpu::fit_viewport((width, height), (self.width, self.height))?;
        Ok(())
    }

    fn params(&self, lens: Lens, height: [f32; 4]) -> [f32; 20] {
        let (width, rows) = self.viewport;
        let (tiles_x, tiles_y) = (width.div_ceil(TILE), rows.div_ceil(TILE));
        [
            f32::from_bits(width),
            f32::from_bits(rows),
            f32::from_bits(tiles_x),
            f32::from_bits(tiles_y),
            lens.focal_m,
            lens.sensor_m,
            lens.fstop,
            lens.aperture,
            lens.focus_m,
            lens.near_m,
            lens.near_blur,
            lens.far_m,
            lens.far_blur,
            lens.ground_m.unwrap_or(0.0),
            if lens.ground_m.is_some() { 1.0 } else { 0.0 },
            MAX_RADIUS,
            height[0],
            height[1],
            height[2],
            height[3],
        ]
    }

    fn bind(
        &self,
        label: &'static str,
        layout: &wgpu::BindGroupLayout,
        resources: &[wgpu::BindingResource<'_>],
    ) -> wgpu::BindGroup {
        let entries: Vec<_> = resources
            .iter()
            .enumerate()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: resource.clone(),
            })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &entries,
        })
    }

    pub fn render<'a>(
        &'a self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        lens: Lens,
        height: [f32; 4],
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> &'a wgpu::TextureView {
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("depth of field lens"),
                contents: bytemuck::cast_slice(&self.params(lens, height)),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let targets = &self.targets;
        let tiles = self.bind(
            "depth of field tile bindings",
            &self.tile_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(depth),
                wgpu::BindingResource::TextureView(&targets.tiles),
            ],
        );
        let dilate = self.bind(
            "depth of field dilate bindings",
            &self.dilate_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(&targets.tiles),
                wgpu::BindingResource::TextureView(&targets.dilated),
            ],
        );
        let gather = self.bind(
            "depth of field gather bindings",
            &self.gather_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(source),
                wgpu::BindingResource::TextureView(depth),
                wgpu::BindingResource::TextureView(&targets.dilated),
                wgpu::BindingResource::TextureView(&targets.output),
            ],
        );
        let (width, rows) = self.viewport;
        let (tiles_x, tiles_y) = (width.div_ceil(TILE), rows.div_ceil(TILE));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("depth of field"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.tile_pipeline);
        pass.set_bind_group(0, &tiles, &[]);
        pass.dispatch_workgroups(tiles_x, tiles_y, 1);
        pass.set_pipeline(&self.dilate_pipeline);
        pass.set_bind_group(0, &dilate, &[]);
        pass.dispatch_workgroups(tiles_x.div_ceil(8), tiles_y.div_ceil(8), 1);
        pass.set_pipeline(&self.gather_pipeline);
        pass.set_bind_group(0, &gather, &[]);
        pass.dispatch_workgroups(width.div_ceil(8), rows.div_ceil(8), 1);
        &targets.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::tests::{CAPACITY, VIEWPORTS, framed, half};

    const TAN_HALF_WIDTH: f32 = 0.5;
    const FRAME: u32 = 800;

    fn lens() -> Lens {
        Lens {
            focal_m: 0.05,
            sensor_m: Lens::sensor_for_view(0.05, 1.0, TAN_HALF_WIDTH),
            fstop: 2.0,
            aperture: 0.5,
            focus_m: 1.0,
            near_m: 1.0,
            near_blur: 12.0,
            far_m: 1.5,
            far_blur: 24.0,
            ground_m: None,
        }
    }

    fn plain() -> Lens {
        Lens {
            near_blur: 0.0,
            far_blur: 0.0,
            ..lens()
        }
    }

    #[test]
    fn coc_matches_thin_lens_formula() {
        let lens = plain();
        for distance in [0.5_f32, 0.8, 1.5, 3.0] {
            let blur = (lens.focal_m * lens.aperture / lens.fstop) * lens.focal_m
                / (lens.focus_m - lens.focal_m)
                * (distance - lens.focus_m)
                / distance;
            let expected =
                (blur * FRAME as f32 / (2.0 * lens.sensor_m)).clamp(-MAX_RADIUS, MAX_RADIUS);
            let got = lens.circle_of_confusion(distance, 0.3, FRAME);
            assert!(
                (got - expected).abs() < 1e-4,
                "{distance}: {got} against {expected}"
            );
        }
    }

    #[test]
    fn coc_matches_the_lens_disc() {
        let lens = lens();
        let radius = lens.focal_m / (2.0 * lens.fstop) * lens.aperture;
        for (distance, height) in [(0.6_f32, 0.0_f32), (1.6, 0.4), (2.5, 0.4)] {
            let open = lens.opening(distance, height);
            let spread =
                radius * open * (distance - lens.focus_m).abs() / (distance * lens.focus_m);
            let expected = (spread * FRAME as f32 / (2.0 * TAN_HALF_WIDTH)).min(MAX_RADIUS);
            let got = lens.circle_of_confusion(distance, height, FRAME).abs();
            assert!(
                (got - expected).abs() < 1e-3,
                "{distance}: {got} against {expected}"
            );
        }
    }

    #[test]
    fn near_blur_is_only_on_the_ground() {
        let lens = Lens {
            ground_m: Some(0.1),
            ..lens()
        };
        assert!((lens.opening(0.5, 0.0) - (1.0 + 12.0 * 0.5 / 1.0)).abs() < 1e-5);
        assert_eq!(lens.opening(0.5, 0.2), 1.0);
        assert!((lens.opening(2.0, 0.2) - (1.0 + 24.0 * 0.5 / 1.5)).abs() < 1e-5);
    }

    #[test]
    fn focus_plane_stays_sharp() {
        assert_eq!(lens().circle_of_confusion(lens().focus_m, 0.0, FRAME), 0.0);
        let (width, height) = (48, 32);
        let image: Vec<_> = (0..width * height)
            .map(|i| {
                [
                    (i % 7) as f32 / 7.0,
                    (i % 5) as f32 / 5.0,
                    (i % 3) as f32,
                    1.0,
                ]
            })
            .collect();
        let depth = vec![lens().focus_m; width * height];
        let heights = vec![0.0; width * height];
        assert_eq!(
            gather(&image, &depth, &heights, width, height, lens()),
            image
        );
    }

    fn edge(near: f32, far: f32, lens: Lens) -> (Vec<[f32; 4]>, usize) {
        let (width, height) = (160, 48);
        let image: Vec<_> = (0..width * height)
            .map(|i| {
                if i % width < 80 {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                }
            })
            .collect();
        let depth: Vec<_> = (0..width * height)
            .map(|i| if i % width < 80 { near } else { far })
            .collect();
        let heights = vec![0.0; width * height];
        (gather(&image, &depth, &heights, width, height, lens), width)
    }

    #[test]
    fn sharp_foreground_takes_no_halo_from_a_blurred_background() {
        let (out, width) = edge(lens().focus_m, 3.0, lens());
        let row = 24 * width;
        for x in 0..80 {
            assert_eq!(out[row + x], [1.0, 0.0, 0.0, 1.0], "foreground at {x}");
        }
        for x in 80..160 {
            assert!(
                out[row + x][0] < 1e-6,
                "background at {x} took {:?}",
                out[row + x]
            );
        }
    }

    #[test]
    fn blurred_foreground_spreads_over_a_sharp_background_without_a_halo() {
        let wide = Lens {
            aperture: 4.0,
            ..lens()
        };
        let (out, width) = edge(0.5, lens().focus_m, wide);
        let row = 24 * width;
        let reach = wide.circle_of_confusion(0.6, 0.0, 160).abs();
        assert!(reach > 4.0);
        assert!(out[row + 81][0] > 0.2, "{:?}", out[row + 81]);
        for x in 0..159 {
            let here = out[row + x][0] + out[row + x][2];
            assert!((here - 1.0).abs() < 1e-4, "energy at {x}: {here}");
            assert!(
                out[row + x + 1][0] <= out[row + x][0] + 1e-4,
                "red rises at {x}"
            );
        }
        assert_eq!(out[row + 159], [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn blur_keeps_surface_detail_above_the_ground() {
        let lens = Lens {
            ground_m: Some(0.1),
            ..lens()
        };
        let (width, height) = (96, 64);
        let image: Vec<_> = (0..width * height)
            .map(|i| {
                if (i % width) % 4 < 2 {
                    [1.0; 4]
                } else {
                    [0.0, 0.0, 0.0, 1.0]
                }
            })
            .collect();
        let depth = vec![2.5; width * height];
        let wall = vec![0.4; width * height];
        assert_eq!(gather(&image, &depth, &wall, width, height, lens), image);
        let floor = vec![0.0; width * height];
        let blurred = gather(&image, &depth, &floor, width, height, lens);
        let row: Vec<f32> = (16..80).map(|x| blurred[32 * width + x][0]).collect();
        let (low, high) = row.iter().fold((f32::MAX, f32::MIN), |(low, high), &v| {
            (low.min(v), high.max(v))
        });
        assert!(
            high - low < 0.5,
            "the floor kept its stripes: {low}..{high}"
        );
    }

    #[test]
    fn height_plane_finds_the_world_height() {
        let (tan_x, tan_y, shift) = (0.5_f32, 0.3_f32, 0.1_f32);
        let eye = [0.0_f32, 1.0, 2.0];
        let view = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-eye[0], -eye[1], -eye[2], 1.0],
        ];
        let projection = [
            [1.0 / tan_x, 0.0, 0.0, 0.0],
            [0.0, 1.0 / tan_y, 0.0, 0.0],
            [0.0, -shift / tan_y, -1.0, -1.0],
            [0.0, 0.0, -0.01, 0.0],
        ];
        let plane = height_plane(view, projection);
        for (ndc, depth) in [([0.0_f32, -1.0_f32], 0.75_f32), ([0.5, 0.3], 1.2)] {
            let ray_y = ndc[1] * tan_y - shift;
            let expected = eye[1] + ray_y * depth;
            let got =
                plane[0] * ndc[0] * depth + plane[1] * ndc[1] * depth + plane[2] * depth + plane[3];
            assert!((got - expected).abs() < 1e-5, "{got} against {expected}");
        }
    }

    #[test]
    fn shaders_parse() {
        for part in [TILE_WGSL, DILATE_WGSL, GATHER_WGSL] {
            naga::front::wgsl::parse_str(&shader(part)).unwrap();
        }
    }

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

    fn colour(x: u32, y: u32) -> [f32; 4] {
        [
            ((x * 7 + y * 3) % 17) as f32 / 17.0,
            ((x / 6 + y / 4) % 5) as f32 / 4.0,
            ((x + 2 * y) % 11) as f32 / 10.0,
            1.0,
        ]
    }

    fn distance(x: u32, y: u32) -> [f32; 4] {
        let d = match (x / 13 + y / 9) % 3 {
            0 => 0.6,
            1 => lens().focus_m,
            _ => 2.5,
        };
        [d, 0.0, 0.0, 1.0]
    }

    struct Frame {
        source: pfx_gpu::OffscreenTarget,
        depth: pfx_gpu::OffscreenTarget,
    }

    fn frame(gpu: &pfx_gpu::Gpu, size: (u32, u32), active: (u32, u32)) -> Frame {
        let source = gpu.offscreen(size.0, size.1, FORMAT).unwrap();
        gpu.upload_rgba16(
            &source,
            &framed(size, active, colour, [400.0, 0.0, 900.0, 1.0]),
        )
        .unwrap();
        let depth = gpu.offscreen(size.0, size.1, FORMAT).unwrap();
        gpu.upload_rgba16(
            &depth,
            &framed(size, active, distance, [0.2, 0.0, 0.0, 1.0]),
        )
        .unwrap();
        Frame { source, depth }
    }

    fn render(gpu: &pfx_gpu::Gpu, dof: &GpuDof, frame: &Frame) -> Vec<u16> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("depth of field viewport test"),
            });
        dof.render(
            &mut encoder,
            &frame.source.view,
            &frame.depth.view,
            lens(),
            [0.0, 1.0, 0.0, -0.5],
            None,
        );
        gpu.queue.submit(Some(encoder.finish()));
        let (width, height) = dof.capacity();
        gpu.readback_rgba16(&pfx_gpu::OffscreenTarget {
            texture: dof.targets._output.clone(),
            view: dof.targets.output.clone(),
            format: FORMAT,
            width,
            height,
        })
        .unwrap()
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_viewport_dof_equals_dof_at_the_viewport_size() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let garbage = frame(&gpu, CAPACITY, (0, 0));
        let mut dof = GpuDof::new(&gpu.device, CAPACITY.0, CAPACITY.1);
        for viewport in VIEWPORTS {
            dof.set_viewport(CAPACITY.0, CAPACITY.1).unwrap();
            let before = render(&gpu, &dof, &garbage);
            dof.set_viewport(viewport.0, viewport.1).unwrap();
            assert_eq!(dof.viewport(), viewport);
            let got = render(&gpu, &dof, &frame(&gpu, CAPACITY, viewport));
            let reference = GpuDof::new(&gpu.device, viewport.0, viewport.1);
            let want = render(&gpu, &reference, &frame(&gpu, viewport, viewport));
            let mut blurred = 0;
            for y in 0..CAPACITY.1 {
                for x in 0..CAPACITY.0 {
                    let at = ((y * CAPACITY.0 + x) * 4) as usize;
                    if x >= viewport.0 || y >= viewport.1 {
                        assert_eq!(
                            got[at..at + 4],
                            before[at..at + 4],
                            "{viewport:?} wrote ({x},{y}) outside its viewport"
                        );
                        continue;
                    }
                    let there = ((y * viewport.0 + x) * 4) as usize;
                    assert_eq!(
                        got[at..at + 4],
                        want[there..there + 4],
                        "{viewport:?} ({x},{y}) must equal depth of field at its size exactly"
                    );
                    let sharp = colour(x, y).map(half);
                    if got[at..at + 4] != sharp {
                        blurred += 1;
                    }
                }
            }
            assert!(blurred > 1000, "{viewport:?} blurred only {blurred} pixels");
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_full_viewport_dof_is_byte_identical() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let whole = frame(&gpu, CAPACITY, CAPACITY);
        let fresh = GpuDof::new(&gpu.device, CAPACITY.0, CAPACITY.1);
        let want = render(&gpu, &fresh, &whole);
        let mut dof = GpuDof::new(&gpu.device, CAPACITY.0, CAPACITY.1);
        dof.set_viewport(VIEWPORTS[1].0, VIEWPORTS[1].1).unwrap();
        render(&gpu, &dof, &frame(&gpu, CAPACITY, (0, 0)));
        dof.set_viewport(CAPACITY.0, CAPACITY.1).unwrap();
        assert!(render(&gpu, &dof, &whole) == want);
        assert!(dof.set_viewport(0, 10).is_err());
        assert!(dof.set_viewport(161, 10).is_err());
        dof.set_viewport(50, 40).unwrap();
        dof.resize(CAPACITY.0, CAPACITY.1);
        assert_eq!(dof.viewport(), CAPACITY);
    }
}
