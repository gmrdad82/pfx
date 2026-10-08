use std::collections::BTreeMap;

use pfx_geom::{
    detail::detail_level,
    lod,
    mesh::{Bounds, Mesh},
    shapes::Shape,
};

use crate::{
    frame::{Camera, Frame, Instance, Matrix, MeshData, MeshHandle, transform},
    renderer::Renderer,
};

pub const LOD_WGSL: &str = r#"
fn lod_hash(pixel: vec2u, object_id: u32) -> f32 {
    var value = pixel.x * 1664525u + pixel.y * 1013904223u + object_id * 747796405u;
    value = (value ^ (value >> 16u)) * 2246822519u;
    value = (value ^ (value >> 13u)) * 3266489917u;
    return f32(value ^ (value >> 16u)) * (1.0 / 4294967296.0);
}

fn lod_keep(pixel: vec2u, object_id: u32, coverage: f32) -> bool {
    return lod_hash(pixel, object_id) < coverage;
}
"#;

const FIRST_BUCKET: i32 = -40;
const LAST_BUCKET: i32 = 40;
const ERROR_SAFETY: f32 = 1.25;

pub trait MeshUploader {
    fn upload_mesh(&mut self, data: MeshData<'_>) -> Result<MeshHandle, String>;
}

impl MeshUploader for Frame {
    fn upload_mesh(&mut self, data: MeshData<'_>) -> Result<MeshHandle, String> {
        Frame::upload_mesh(self, data)
    }
}

impl MeshUploader for Renderer {
    fn upload_mesh(&mut self, data: MeshData<'_>) -> Result<MeshHandle, String> {
        Renderer::upload_mesh(self, data)
    }
}

struct Level {
    mesh: Mesh,
    handle: Option<MeshHandle>,
    error: f32,
}

enum Source {
    Chain(Vec<Level>),
    Analytic {
        shape: Shape,
        levels: BTreeMap<i32, Level>,
        last_bucket: i32,
    },
}

struct Object {
    instance: Instance,
    bounds: Bounds,
    source: Source,
    selected: usize,
    previous: Option<usize>,
    distance: f32,
    ideal_triangles: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LodStats {
    pub triangles: usize,
    pub ideal_triangles: usize,
    pub degraded_objects: usize,
    pub over_budget_triangles: usize,
    pub cached_levels: usize,
}

pub struct LodSet {
    objects: Vec<Object>,
    triangle_budget: usize,
    stats: LodStats,
}

fn bucket_error(bucket: i32) -> f32 {
    2.0_f32.powf(bucket as f32 * 0.5)
}

fn level(mesh: Mesh, error: f32) -> Level {
    Level {
        mesh,
        handle: None,
        error,
    }
}

fn world_bounds(bounds: Bounds, model: Matrix) -> Bounds {
    let mut result = Bounds::empty();
    for x in [bounds.min[0], bounds.max[0]] {
        for y in [bounds.min[1], bounds.max[1]] {
            for z in [bounds.min[2], bounds.max[2]] {
                let point = transform(model, [x, y, z, 1.0]);
                result.include([point[0], point[1], point[2]]);
            }
        }
    }
    result
}

fn maximum_stretch(model: Matrix) -> f32 {
    let row = (0..3)
        .map(|row| (0..3).map(|column| model[column][row].abs()).sum::<f32>())
        .fold(0.0, f32::max);
    let column = (0..3)
        .map(|column| (0..3).map(|row| model[column][row].abs()).sum::<f32>())
        .fold(0.0, f32::max);
    (row * column).sqrt().max(1e-6)
}

impl Object {
    fn level(&self, index: usize) -> Option<&Level> {
        match &self.source {
            Source::Chain(levels) => levels.get(index),
            Source::Analytic { levels, .. } => levels.get(&(FIRST_BUCKET + index as i32)),
        }
    }

    fn level_mut(&mut self, index: usize) -> Option<&mut Level> {
        match &mut self.source {
            Source::Chain(levels) => levels.get_mut(index),
            Source::Analytic { levels, .. } => levels.get_mut(&(FIRST_BUCKET + index as i32)),
        }
    }

    fn ensure_level(&mut self, index: usize) {
        if let Source::Analytic { shape, levels, .. } = &mut self.source {
            let bucket = FIRST_BUCKET + index as i32;
            levels
                .entry(bucket)
                .or_insert_with(|| level(shape.mesh(bucket_error(bucket)), bucket_error(bucket)));
        }
    }

    fn max_index(&self) -> usize {
        match &self.source {
            Source::Chain(levels) => levels.len() - 1,
            Source::Analytic { last_bucket, .. } => (last_bucket - FIRST_BUCKET) as usize,
        }
    }

    fn triangle_count(&self) -> usize {
        self.level(self.selected)
            .map_or(0, |level| level.mesh.triangle_count())
    }
}

impl LodSet {
    pub fn new(triangle_budget: usize) -> Self {
        Self {
            objects: Vec::new(),
            triangle_budget,
            stats: LodStats::default(),
        }
    }

    pub fn add_mesh(&mut self, instance: Instance, mesh: &Mesh) -> Result<usize, String> {
        let levels = lod::chain(mesh)
            .into_iter()
            .map(|item| level(item.mesh, item.error))
            .collect::<Vec<_>>();
        if levels[0].mesh.triangle_count() == 0 {
            return Err("LOD mesh has no triangles".into());
        }
        let index = self.objects.len();
        self.objects.push(Object {
            instance,
            bounds: mesh.bounds,
            source: Source::Chain(levels),
            selected: 0,
            previous: None,
            distance: 0.0,
            ideal_triangles: 0,
        });
        Ok(index)
    }

    pub fn add_shape(&mut self, instance: Instance, shape: Shape) -> usize {
        let bounds = shape.mesh(0.05).bounds;
        let last_bucket = ((bounds.extent() * 0.1).max(1e-6).log2() * 2.0)
            .floor()
            .clamp(FIRST_BUCKET as f32, LAST_BUCKET as f32) as i32;
        let index = self.objects.len();
        self.objects.push(Object {
            instance,
            bounds,
            source: Source::Analytic {
                shape,
                levels: BTreeMap::new(),
                last_bucket,
            },
            selected: 0,
            previous: None,
            distance: 0.0,
            ideal_triangles: 0,
        });
        index
    }

    pub fn instance_mut(&mut self, index: usize) -> Option<&mut Instance> {
        self.objects
            .get_mut(index)
            .map(|object| &mut object.instance)
    }

    pub fn selected_level(&self, index: usize) -> Option<(usize, f32, usize)> {
        self.objects.get(index).and_then(|object| {
            object
                .level(object.selected)
                .map(|level| (object.selected, level.error, level.mesh.triangle_count()))
        })
    }

    pub fn selected_mesh(&self, index: usize) -> Option<&Mesh> {
        self.objects
            .get(index)
            .and_then(|object| object.level(object.selected))
            .map(|level| &level.mesh)
    }

    pub fn stats(&self) -> LodStats {
        self.stats
    }

    pub fn select(&mut self, camera: Camera, viewport_height: u32) -> LodStats {
        let fov_y = 2.0 * (1.0 / camera.projection[1][1].abs().max(1e-8)).atan();
        let mut stats = LodStats::default();
        for object in &mut self.objects {
            let bounds = world_bounds(object.bounds, object.instance.model);
            let center = bounds.center();
            let distance = center
                .iter()
                .zip(camera.position)
                .map(|(a, b)| (*a - b) * (*a - b))
                .sum::<f32>()
                .sqrt();
            object.distance = distance;
            let scale = maximum_stretch(object.instance.model);
            let index = match &object.source {
                Source::Chain(levels) => {
                    let errors = levels
                        .iter()
                        .map(|level| level.error * scale * ERROR_SAFETY)
                        .collect::<Vec<_>>();
                    detail_level(
                        bounds,
                        &errors,
                        distance,
                        fov_y,
                        viewport_height as f32,
                        object.previous,
                    )
                }
                Source::Analytic { last_bucket, .. } => {
                    let errors = (FIRST_BUCKET..=*last_bucket)
                        .map(|bucket| bucket_error(bucket) * scale * ERROR_SAFETY)
                        .collect::<Vec<_>>();
                    detail_level(
                        bounds,
                        &errors,
                        distance,
                        fov_y,
                        viewport_height as f32,
                        object.previous,
                    )
                }
            };
            object.selected = index;
            object.previous = Some(index);
            object.ensure_level(index);
            object.ideal_triangles = object.triangle_count();
            stats.ideal_triangles += object.ideal_triangles;
        }
        stats.triangles = stats.ideal_triangles;
        if stats.triangles > self.triangle_budget {
            let mut order = (0..self.objects.len()).collect::<Vec<_>>();
            order.sort_by(|&a, &b| {
                self.objects[b]
                    .distance
                    .total_cmp(&self.objects[a].distance)
                    .then(a.cmp(&b))
            });
            for index in order {
                let object = &mut self.objects[index];
                let before = object.selected;
                while stats.triangles > self.triangle_budget && object.selected < object.max_index()
                {
                    let old = object.triangle_count();
                    object.selected += 1;
                    object.ensure_level(object.selected);
                    let new = object.triangle_count();
                    stats.triangles = stats.triangles.saturating_sub(old).saturating_add(new);
                }
                stats.degraded_objects += usize::from(object.selected != before);
                if stats.triangles <= self.triangle_budget {
                    break;
                }
            }
        }
        stats.over_budget_triangles = stats.triangles.saturating_sub(self.triangle_budget);
        stats.cached_levels = self
            .objects
            .iter()
            .map(|object| match &object.source {
                Source::Chain(levels) => levels.len(),
                Source::Analytic { levels, .. } => levels.len(),
            })
            .sum();
        self.stats = stats;
        stats
    }

    pub fn upload_selected(
        &mut self,
        uploader: &mut impl MeshUploader,
    ) -> Result<Vec<Instance>, String> {
        let mut instances = Vec::with_capacity(self.objects.len());
        for object in &mut self.objects {
            let index = object.selected;
            let level = object
                .level_mut(index)
                .ok_or("LOD level was not selected")?;
            let handle = if let Some(handle) = level.handle {
                handle
            } else {
                let mesh = &level.mesh;
                let handle = uploader.upload_mesh(MeshData {
                    positions: &mesh.positions,
                    normals: &mesh.normals,
                    tangents: &mesh.tangents,
                    uvs: &mesh.uvs,
                    uvs1: None,
                    alpha: None,
                    indices: &mesh.indices,
                })?;
                level.handle = Some(handle);
                handle
            };
            let mut instance = object.instance;
            instance.mesh = handle;
            instances.push(instance);
        }
        Ok(instances)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_geom::detail::{SILHOUETTE_PIXELS, projected_pixels};
    use pfx_gpu::{Gpu, wgpu};
    use std::f32::consts::PI;

    fn identity() -> Matrix {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    fn camera(z: f32) -> Camera {
        let mut projection = identity();
        projection[1][1] = 1.0 / (0.75_f32 * 0.5).tan();
        Camera {
            view: identity(),
            projection,
            previous_view_projection: identity(),
            position: [0.0, 0.0, z],
        }
    }

    fn instance(z: f32, id: u32) -> Instance {
        let mut model = identity();
        model[3][2] = z;
        Instance::new(MeshHandle(0), model, 0, id)
    }

    fn slab() -> Shape {
        Shape::Slab {
            half_length: 2.8,
            radius: 1.8,
            height: 0.16,
            fillet: 0.12,
        }
    }

    #[test]
    fn close_slab_is_finer_than_far_slab() {
        let mut set = LodSet::new(2_000_000);
        set.add_shape(instance(0.0, 1), slab());
        set.add_shape(instance(-20.0, 2), slab());
        let stats = set.select(camera(5.0), 2160);
        let close = set.selected_level(0).unwrap();
        let far = set.selected_level(1).unwrap();
        assert!(close.0 < far.0);
        assert!(close.2 > far.2);
        assert_eq!(stats.degraded_objects, 0);
    }

    #[test]
    fn hysteresis_holds_level_across_threshold() {
        let mut set = LodSet::new(2_000_000);
        set.add_shape(instance(0.0, 1), slab());
        set.select(camera(5.0), 2160);
        let selected = set.selected_level(0).unwrap().0;
        let next_error = bucket_error(FIRST_BUCKET + selected as i32 + 1) * ERROR_SAFETY;
        let bound = world_bounds(set.objects[0].bounds, identity());
        let fov = 0.75_f32;
        let threshold = bound.center()[2]
            + bound.radius()
            + next_error * 2160.0 / (2.0 * (fov * 0.5).tan() * SILHOUETTE_PIXELS);
        set.select(camera(threshold * 0.99), 2160);
        let before = set.selected_level(0).unwrap().0;
        set.select(camera(threshold * 1.01), 2160);
        assert_eq!(set.selected_level(0).unwrap().0, before);
    }

    #[test]
    fn analytic_cache_reuses_error_bucket() {
        struct CountingUploader(u32);
        impl MeshUploader for CountingUploader {
            fn upload_mesh(&mut self, _data: MeshData<'_>) -> Result<MeshHandle, String> {
                let handle = MeshHandle(self.0);
                self.0 += 1;
                Ok(handle)
            }
        }
        let mut set = LodSet::new(2_000_000);
        set.add_shape(instance(0.0, 1), slab());
        let mut uploader = CountingUploader(0);
        let first = set.select(camera(5.0), 2160);
        let selected = set.selected_level(0).unwrap().0;
        set.upload_selected(&mut uploader).unwrap();
        let second = set.select(camera(5.01), 2160);
        set.upload_selected(&mut uploader).unwrap();
        assert_eq!(set.selected_level(0).unwrap().0, selected);
        assert_eq!(first.cached_levels, second.cached_levels);
        assert_eq!(second.cached_levels, 1);
        assert_eq!(uploader.0, 1);
    }

    #[test]
    fn budget_degrades_farther_instance_first() {
        let mut set = LodSet::new(0);
        set.add_shape(instance(0.0, 1), slab());
        set.add_shape(instance(-20.0, 2), slab());
        let stats = set.select(camera(5.0), 2160);
        assert!(stats.degraded_objects > 0);
        assert_eq!(set.objects[1].selected, set.objects[1].max_index());
        assert!(stats.triangles <= stats.ideal_triangles);
        assert_eq!(stats.over_budget_triangles, stats.triangles);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn bench_slab_4k_silhouette_under_half_pixel() {
        let width = 3840;
        let height = 2160;
        let mut set = LodSet::new(2_000_000);
        set.add_shape(instance(0.0, 1), slab());
        let stats = set.select(camera(5.0), height);
        assert_eq!(stats.degraded_objects, 0);
        let mesh = set.selected_mesh(0).unwrap().clone();
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let instances = set.upload_selected(&mut frame).unwrap();
        let (positions, indices, count) = frame.shadow_mesh(instances[0].mesh).unwrap();
        let target = frame
            .gpu
            .offscreen(width, height, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        let shader = frame.gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("LOD silhouette test"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{LOD_WGSL}\n@vertex fn vertex(@location(0) position: vec3f) -> @builtin(position) vec4f {{ let f = 1.0 / tan(0.375); return vec4f(position.x * f / (3840.0 / 2160.0), position.y * f, 0.0, 5.0 - position.z); }} @fragment fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {{ if !lod_keep(vec2u(position.xy), 1u, 1.0) {{ discard; }} return vec4f(1.0); }}"
                )
                .into(),
            ),
        });
        let layout = frame
            .gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("LOD silhouette layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });
        let pipeline = frame
            .gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("LOD silhouette pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target.format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            });
        let mut encoder =
            frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("LOD silhouette encoder"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("LOD silhouette"),
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
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_vertex_buffer(0, positions.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..count, 0, 0..1);
        }
        frame.gpu.queue.submit(Some(encoder.finish()));
        let pixels = frame.gpu.readback_rgba8(&target).unwrap();
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[0] > 0));
        assert_eq!(pixels[0], 0);
        assert!(pixels[((height / 2 * width + width / 2) * 4) as usize] > 0);
        let straight = 2.8 - 1.8;
        let mut maximum = 0.0_f32;
        for side in [-1.0_f32, 1.0] {
            let mut outer = mesh
                .positions
                .iter()
                .filter(|position| (position[2] - 0.08).abs() < 1e-5)
                .filter(|position| position[0] * side >= straight - 1e-5)
                .map(|position| (position[1].atan2(position[0] * side - straight), *position))
                .collect::<Vec<_>>();
            outer.sort_by(|a, b| a.0.total_cmp(&b.0));
            assert!(outer.len() > 4);
            for pair in outer.windows(2) {
                if pair[1].0 - pair[0].0 > PI * 0.5 {
                    continue;
                }
                let midpoint = [
                    (pair[0].1[0] + pair[1].1[0]) * 0.5,
                    (pair[0].1[1] + pair[1].1[1]) * 0.5,
                ];
                let radial = ((midpoint[0] * side - straight).powi(2) + midpoint[1].powi(2)).sqrt();
                let error = (1.8 - radial).abs();
                maximum = maximum.max(projected_pixels(error, 5.0 - 0.08, 0.75, height as f32));
            }
        }
        println!("4K bench slab maximum silhouette deviation: {maximum:.4} px");
        assert!(maximum < 0.5, "{maximum} px");
    }
}
