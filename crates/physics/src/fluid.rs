use crate::math::{add2, clamp2, dot2, len2, scale2, sign, smoothstep, sub2};
use crate::shape::{
    DRAIN_TIME, KERNEL, Look, MAX_PARTICLES, MAX_TARGETS, Params, Particle, Pour, SLOTS, SPACING,
    SUBSTEP, Target, along, cells_for, poly6, region, rest_density, spawn, spiky,
};

#[path = "fluid_pipeline.rs"]
mod pipeline;
use pfx_gpu::GpuProfiler;
use pipeline::Pipeline;
#[path = "fluid_inspect.rs"]
mod inspect;

pub const SPH_WGSL: &str = include_str!("sph.wgsl");
pub const BLUR_WGSL: &str = include_str!("fluid_blur.wgsl");
pub const JFA_WGSL: &str = include_str!("fluid_jfa.wgsl");
pub const EDGE_WGSL: &str = include_str!("fluid_edge.wgsl");
pub const RAISE_WGSL: &str = include_str!("fluid_raise.wgsl");

const BLOCK: u32 = 256;
const SETTLE: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoothing {
    pub wide: f32,
    pub fine: f32,
    pub pool: f32,
}

impl Default for Smoothing {
    fn default() -> Self {
        Self {
            wide: 10.0,
            fine: 2.0,
            pool: 20.0,
        }
    }
}

impl Smoothing {
    pub fn residuals(self) -> [f32; 2] {
        [
            ((self.wide * self.wide - self.fine * self.fine).max(0.25)).sqrt(),
            ((self.pool * self.pool - self.fine * self.fine).max(0.25)).sqrt(),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wire {
    pub radius: f32,
    pub lift: f32,
    pub period: f32,
}

impl Default for Wire {
    fn default() -> Self {
        Self {
            radius: 2.0,
            lift: 4.0,
            period: 3.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    pub shape: [f32; 4],
    pub size: [f32; 4],
}

impl Default for Outline {
    fn default() -> Self {
        Self {
            shape: [8.0, 16.0, 0.0, 4.0],
            size: [6.0, 16.0, 0.3, 0.0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FluidState {
    pub live: u32,
    pub asleep: u32,
    pub bounds: Option<([f32; 2], [f32; 2])>,
    pub height_min: f32,
    pub height_max: f32,
    pub height_mean: f32,
    pub channel_histogram: [std::collections::BTreeMap<i32, u32>; 3],
    pub resting: bool,
    pub row: Option<Vec<f32>>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    grid: [f32; 4],
    table: [f32; 4],
    k: [f32; 4],
    misc: [f32; 4],
    apart: [f32; 4],
    tension: [f32; 4],
    clock: [f32; 4],
}

pub struct Reference {
    particles: Vec<Particle>,
    targets: Vec<Target>,
    table: [f32; 2],
    params: Params,
    rest: f32,
    clock: f32,
}

impl Reference {
    pub fn bead(center: [f32; 2], radius: f32, table: [f32; 2]) -> Self {
        let look = Look {
            tall: 1.0,
            active: 0.0,
            weight_b: 0.0,
            weight_a: 0.0,
        };
        let target = Target::bead(center, radius, &look);
        Self {
            particles: spawn(&target, 0, None, 1),
            targets: vec![target],
            table,
            params: Params::default(),
            rest: rest_density(),
            clock: 0.0,
        }
    }

    pub fn count(&self) -> usize {
        self.particles.iter().filter(|p| p.born >= 0.0).count()
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    pub fn step(&mut self, dt: f32, time: f32, calm: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.clock = (self.clock + dt).min(SUBSTEP * 4.0);
        let steps = (self.clock / SUBSTEP).floor() as usize;
        self.clock -= steps as f32 * SUBSTEP;
        for _ in 0..steps {
            step_once(
                &mut self.particles,
                &self.targets,
                self.table,
                &self.params,
                self.rest,
                time,
                calm,
            );
        }
    }
}

fn step_once(
    particles: &mut [Particle],
    targets: &[Target],
    table: [f32; 2],
    params: &Params,
    rest: f32,
    time: f32,
    calm: f32,
) {
    let cells = cells_for(table);
    let Binned {
        counts,
        starts,
        items,
    } = bin(particles, cells);
    let neighbors = Neighbors {
        counts: &counts,
        starts: &starts,
        items: &items,
        cells,
    };
    for i in 0..particles.len() {
        if particles[i].born < 0.0 {
            continue;
        }
        particles[i].rho = density(i, particles, &neighbors).max(rest * 0.05);
    }
    let accel: Vec<[f32; 2]> = (0..particles.len())
        .map(|i| {
            if particles[i].born < 0.0 {
                [0.0; 2]
            } else {
                force(i, particles, targets, &neighbors, params, time, calm)
            }
        })
        .collect();
    for (i, particle) in particles.iter_mut().enumerate() {
        if particle.born < 0.0 {
            continue;
        }
        let Some(target) = targets.get(particle.owner as usize) else {
            continue;
        };
        let mut vel = scale2(particle.vel, (-params.damping * SUBSTEP).exp());
        vel = add2(vel, scale2(accel[i], SUBSTEP));
        let speed = len2(vel);
        if speed > 900.0 {
            vel = scale2(vel, 900.0 / speed);
        }
        let pos = clamp2(
            add2(
                particle.pos,
                scale2(add2(vel, [target.carry[0], target.carry[1]]), SUBSTEP),
            ),
            [1.0, 1.0],
            [table[0] - 1.0, table[1] - 1.0],
        );
        let mut goal = if target.flow[3] > 0.5 {
            target.flow[0]
        } else {
            0.0
        };
        if target.kind() == 2 && target.carry[3] > 0.5 {
            let curve = along(target, pos);
            goal *= smoothstep(target.carry[2] + 0.05, target.carry[2] - 0.05, curve[0]);
        }
        let rate = params.life_rate * SUBSTEP;
        let life = (particle.life + (goal - particle.life).clamp(-rate, rate)).clamp(0.0, 4.0);
        particle.vel = vel;
        particle.pos = pos;
        particle.life = life;
    }
}

fn cell_of(p: [f32; 2], cells: [usize; 2]) -> [usize; 2] {
    [
        (p[0] / KERNEL).floor().clamp(0.0, cells[0] as f32 - 1.0) as usize,
        (p[1] / KERNEL).floor().clamp(0.0, cells[1] as f32 - 1.0) as usize,
    ]
}

fn binned_cell(particle: &Particle, cells: [usize; 2]) -> Option<usize> {
    if particle.born < 0.0 || particle.life <= 1e-3 {
        return None;
    }
    let c = cell_of(particle.pos, cells);
    Some(c[1] * cells[0] + c[0])
}

fn exclusive_scan(counts: &[u32]) -> Vec<u32> {
    let mut total = 0;
    counts
        .iter()
        .map(|count| {
            let start = total;
            total += count;
            start
        })
        .collect()
}

struct Binned {
    counts: Vec<u32>,
    starts: Vec<u32>,
    items: Vec<u32>,
}

fn bin(particles: &[Particle], cells: [usize; 2]) -> Binned {
    let mut counts = vec![0u32; cells[0] * cells[1]];
    for particle in particles {
        if let Some(id) = binned_cell(particle, cells) {
            counts[id] += 1;
        }
    }
    let starts = exclusive_scan(&counts);
    let mut items = vec![0u32; counts.iter().sum::<u32>() as usize];
    let mut next = starts.clone();
    for (i, particle) in particles.iter().enumerate() {
        if let Some(id) = binned_cell(particle, cells) {
            items[next[id] as usize] = i as u32;
            next[id] += 1;
        }
    }
    Binned {
        counts,
        starts,
        items,
    }
}

struct Neighbors<'a> {
    counts: &'a [u32],
    starts: &'a [u32],
    items: &'a [u32],
    cells: [usize; 2],
}

fn visit_cell(home: [usize; 2], span: i32, neighbors: &Neighbors, mut body: impl FnMut(usize)) {
    for dy in -span..=span {
        for dx in -span..=span {
            let c = [home[0] as i32 + dx, home[1] as i32 + dy];
            if c[0] < 0
                || c[1] < 0
                || c[0] >= neighbors.cells[0] as i32
                || c[1] >= neighbors.cells[1] as i32
            {
                continue;
            }
            let id = c[1] as usize * neighbors.cells[0] + c[0] as usize;
            let start = neighbors.starts[id] as usize;
            let n = neighbors.counts[id].min(SLOTS) as usize;
            for &item in &neighbors.items[start..start + n] {
                body(item as usize);
            }
        }
    }
}

fn density(i: usize, particles: &[Particle], neighbors: &Neighbors) -> f32 {
    let home = cell_of(particles[i].pos, neighbors.cells);
    let mut rho = 0.0;
    visit_cell(home, 1, neighbors, |j| {
        let d = sub2(particles[i].pos, particles[j].pos);
        rho += particles[j].life * poly6(dot2(d, d));
    });
    rho
}

fn force(
    i: usize,
    particles: &[Particle],
    targets: &[Target],
    neighbors: &Neighbors,
    params: &Params,
    time: f32,
    calm: f32,
) -> [f32; 2] {
    let p = particles[i];
    let Some(target) = targets.get(p.owner as usize).copied() else {
        return [0.0; 2];
    };
    let home = cell_of(p.pos, neighbors.cells);
    let mut accel = [0.0; 2];
    let mut drag = [0.0; 2];
    let card = target.kind() == 1;
    let film = KERNEL * params.apart[1].max(1.0);
    let span = if card {
        params.apart[1].max(1.0).ceil() as i32
    } else {
        1
    };
    visit_cell(home, span, neighbors, |j| {
        if j == i {
            return;
        }
        let q = particles[j];
        let d = sub2(p.pos, q.pos);
        let r = len2(d);
        if card
            && q.owner != p.owner
            && targets
                .get(q.owner as usize)
                .is_some_and(|other| other.kind() == 1)
        {
            if r < film && r > 1e-4 {
                let falloff = 1.0 - r / film;
                let push = params.apart[0] * q.life * p.life * falloff * falloff / r;
                accel = add2(accel, scale2(d, push));
            }
            return;
        }
        if r >= KERNEL * 1.5 {
            return;
        }
        if r < 1e-4 {
            accel[0] += ((i as f32 * 0.618).fract() - 0.5) * 50.0;
            accel[1] += ((j as f32 * 0.618).fract() - 0.5) * 50.0;
            return;
        }
        let press = params.pressure * q.life * (1.0 / p.rho + 1.0 / q.rho) * spiky(r) / r;
        accel = sub2(accel, scale2(d, press));
        drag = add2(
            drag,
            scale2(sub2(q.vel, p.vel), q.life * poly6(r * r) / q.rho),
        );
    });
    let shape = region(&target, p.pos, time, calm);
    let out = [p.pos[0] - shape[1], p.pos[1] - shape[2]];
    let dir = scale2(out, 1.0 / len2(out).max(1e-3));
    if target.c[3] > 0.5 {
        accel = sub2(
            accel,
            scale2(dir, params.contain * 0.35 * (shape[0] + target.c[3] - 1.0)),
        );
        if target.kind() == 2 {
            let curve = along(&target, p.pos);
            let keep = curve[0].clamp(target.form[2], 1.0 - target.form[2]);
            accel = add2(
                accel,
                scale2(
                    [curve[1], curve[2]],
                    params.contain * 0.5 * (keep - curve[0]) * curve[3],
                ),
            );
        }
        if target.kind() == 0 && target.b[3] > 0.0 {
            let rel = sub2(p.pos, [target.a[0], target.a[1]]);
            let delta = rel[1].atan2(rel[0]) - target.b[2];
            let off = delta.sin().atan2(delta.cos());
            if off.abs() < target.b[3] {
                let tangent = scale2([-rel[1], rel[0]], 1.0 / len2(rel).max(1.0));
                let push = params.contain * 2.0 * sign(off) * (target.b[3] - off.abs()) * len2(rel);
                accel = add2(accel, scale2(tangent, push));
            }
        }
    } else {
        if shape[0] > 0.0 {
            accel = sub2(accel, scale2(dir, params.contain * shape[0]));
        }
        let mut hold = 1.0;
        if target.kind() == 0 && params.tension[0] > 0.0 {
            hold = params.tension[2]
                + smoothstep(-params.tension[0], 0.0, shape[0]) * params.tension[1];
        }
        accel = sub2(accel, scale2(out, params.cohesion * target.flow[1] * hold));
        if target.kind() == 2 && target.form[1] != 0.0 {
            let curve = along(&target, p.pos);
            let push = params.cohesion
                * target.flow[1]
                * target.form[1]
                * (2.0 * curve[0] - 1.0)
                * curve[3]
                * 0.5;
            accel = add2(accel, scale2([curve[1], curve[2]], push));
        }
    }
    add2(accel, scale2(drag, params.viscosity))
}

pub struct FluidDesc {
    pub width: u32,
    pub height: u32,
    pub max_particles: u32,
    pub max_targets: usize,
    pub params: Params,
    pub settle: f32,
}

impl FluidDesc {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ..Self::default()
        }
    }
}

impl Default for FluidDesc {
    fn default() -> Self {
        Self {
            width: 1024,
            height: 1024,
            max_particles: MAX_PARTICLES,
            max_targets: MAX_TARGETS,
            params: Params::default(),
            settle: SETTLE,
        }
    }
}

struct Body {
    key: String,
    slot: usize,
    start: u32,
    count: u32,
    seen: bool,
    gone: f32,
    quiet: f32,
}

struct Pipes {
    clear_cells: wgpu::ComputePipeline,
    count: wgpu::ComputePipeline,
    scan_cells: wgpu::ComputePipeline,
    scan_blocks: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    order: wgpu::ComputePipeline,
    density: wgpu::ComputePipeline,
    forces: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    gather: wgpu::ComputePipeline,
}

struct Grid {
    counts: wgpu::Buffer,
    starts: wgpu::Buffer,
    items: wgpu::Buffer,
    ranks: wgpu::Buffer,
}

pub struct Fluid {
    particles: wgpu::Buffer,
    grid: Grid,
    targets_buffer: wgpu::Buffer,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pipes: Pipes,
    pipeline: Pipeline,
    height_texture: wgpu::Texture,
    height_view: wgpu::TextureView,
    width: u32,
    height: u32,
    cells: [u32; 2],
    params: Params,
    bodies: Vec<Body>,
    free_slots: Vec<usize>,
    free_ranges: Vec<(u32, u32)>,
    high: u32,
    targets: Vec<Target>,
    clock: f32,
    rest: f32,
    still: f32,
    settle: f32,
    pub moved: bool,
    stale: bool,
    wires: Vec<[f32; 4]>,
    last_wires: Vec<[f32; 4]>,
    wire: Wire,
    smoothing: Smoothing,
    outline: Outline,
    wire_changed: bool,
}

impl Fluid {
    pub fn new(device: &wgpu::Device, desc: FluidDesc) -> Self {
        let width = desc.width.max(1);
        let height = desc.height.max(1);
        let max_particles = desc.max_particles.max(1);
        let max_targets = desc.max_targets.max(1);
        let cells_usize = cells_for([width as f32, height as f32]);
        let cells = [cells_usize[0] as u32, cells_usize[1] as u32];
        let cell_total = (cells[0] * cells[1]) as u64;
        let block_total = cell_total.div_ceil(BLOCK as u64);
        let storage = |label: &str, size: u64, mapped: bool| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: mapped,
            })
        };
        let particles = storage(
            "Particles",
            max_particles as u64 * std::mem::size_of::<Particle>() as u64,
            true,
        );
        particles.slice(..).get_mapped_range_mut().fill(0);
        particles.unmap();
        let targets_buffer = storage(
            "Targets",
            max_targets as u64 * std::mem::size_of::<Target>() as u64,
            true,
        );
        targets_buffer.slice(..).get_mapped_range_mut().fill(0);
        targets_buffer.unmap();
        let grid = Grid {
            counts: storage("Cell counts", cell_total * 4, false),
            starts: storage("Cell starts", (cell_total + block_total) * 4, false),
            items: storage("Cell items", max_particles as u64 * 4, false),
            ranks: storage("Cell ranks", max_particles as u64 * 4, false),
        };
        let accel = storage("Accelerations", max_particles as u64 * 8, false);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sim"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
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
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let height_texture = texture("Liquid height");
        let height_view = height_texture.create_view(&Default::default());
        let raw_texture = texture("Liquid raw");
        let raw_view = raw_texture.create_view(&Default::default());
        let buffer_entry = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Sim"),
            entries: &[
                buffer_entry(0, false),
                buffer_entry(1, true),
                buffer_entry(2, false),
                buffer_entry(3, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                buffer_entry(5, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                buffer_entry(7, false),
                buffer_entry(8, false),
            ],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Sim"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: particles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: targets_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: grid.counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: grid.items.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: accel.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&raw_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: grid.starts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: grid.ranks.as_entire_binding(),
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sim"),
            source: wgpu::ShaderSource::Wgsl(SPH_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Sim"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipe = |name: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipeline = Pipeline::new(device, width, height, &raw_view, &height_view);
        Self {
            particles,
            grid,
            targets_buffer,
            uniform,
            bind,
            pipes: Pipes {
                clear_cells: pipe("clear_cells"),
                count: pipe("count"),
                scan_cells: pipe("scan_cells"),
                scan_blocks: pipe("scan_blocks"),
                scatter: pipe("scatter"),
                order: pipe("order"),
                density: pipe("density"),
                forces: pipe("forces"),
                integrate: pipe("integrate"),
                gather: pipe("gather"),
            },
            pipeline,
            height_texture,
            height_view,
            width,
            height,
            cells,
            params: desc.params,
            bodies: Vec::new(),
            free_slots: (0..max_targets).rev().collect(),
            free_ranges: vec![(0, max_particles)],
            high: 0,
            targets: vec![Target::default(); max_targets],
            clock: 0.0,
            rest: rest_density(),
            still: 0.0,
            settle: if desc.settle.is_finite() {
                desc.settle.max(0.0)
            } else {
                SETTLE
            },
            moved: true,
            stale: true,
            wires: Vec::new(),
            last_wires: Vec::new(),
            wire: Wire::default(),
            smoothing: Smoothing::default(),
            outline: Outline::default(),
            wire_changed: false,
        }
    }

    pub fn height(&self) -> &wgpu::TextureView {
        &self.height_view
    }

    pub fn edge_view(&self) -> &wgpu::TextureView {
        self.pipeline.edge_view()
    }

    pub fn edge_texture(&self) -> &wgpu::Texture {
        self.pipeline.edge_texture()
    }

    pub fn settle(&self) -> f32 {
        self.settle
    }

    pub fn height_texture(&self) -> &wgpu::Texture {
        &self.height_texture
    }

    pub fn particles_buffer(&self) -> &wgpu::Buffer {
        &self.particles
    }

    pub fn cell_counts(&self) -> &wgpu::Buffer {
        &self.grid.counts
    }

    pub fn cell_starts(&self) -> &wgpu::Buffer {
        &self.grid.starts
    }

    pub fn cell_items(&self) -> &wgpu::Buffer {
        &self.grid.items
    }

    pub fn particle_count(&self) -> u32 {
        self.bodies.iter().map(|body| body.count).sum()
    }

    pub fn set_wires(&mut self, segments: &[[f32; 4]], wire: Wire) {
        let next = &segments[..segments.len().min(256)];
        if self.last_wires.len() != next.len()
            || self
                .last_wires
                .iter()
                .zip(next)
                .any(|(left, right)| left.iter().zip(right).any(|(a, b)| (a - b).abs() > 0.05))
        {
            self.still = 0.0;
            self.wire_changed = true;
            self.last_wires = next.to_vec();
        }
        if self.wire != wire {
            self.still = 0.0;
            self.wire_changed = true;
        }
        self.wires.clear();
        self.wires.extend_from_slice(next);
        self.wire = wire;
    }

    pub fn set_smoothing(&mut self, smoothing: Smoothing) {
        if self.smoothing != smoothing {
            self.still = 0.0;
            self.wire_changed = true;
        }
        self.smoothing = smoothing;
    }

    pub fn set_outline(&mut self, outline: Outline) {
        if self.outline != outline {
            self.still = 0.0;
            self.wire_changed = true;
        }
        self.outline = outline;
    }

    pub fn set_field(
        &mut self,
        segments: &[[f32; 4]],
        wire: [f32; 3],
        smoothing: [f32; 3],
        outline: [f32; 8],
    ) {
        self.set_wires(
            segments,
            Wire {
                radius: wire[0],
                lift: wire[1],
                period: wire[2],
            },
        );
        self.set_smoothing(Smoothing {
            wide: smoothing[0],
            fine: smoothing[1],
            pool: smoothing[2],
        });
        self.set_outline(Outline {
            shape: outline[..4].try_into().expect("outline shape"),
            size: outline[4..].try_into().expect("outline size"),
        });
    }

    pub fn sync(&mut self, queue: &wgpu::Queue, pours: &[Pour], dt: f32) {
        let before = self.targets.clone();
        let count = self.bodies.len();
        for body in &mut self.bodies {
            body.seen = false;
        }
        for pour in pours {
            if let Some(index) = self.bodies.iter().position(|body| body.key == pour.key) {
                let slot = self.bodies[index].slot;
                let next = pour.target.carried(&self.targets[slot], dt);
                let moved = target_drift(&self.targets[slot], &next) > 0.05;
                let body = &mut self.bodies[index];
                body.seen = true;
                body.gone = 0.0;
                body.quiet = if moved { 0.0 } else { body.quiet + dt };
                self.targets[slot] = next;
                continue;
            }
            let Some(slot) = self.free_slots.pop() else {
                continue;
            };
            let seed = pour.key.bytes().fold(2_166_136_261u32, |hash, byte| {
                (hash ^ byte as u32).wrapping_mul(16_777_619)
            });
            let particles = spawn(
                pour.fill.as_ref().unwrap_or(&pour.target),
                slot,
                pour.from,
                seed,
            );
            if particles.is_empty() {
                self.free_slots.push(slot);
                continue;
            }
            let count = particles.len() as u32;
            let Some(start) = self.allocate(count) else {
                self.free_slots.push(slot);
                continue;
            };
            queue.write_buffer(
                &self.particles,
                start as u64 * std::mem::size_of::<Particle>() as u64,
                bytemuck::cast_slice(&particles),
            );
            self.targets[slot] = pour.target;
            self.bodies.push(Body {
                key: pour.key.clone(),
                slot,
                start,
                count,
                seen: true,
                gone: 0.0,
                quiet: 0.0,
            });
        }
        let mut freed = Vec::new();
        for body in &mut self.bodies {
            if body.seen {
                continue;
            }
            self.targets[body.slot].flow[3] = 0.0;
            self.targets[body.slot].carry[0] = 0.0;
            self.targets[body.slot].carry[1] = 0.0;
            body.gone += dt;
            if body.gone > DRAIN_TIME {
                freed.push(body.key.clone());
            }
        }
        for key in freed {
            let Some(index) = self.bodies.iter().position(|body| body.key == key) else {
                continue;
            };
            let body = self.bodies.remove(index);
            let dead = vec![
                Particle {
                    born: -1.0,
                    ..Particle::default()
                };
                body.count as usize
            ];
            queue.write_buffer(
                &self.particles,
                body.start as u64 * std::mem::size_of::<Particle>() as u64,
                bytemuck::cast_slice(&dead),
            );
            self.release(body.start, body.count);
            self.free_slots.push(body.slot);
        }
        let changed = self.bodies.len() != count
            || self.bodies.iter().any(|body| body.gone > 0.0)
            || target_drift_all(&before, &self.targets) > 0.05
            || self.wire_changed;
        self.still = if changed { 0.0 } else { self.still + dt };
        self.wire_changed = false;
        queue.write_buffer(&self.targets_buffer, 0, bytemuck::cast_slice(&self.targets));
    }

    pub fn step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dt: f32,
        time: f32,
        calm: f32,
        field: bool,
    ) {
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Sim") });
        self.encode(queue, &mut encoder, dt, time, calm, field, None);
        queue.submit(Some(encoder.finish()));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        dt: f32,
        time: f32,
        calm: f32,
        field: bool,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        if !dt.is_finite() || dt < 0.0 {
            return;
        }
        let resting = self.still > self.settle;
        if resting {
            self.clock = 0.0;
            if !(field && self.stale) {
                self.moved = false;
                return;
            }
        }
        self.moved = field;
        let asleep = self.asleep();
        let uniform = self.sim_uniform(asleep, time, calm);
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        if field {
            self.pipeline
                .update(queue, self.smoothing, self.outline, &self.wires, self.wire);
        }
        self.clock = (self.clock + dt).min(SUBSTEP * 4.0);
        let particle_groups = self.high.div_ceil(256).max(1);
        let awake_groups = self.high.saturating_sub(asleep).div_ceil(256).max(1);
        let cell_groups = (self.cells[0] * self.cells[1]).div_ceil(BLOCK).max(1);
        {
            let stamp = profiler
                .as_deref_mut()
                .and_then(|profile| profile.pass("fluid sim"));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Sim"),
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|profile| profile.compute_writes(stamp)),
            });
            pass.set_bind_group(0, &self.bind, &[]);
            while self.clock >= SUBSTEP {
                self.clock -= SUBSTEP;
                self.bin(&mut pass, particle_groups, cell_groups);
                dispatch(&mut pass, &self.pipes.density, awake_groups, 1);
                dispatch(&mut pass, &self.pipes.forces, awake_groups, 1);
                dispatch(&mut pass, &self.pipes.integrate, awake_groups, 1);
            }
        }
        if !field {
            self.stale = true;
            return;
        }
        self.stale = false;
        {
            let stamp = profiler
                .as_deref_mut()
                .and_then(|profile| profile.pass("fluid gather"));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Gather"),
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|profile| profile.compute_writes(stamp)),
            });
            pass.set_bind_group(0, &self.bind, &[]);
            self.bin(&mut pass, particle_groups, cell_groups);
            dispatch(
                &mut pass,
                &self.pipes.gather,
                self.width.div_ceil(8),
                self.height.div_ceil(8),
            );
        }
        self.pipeline.encode(encoder, profiler);
    }

    fn sim_uniform(&self, asleep: u32, time: f32, calm: f32) -> Uniform {
        let mut apart = self.params.apart;
        apart[2] = asleep as f32;
        Uniform {
            grid: [
                self.cells[0] as f32,
                self.cells[1] as f32,
                KERNEL,
                self.high as f32,
            ],
            table: [self.width as f32, self.height as f32, self.rest, SUBSTEP],
            k: [
                self.params.pressure,
                self.params.contain,
                self.params.cohesion,
                self.params.damping,
            ],
            misc: [
                self.params.life_rate,
                SPACING,
                self.params.gather.clamp(1.0, 2.0),
                self.params.viscosity,
            ],
            apart,
            tension: self.params.tension,
            clock: [time, calm, 0.0, 0.0],
        }
    }

    fn bin(&self, pass: &mut wgpu::ComputePass, particle_groups: u32, cell_groups: u32) {
        dispatch(pass, &self.pipes.clear_cells, cell_groups, 1);
        dispatch(pass, &self.pipes.count, particle_groups, 1);
        dispatch(pass, &self.pipes.scan_cells, cell_groups, 1);
        dispatch(pass, &self.pipes.scan_blocks, 1, 1);
        dispatch(pass, &self.pipes.scatter, particle_groups, 1);
        dispatch(pass, &self.pipes.order, cell_groups, 1);
    }

    fn asleep(&self) -> u32 {
        self.bodies
            .iter()
            .find(|body| body.start == 0 && body.seen && body.quiet > self.settle)
            .map_or(0, |body| body.count)
    }

    fn allocate(&mut self, count: u32) -> Option<u32> {
        let at = self.free_ranges.iter().position(|(_, len)| *len >= count)?;
        let (start, len) = self.free_ranges[at];
        if len == count {
            self.free_ranges.remove(at);
        } else {
            self.free_ranges[at] = (start + count, len - count);
        }
        self.high = self.high.max(start + count);
        Some(start)
    }

    fn release(&mut self, start: u32, count: u32) {
        self.free_ranges.push((start, count));
        self.free_ranges.sort_by_key(|range| range.0);
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for (start, len) in self.free_ranges.drain(..) {
            if let Some(last) = merged.last_mut()
                && last.0 + last.1 == start
            {
                last.1 += len;
                continue;
            }
            merged.push((start, len));
        }
        self.free_ranges = merged;
    }
}

fn dispatch(pass: &mut wgpu::ComputePass, pipeline: &wgpu::ComputePipeline, x: u32, y: u32) {
    pass.set_pipeline(pipeline);
    pass.dispatch_workgroups(x.max(1), y.max(1), 1);
}

fn target_drift(a: &Target, b: &Target) -> f32 {
    let left: &[f32] = bytemuck::cast_slice(std::slice::from_ref(a));
    let right: &[f32] = bytemuck::cast_slice(std::slice::from_ref(b));
    left.iter()
        .zip(right)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

fn target_drift_all(a: &[Target], b: &[Target]) -> f32 {
    let left: &[f32] = bytemuck::cast_slice(a);
    let right: &[f32] = bytemuck::cast_slice(b);
    left.iter()
        .zip(right)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[cfg(test)]
#[path = "fluid_seams_test.rs"]
mod seams;

#[cfg(test)]
#[path = "fluid_determinism_test.rs"]
mod determinism;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::Form;

    fn reference_raise(
        p: [f32; 2],
        value: [f32; 4],
        segments: &[[f32; 4]],
        wire: Wire,
    ) -> [f32; 4] {
        let mut h = value[0];
        let mut ch = [
            value[1] / h.max(1e-3),
            value[2] / h.max(1e-3),
            value[3] / h.max(1e-3),
        ];
        let mut near = 1e9f32;
        let mut along = 0.0;
        let mut side = 0.0;
        let mut run = 0.0;
        for segment in segments.iter().take(256) {
            let a = [segment[0], segment[1]];
            let ba = [segment[2] - a[0], segment[3] - a[1]];
            let span = ba[0].hypot(ba[1]);
            let t = (dot2(sub2(p, a), ba) / (span * span).max(1e-4)).clamp(0.0, 1.0);
            let off = sub2(sub2(p, a), scale2(ba, t));
            let d = len2(off);
            if d < near {
                near = d;
                along = run + t * span;
                side = (ba[0] * off[1] - ba[1] * off[0]).signum() * d;
            }
            run += span;
        }
        if near < wire.radius + 1.0 {
            let across = (near / wire.radius).clamp(0.0, 1.0);
            let strands = 0.5
                + 0.5
                    * ((along / wire.period + side / wire.radius * 0.9) * std::f32::consts::TAU)
                        .cos();
            let lift =
                wire.lift * (1.0 - across * across).max(0.0).sqrt() * (0.82 + 0.18 * strands);
            let edge = 1.0 - smoothstep(wire.radius - 0.6, wire.radius + 0.6, near);
            if lift > h {
                for (channel, target) in ch.iter_mut().zip([0.0, 1.0, 12.0]) {
                    *channel += (target - *channel) * edge;
                }
                h += (lift - h) * edge;
            }
        }
        [h, ch[0] * h, ch[1] * h, ch[2] * h]
    }

    #[test]
    fn wire_raise_single_segment_and_strands() {
        let wire = Wire::default();
        let segment = [[0.0, 0.0, 20.0, 0.0]];
        let crest = reference_raise([0.0, 0.0], [0.0; 4], &segment, wire);
        let trough = reference_raise([1.6, 0.0], [0.0; 4], &segment, wire);
        let strands = 0.5 + 0.5 * (1.6 / wire.period * std::f32::consts::TAU).cos();
        assert!((crest[0] - wire.lift).abs() < 1e-5);
        assert!((trough[0] - wire.lift * (0.82 + 0.18 * strands)).abs() < 1e-5);
        assert!((crest[3] / crest[0] - 12.0).abs() < 1e-5);
        let edge = reference_raise([5.0, 1.8], [0.2, 0.0, 0.0, 0.0], &segment, wire);
        assert!(edge[0] > 0.2 && edge[0] < crest[0]);
        let high = reference_raise([5.0, 0.0], [10.0, 1.0, 2.0, 3.0], &segment, wire);
        assert_eq!(high, [10.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn dual_sigma_and_taps() {
        let smoothing = Smoothing::default();
        let [wide, pool] = smoothing.residuals();
        assert!((wide - (100.0f32 - 4.0).sqrt()).abs() < 0.0001);
        assert!((pool - (400.0f32 - 4.0).sqrt()).abs() < 0.0001);
        let sigma = 3.0f32;
        let reach = (sigma * 2.5).ceil() as i32;
        let weights: Vec<f32> = (-reach..=reach)
            .map(|k| (-(k * k) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let total: f32 = weights.iter().sum();
        let normalized: f32 = weights.iter().map(|w| w / total).sum();
        assert!((normalized - 1.0).abs() < 1e-6);
        let analytic: f32 = weights.iter().map(|w| w / (sigma * 2.5066283)).sum();
        assert!((analytic - 1.0).abs() < 0.02);
    }

    #[test]
    fn field_shaders_validate() {
        for source in [SPH_WGSL, BLUR_WGSL, JFA_WGSL, EDGE_WGSL, RAISE_WGSL] {
            let module = naga::front::wgsl::parse_str(source).expect("parse field shader");
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("validate field shader");
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn two_wires_and_small_pool_inspect() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("fluid wire test"),
            required_features: wgpu::Features::empty(),
            ..Default::default()
        }))
        .expect("device");
        let mut fluid = Fluid::new(
            &device,
            FluidDesc {
                width: 64,
                height: 64,
                max_particles: 512,
                max_targets: 4,
                params: Params::default(),
                settle: SETTLE,
            },
        );
        let look = Look {
            tall: 1.0,
            active: 0.0,
            weight_b: 0.0,
            weight_a: 0.0,
        };
        let pour = Pour {
            key: "pool".into(),
            target: Target::bead([32.0, 32.0], 8.0, &look),
            from: None,
            fill: None,
        };
        let wires = [[8.0, 32.5, 56.0, 32.5], [32.5, 8.0, 32.5, 56.0]];
        let dt = SUBSTEP * 4.0;
        fluid.sync(&queue, std::slice::from_ref(&pour), dt);
        let wire = Wire::default();
        let smoothing = Smoothing::default();
        let outline = Outline::default();
        fluid.set_field(
            &wires,
            [wire.radius, wire.lift, wire.period],
            [smoothing.wide, smoothing.fine, smoothing.pool],
            [
                outline.shape[0],
                outline.shape[1],
                outline.shape[2],
                outline.shape[3],
                outline.size[0],
                outline.size[1],
                outline.size[2],
                outline.size[3],
            ],
        );
        for frame in 0..10 {
            fluid.step(&device, &queue, dt, frame as f32 * dt, 1.0, true);
        }
        let state = fluid.inspect_row(&device, &queue, 32);
        assert_eq!(state.live, fluid.particle_count());
        let (lo, hi) = state.bounds.expect("pool bounds");
        assert!(lo[0] > 16.0 && lo[1] > 16.0 && hi[0] < 48.0 && hi[1] < 48.0);
        assert!(
            state.height_min >= 0.0
                && state.height_max > wire.lift - 1.0
                && state.height_max < wire.lift + 1.0
        );
        assert!(state.height_mean > 0.0 && state.height_mean < 2.0);
        assert_eq!(state.row.as_ref().map(Vec::len), Some(64));
        assert!(state.channel_histogram[2].contains_key(&192));
        fluid.sync(&queue, std::slice::from_ref(&pour), 3.0);
        fluid.sync(&queue, &[pour], 3.0);
        let rested = fluid.inspect(&device, &queue);
        assert!(rested.resting);
        assert_eq!(rested.asleep, rested.live);
    }

    #[test]
    fn a_bead_keeps_every_particle() {
        let mut fluid = Reference::bead([40.0, 40.0], 10.0, [80.0, 80.0]);
        let n = fluid.count();
        assert!(n > 8, "{n}");
        assert!(rest_density() > 0.0 && rest_density().is_finite());
        let start: Vec<[f32; 2]> = fluid.particles().iter().map(|p| p.pos).collect();
        for frame in 0..60 {
            fluid.step(SUBSTEP * 4.0, frame as f32 * SUBSTEP * 4.0, 1.0);
        }
        assert_eq!(fluid.count(), n);
        assert_eq!(fluid.particles().len(), n);
        let mut moved = false;
        for (particle, from) in fluid.particles().iter().zip(&start) {
            assert!(particle.pos[0].is_finite() && particle.pos[1].is_finite());
            assert!(particle.born > 0.0);
            let away = (particle.pos[0] - 40.0).hypot(particle.pos[1] - 40.0);
            assert!(away < 16.0, "{away}");
            if (particle.pos[0] - from[0]).abs() > 1e-3 || (particle.pos[1] - from[1]).abs() > 1e-3
            {
                moved = true;
            }
        }
        assert!(moved);
    }

    #[test]
    fn the_same_bead_repeats() {
        let mut left = Reference::bead([40.0, 40.0], 8.0, [80.0, 80.0]);
        let mut right = Reference::bead([40.0, 40.0], 8.0, [80.0, 80.0]);
        for frame in 0..20 {
            let time = frame as f32 * SUBSTEP * 4.0;
            left.step(SUBSTEP * 4.0, time, 1.0);
            right.step(SUBSTEP * 4.0, time, 1.0);
        }
        for (a, b) in left.particles().iter().zip(right.particles()) {
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.vel, b.vel);
            assert_eq!(a.life, b.life);
        }
    }

    #[test]
    fn a_drop_spawns_inside_itself() {
        let look = Look {
            tall: 1.0,
            active: 0.2,
            weight_b: 0.0,
            weight_a: 0.1,
        };
        let form = Form {
            tilt: 0.2,
            irregular: 0.15,
            seed: 1.7,
            rim: 0.0,
            weight: [0.3, 0.2],
        };
        let drop = Target::drop([48.0, 48.0], [14.0, 11.0], &form, &look).organic(0.1, 0.05, 2.0);
        let particles = spawn(&drop, 0, None, 9);
        assert!(particles.len() > 4, "{}", particles.len());
        assert!(particles.iter().all(|particle| drop.inside(particle.pos)));
        let bead = Target::bead([20.0, 20.0], 6.0, &look)
            .pull(0.5)
            .mass(1.2)
            .shifted([1.0, -1.0]);
        assert_eq!(bead.kind(), 3);
        assert!(bead.inside(bead.origin()));
        assert_eq!(
            Target::capsule([10.0, 12.0], [28.0, 16.0], [4.0, 3.0], &look)
                .drifting(0.2)
                .kind(),
            1
        );
        assert_eq!(
            Target::curve(
                [[8.0, 8.0], [16.0, 18.0], [28.0, 16.0], [36.0, 10.0]],
                [3.0, 4.0, 3.0],
                &look,
            )
            .flare(1.4)
            .revealed(0.5)
            .kind(),
            2
        );
        assert_eq!(
            Target::outline(
                [24.0, 24.0],
                9.0,
                [
                    [0.04, 0.2],
                    [0.02, 0.4],
                    [0.01, 1.0],
                    [0.02, 0.6],
                    [0.01, 0.3],
                ],
                &look,
            )
            .squash(0.85)
            .gathered([24.0, 24.0], 0.1, 0.9)
            .kind(),
            4
        );
    }
}
