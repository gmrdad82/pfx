use bytemuck::{Pod, Zeroable};
use pfx_gpu::wgpu;
use pfx_physics::{Boid, Emitter, ParticleKind, Speck};
use wgpu::util::DeviceExt;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const SCENE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Plume {
    pub source: [f32; 3],
    pub radius: f32,
    pub spread: [f32; 2],
    pub drift: [[f32; 2]; 2],
    pub sway: [[f32; 5]; 2],
    pub fade: [f32; 3],
    pub depth_per_width: f32,
    pub grain: [f32; 2],
    pub lift: f32,
    pub warp: [f32; 2],
    pub churn: f32,
    pub threshold: [f32; 2],
}

impl Default for Plume {
    fn default() -> Self {
        Self {
            source: [0.0; 3],
            radius: 0.02,
            spread: [1.0, 2.0],
            drift: [[0.0; 2]; 2],
            sway: [[10.0, 1.0, 0.005, 0.5, 2.0]; 2],
            fade: [0.01, 0.05, 0.15],
            depth_per_width: 6.0,
            grain: [60.0, 40.0],
            lift: 0.05,
            warp: [2.0, 0.2],
            churn: 0.6,
            threshold: [0.4, 0.85],
        }
    }
}

impl Plume {
    pub fn at(source: [f32; 3]) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }

    fn look(&self) -> [f32; 28] {
        let [sx, sz] = self.sway;
        [
            self.spread[0],
            self.spread[1],
            self.drift[0][0],
            self.drift[0][1],
            self.drift[1][0],
            self.drift[1][1],
            sx[0],
            sx[1],
            sx[2],
            sx[3],
            sx[4],
            sz[0],
            sz[1],
            sz[2],
            sz[3],
            sz[4],
            self.fade[0],
            self.fade[1],
            self.fade[2],
            self.depth_per_width,
            self.grain[0],
            self.grain[1],
            self.lift,
            self.warp[0],
            self.warp[1],
            self.churn,
            self.threshold[0],
            self.threshold[1],
        ]
    }

    fn key(&self) -> [u32; 28] {
        self.look().map(f32::to_bits)
    }

    fn constants(&self) -> Vec<(&'static str, f64)> {
        PLUME_CONSTANTS
            .iter()
            .zip(self.look())
            .map(|(name, value)| (*name, f64::from(value)))
            .collect()
    }
}

const MAX_VOLUME_PIPELINES: usize = 16;

fn volume_constants(cuts: VolumeCuts, plume: &Plume) -> Vec<(&'static str, f64)> {
    let mut constants = cuts.constants().to_vec();
    constants.extend(plume.constants());
    constants
}

const PLUME_CONSTANTS: [&str; 28] = [
    "SPREAD_BASE",
    "SPREAD_GROWTH",
    "DRIFT_X_QUAD",
    "DRIFT_X_LIN",
    "DRIFT_Z_QUAD",
    "DRIFT_Z_LIN",
    "SWAY_X_WAVE",
    "SWAY_X_RATE",
    "SWAY_X_AMP",
    "SWAY_X_BASE",
    "SWAY_X_GROWTH",
    "SWAY_Z_WAVE",
    "SWAY_Z_RATE",
    "SWAY_Z_AMP",
    "SWAY_Z_BASE",
    "SWAY_Z_GROWTH",
    "FADE_IN",
    "FADE_OUT_START",
    "FADE_OUT_END",
    "DEPTH_PER_WIDTH",
    "GRAIN_H",
    "GRAIN_V",
    "LIFT_RATE",
    "WARP_AMP",
    "WARP_RATE",
    "CHURN",
    "THRESHOLD_LOW",
    "THRESHOLD_HIGH",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VolumeKind {
    Plume(Plume),
    Smoke,
}

pub const HAZE_AMBIENT_GREY: f32 = 0.03;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomHaze {
    pub fog: f32,
    pub smoke: f32,
    pub mist: f32,
    pub floor: f32,
    pub phase: f32,
    pub back: f32,
    pub gold: [f32; 3],
    pub ambient: [f32; 3],
    pub reach: f32,
    pub unmapped: f32,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub seed: u32,
}

impl RoomHaze {
    pub fn new(lo: [f32; 3], hi: [f32; 3]) -> Self {
        Self {
            fog: 0.0,
            smoke: 0.0,
            mist: 0.0,
            floor: 0.0,
            phase: 0.3,
            back: 0.5,
            gold: [1.0; 3],
            ambient: [HAZE_AMBIENT_GREY; 3],
            reach: 3.0,
            unmapped: 0.0,
            lo,
            hi,
            seed: 0,
        }
    }

    pub fn from_amount(lo: [f32; 3], hi: [f32; 3], amount: f32) -> Self {
        let amount = if amount.is_nan() {
            0.0
        } else {
            amount.clamp(0.0, 1.0)
        };
        Self {
            fog: amount / 6.0,
            smoke: amount * 20.0,
            mist: amount * 0.2,
            floor: amount * 2.0,
            ..Self::new(lo, hi)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomMotes {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub cell: f32,
    pub share: f32,
    pub brightness: f32,
    pub gold: [f32; 3],
}

impl RoomMotes {
    pub fn new(
        lo: [f32; 3],
        hi: [f32; 3],
        cell: f32,
        share: f32,
        brightness: f32,
        gold: [f32; 3],
    ) -> Self {
        Self {
            lo,
            hi,
            cell,
            share,
            brightness,
            gold,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct EffectFrame {
    pub view_proj: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub right: [f32; 4],
    pub up: [f32; 4],
    pub forward: [f32; 4],
    pub lens: [f32; 4],
    pub sun: [f32; 4],
    pub sun_color: [f32; 4],
    pub extent: [f32; 4],
    pub volume_lo: [f32; 4],
    pub volume_hi: [f32; 4],
    pub volume_color: [f32; 4],
    pub volume_control: [f32; 4],
    pub room_light: [f32; 4],
    pub room_shape: [f32; 4],
    pub room_lo: [f32; 4],
    pub room_hi: [f32; 4],
    pub plume: Plume,
}

impl EffectFrame {
    pub fn new(width: u32, height: u32, time: f32, seed: u32) -> Self {
        Self {
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            eye: [0.0, 0.0, 3.0, 0.0],
            right: [1.0, 0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0, 0.0],
            lens: [1.0, 1.0, 0.0, 0.0],
            sun: [0.4, 0.8, 0.4, 0.0],
            sun_color: [1.0, 0.9, 0.7, 1.0],
            extent: [width as f32, height as f32, time, f32::from_bits(seed)],
            volume_lo: [-0.25, -0.4, -0.5, 0.0],
            volume_hi: [0.25, 0.7, 0.5, 0.0],
            volume_color: [0.75, 0.8, 0.85, 0.0],
            volume_control: [2.0, 0.35, 1.0, 0.0],
            room_light: [0.0; 4],
            room_shape: [0.0; 4],
            room_lo: [0.0; 4],
            room_hi: [1.0; 4],
            plume: Plume::default(),
        }
    }

    pub fn set_volume(
        &mut self,
        kind: VolumeKind,
        lo: [f32; 3],
        hi: [f32; 3],
        color: [f32; 3],
        density: f32,
        anisotropy: f32,
    ) {
        self.volume_lo = [lo[0], lo[1], lo[2], 0.0];
        self.volume_hi = [hi[0], hi[1], hi[2], 0.0];
        self.volume_color = [color[0], color[1], color[2], 0.0];
        self.volume_control = [
            density.max(0.0),
            anisotropy.clamp(-0.9, 0.9),
            1.0,
            if kind == VolumeKind::Smoke { 1.0 } else { 0.0 },
        ];
        if let VolumeKind::Plume(plume) = kind {
            self.plume = plume;
        }
    }

    pub fn scale_time(&mut self, scale: f32) {
        self.extent[2] *= scale.max(0.0);
    }

    pub fn set_room_haze(&mut self, haze: RoomHaze) {
        self.volume_lo = [-1.0e4, -1.0e4, -1.0e4, haze.floor.max(0.0)];
        self.volume_hi = [1.0e4, 1.0e4, 1.0e4, haze.smoke.max(0.0)];
        self.volume_color = [
            haze.ambient[0].max(0.0),
            haze.ambient[1].max(0.0),
            haze.ambient[2].max(0.0),
            haze.mist.max(0.0),
        ];
        self.volume_control = [haze.fog.max(0.0), haze.phase.clamp(-0.9, 0.9), 1.0, 2.0];
        self.room_light = [
            haze.gold[0].max(0.0),
            haze.gold[1].max(0.0),
            haze.gold[2].max(0.0),
            haze.back.clamp(0.0, 1.0),
        ];
        self.room_shape = [haze.reach.max(0.0), haze.unmapped.clamp(0.0, 1.0), 0.0, 0.0];
        self.room_lo = [haze.lo[0], haze.lo[1], haze.lo[2], 0.0];
        self.room_hi = [haze.hi[0], haze.hi[1], haze.hi[2], 0.0];
    }
}

pub fn fog_transmittance(fog: f32, distance: f32, reach: f32) -> f32 {
    (-fog.max(0.0) * distance.max(0.0).min(reach.max(0.0))).exp()
}

pub fn fog_in_scatter(density: f32, fog: f32, light: f32, distance: f32) -> f32 {
    let distance = distance.max(0.0);
    if fog <= 0.0 {
        return density * light * distance;
    }
    density * light * (1.0 - (-fog * distance).exp()) / fog
}

pub fn henyey_greenstein(cosine: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cosine;
    (1.0 - g * g) / (4.0 * std::f32::consts::PI * d * d.sqrt())
}

fn cell_hash(cell: [i32; 3], seed: u32) -> f32 {
    let z = hash((cell[2] as u32).wrapping_mul(83_492_791) ^ seed);
    let y = hash((cell[1] as u32).wrapping_mul(19_349_663) ^ z);
    let h = hash((cell[0] as u32).wrapping_mul(73_856_093) ^ y);
    (h >> 8) as f32 / 16_777_216.0
}

pub fn room_motes(seed: u32, time: f32, motes: RoomMotes) -> Vec<ParticleInstance> {
    let cell = motes.cell.max(1.0e-3);
    let flow = [0.0021 * time, 0.0012 * time, -0.0009 * time];
    let range = |axis: usize| {
        let first = ((motes.lo[axis] - flow[axis]) / cell).floor() as i32;
        let last = ((motes.hi[axis] - flow[axis]) / cell).floor() as i32;
        first..=last
    };
    let add =
        |c: [i32; 3], offset: [i32; 3]| [c[0] + offset[0], c[1] + offset[1], c[2] + offset[2]];
    let mut out = Vec::new();
    for x in range(0) {
        for y in range(1) {
            for z in range(2) {
                let c = [x, y, z];
                let h = cell_hash(add(c, [17, 5, 91]), seed);
                if h >= motes.share {
                    continue;
                }
                let j = [
                    cell_hash(add(c, [3, 0, 0]), seed),
                    cell_hash(add(c, [0, 7, 0]), seed),
                    cell_hash(add(c, [0, 0, 11]), seed),
                ];
                let wander = [
                    (time * 0.31 + h * 40.0).sin() * 0.12,
                    (time * 0.23 + j[0] * 30.0).sin() * 0.12,
                    (time * 0.27 + j[1] * 20.0).cos() * 0.12,
                ];
                let centre: [f32; 3] = std::array::from_fn(|axis| {
                    (c[axis] as f32 + 0.5 + (j[axis] - 0.5) * 0.5 + wander[axis]) * cell
                        + flow[axis]
                });
                let radius = 0.00025 + 0.0005 * j[2] * j[2];
                let twinkle = 0.55 + 0.45 * (time * (0.7 + 1.3 * j[0]) + h * 60.0).sin();
                let glow = motes.brightness * twinkle;
                out.push(ParticleInstance {
                    position_size: [centre[0], centre[1], centre[2], radius],
                    velocity_alpha: [0.0, 0.0, 0.0, 1.0],
                    color_rotation: [
                        motes.gold[0] * glow,
                        motes.gold[1] * glow,
                        motes.gold[2] * glow,
                        0.0,
                    ],
                    detail: [1.0, 0.0, h, 1.0],
                });
            }
        }
    }
    out
}

pub fn plume_optical_depth(
    density: f32,
    path_length: f32,
    box_width: f32,
    depth_per_width: f32,
) -> f32 {
    density.max(0.0) * path_length.max(0.0) * depth_per_width / box_width.max(1.0e-6)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ParticleInstance {
    pub position_size: [f32; 4],
    pub velocity_alpha: [f32; 4],
    pub color_rotation: [f32; 4],
    pub detail: [f32; 4],
}

fn hash(seed: u32) -> u32 {
    let state = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let mixed = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (mixed >> 22) ^ mixed
}

pub fn pack_particles(
    specks: &[Speck],
    kind: ParticleKind,
    time: f32,
    color: [f32; 3],
) -> Vec<ParticleInstance> {
    specks
        .iter()
        .filter(|speck| speck.life > 0.0 && speck.age < speck.life)
        .map(|speck| {
            let random = hash(speck.seed) as f32 / u32::MAX as f32;
            let life = (1.0 - speck.age / speck.life).clamp(0.0, 1.0);
            let (size, alpha, kind_id) = match kind {
                ParticleKind::Petal => (0.035 + 0.02 * random, 0.8, 0.0),
                ParticleKind::Dust => (0.003 + 0.004 * random, 0.5, 1.0),
                ParticleKind::Steam => (0.08 + 0.04 * random, 0.2, 2.0),
            };
            let speed = (speck.vel[0] * speck.vel[0]
                + speck.vel[1] * speck.vel[1]
                + speck.vel[2] * speck.vel[2])
                .sqrt();
            let angle = random * std::f32::consts::TAU
                + time
                    * (1.0 + random * 2.0)
                    * if kind == ParticleKind::Petal {
                        1.0
                    } else {
                        0.1
                    };
            ParticleInstance {
                position_size: [speck.pos[0], speck.pos[1], speck.pos[2], size],
                velocity_alpha: [
                    speck.vel[0],
                    speck.vel[1],
                    speck.vel[2],
                    alpha * life.min(0.2 + speck.age * 4.0),
                ],
                color_rotation: [color[0], color[1], color[2], angle],
                detail: [kind_id, speed, random, 0.0],
            }
        })
        .collect()
}

pub fn pack_emitter(
    emitter: &Emitter,
    kind: ParticleKind,
    time: f32,
    color: [f32; 3],
) -> Vec<ParticleInstance> {
    pack_particles(emitter.specks(), kind, time, color)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreatureKind {
    Bird,
    Fish,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct CreatureInstance {
    pub position_scale: [f32; 4],
    pub right: [f32; 4],
    pub up: [f32; 4],
    pub forward: [f32; 4],
    pub motion: [f32; 4],
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if length < 1e-6 {
        [0.0, 0.0, -1.0]
    } else {
        a.map(|v| v / length)
    }
}

fn distance_squared(position: [f32; 4], eye: [f32; 4]) -> f32 {
    (0..3)
        .map(|axis| (position[axis] - eye[axis]).powi(2))
        .sum()
}

pub fn pack_creatures(
    boids: &[Boid],
    kind: CreatureKind,
    size: f32,
    seed: u32,
) -> Vec<CreatureInstance> {
    boids
        .iter()
        .enumerate()
        .map(|(index, boid)| {
            let forward = unit(boid.vel);
            let reference = if forward[1].abs() > 0.95 {
                [0.0, 0.0, 1.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let right = unit(cross(reference, forward));
            let up = cross(forward, right);
            let speed =
                (boid.vel[0] * boid.vel[0] + boid.vel[1] * boid.vel[1] + boid.vel[2] * boid.vel[2])
                    .sqrt();
            let phase = hash(seed ^ index as u32) as f32 / u32::MAX as f32 * std::f32::consts::TAU;
            CreatureInstance {
                position_scale: [boid.pos[0], boid.pos[1], boid.pos[2], size],
                right: [right[0], right[1], right[2], 0.0],
                up: [up[0], up[1], up[2], 0.0],
                forward: [forward[0], forward[1], forward[2], 0.0],
                motion: [
                    phase,
                    speed,
                    if kind == CreatureKind::Bird { 0.0 } else { 1.0 },
                    0.0,
                ],
            }
        })
        .collect()
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct CreatureVertex {
    pub position: [f32; 4],
}

pub fn upsample_weights(
    full_depth: f32,
    depths: [f32; 4],
    fractions: [f32; 2],
    depth_scale: f32,
) -> [f32; 4] {
    let x = fractions[0].clamp(0.0, 1.0);
    let y = fractions[1].clamp(0.0, 1.0);
    let spatial = [(1.0 - x) * (1.0 - y), x * (1.0 - y), (1.0 - x) * y, x * y];
    let mut weights = [0.0; 4];
    let mut total = 0.0;
    for i in 0..4 {
        let difference = (full_depth - depths[i]).abs();
        weights[i] = spatial[i] * (-difference * depth_scale.max(0.0)).exp();
        total += weights[i];
    }
    if total > 1e-6 {
        weights.map(|weight| weight / total)
    } else {
        let nearest = depths
            .iter()
            .enumerate()
            .min_by(|left, right| {
                (full_depth - *left.1)
                    .abs()
                    .total_cmp(&(full_depth - *right.1).abs())
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        let mut fallback = [0.0; 4];
        fallback[nearest] = 1.0;
        fallback
    }
}

fn volume_bounds(frame: &EffectFrame, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for x in [frame.volume_lo[0], frame.volume_hi[0]] {
        for y in [frame.volume_lo[1], frame.volume_hi[1]] {
            for z in [frame.volume_lo[2], frame.volume_hi[2]] {
                let point = [x, y, z, 1.0];
                let mut clip = [0.0; 4];
                for (row, value) in clip.iter_mut().enumerate() {
                    *value = (0..4)
                        .map(|column| frame.view_proj[column][row] * point[column])
                        .sum();
                }
                if clip[3] <= 0.0 || !clip.iter().all(|value| value.is_finite()) {
                    return Some((0, 0, width, height));
                }
                let screen = [
                    (clip[0] / clip[3] * 0.5 + 0.5) * width as f32,
                    (0.5 - clip[1] / clip[3] * 0.5) * height as f32,
                ];
                for axis in 0..2 {
                    min[axis] = min[axis].min(screen[axis]);
                    max[axis] = max[axis].max(screen[axis]);
                }
            }
        }
    }
    let left = (min[0] - 8.0).floor().clamp(0.0, width as f32) as u32;
    let top = (min[1] - 8.0).floor().clamp(0.0, height as f32) as u32;
    let right = (max[0] + 8.0).ceil().clamp(0.0, width as f32) as u32;
    let bottom = (max[1] + 8.0).ceil().clamp(0.0, height as f32) as u32;
    if right <= left || bottom <= top {
        None
    } else {
        Some((left, top, right - left, bottom - top))
    }
}

pub const PARTICLE_WGSL: &str = r#"
struct Frame {
    view_proj: mat4x4f, eye: vec4f, right: vec4f, up: vec4f, forward: vec4f,
    lens: vec4f, sun: vec4f, sun_color: vec4f, extent: vec4f,
    volume_lo: vec4f, volume_hi: vec4f, volume_color: vec4f, volume_control: vec4f,
}
struct Shadow {
    splits: vec4f, filter_params: vec4f, sun: vec4f, texels: vec4f,
    view_proj_0: mat4x4f, view_proj_1: mat4x4f, view_proj_2: mat4x4f,
}
struct Particle {
    position_size: vec4f, velocity_alpha: vec4f, color_rotation: vec4f, detail: vec4f,
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var scene_depth: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> particles: array<Particle>;
@group(0) @binding(3) var shadow_atlas: texture_depth_2d_array;
@group(0) @binding(4) var<uniform> shadow: Shadow;
struct Out { @builtin(position) clip: vec4f, @location(0) uv: vec2f, @location(1) color: vec4f, @location(2) distance: f32, @location(3) kind: f32, @location(4) world: vec3f, @location(5) lit: f32, }
@vertex fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> Out {
    let p = particles[index];
    let corners = array<vec2f, 6>(vec2f(-1.0,-1.0),vec2f(1.0,-1.0),vec2f(-1.0,1.0),vec2f(-1.0,1.0),vec2f(1.0,-1.0),vec2f(1.0,1.0));
    let corner = corners[vertex];
    let spin = p.color_rotation.w;
    var axis_x = frame.right.xyz * cos(spin) + frame.up.xyz * sin(spin);
    var axis_y = frame.up.xyz * cos(spin) - frame.right.xyz * sin(spin);
    if (p.detail.x > 0.5 && p.detail.x < 1.5 && p.detail.y > 0.02) {
        let v = normalize(p.velocity_alpha.xyz - frame.forward.xyz * dot(p.velocity_alpha.xyz, frame.forward.xyz));
        axis_y = v;
        axis_x = cross(axis_y, frame.forward.xyz);
    }
    let tumble = select(1.0, 0.45 + 0.55 * abs(sin(spin * 0.67)), p.detail.x < 0.5);
    let world = p.position_size.xyz + (axis_x * corner.x * tumble + axis_y * corner.y) * p.position_size.w;
    var out: Out;
    out.clip = frame.view_proj * vec4f(world, 1.0);
    out.uv = corner;
    out.color = vec4f(p.color_rotation.xyz, p.velocity_alpha.w);
    out.distance = length(world - frame.eye.xyz);
    out.kind = p.detail.x;
    out.world = world;
    out.lit = p.detail.w;
    return out;
}
fn sunlight(world: vec3f, distance: f32) -> f32 {
    if (shadow.splits.w <= 0.0) { return 1.0; }
    var layer = 0;
    var matrix = shadow.view_proj_0;
    if (distance > shadow.splits.y) { layer = 1; matrix = shadow.view_proj_1; }
    if (distance > shadow.splits.z) { layer = 2; matrix = shadow.view_proj_2; }
    let clip = matrix * vec4f(world, 1.0);
    let point = clip.xyz / clip.w;
    let uv = point.xy * vec2f(0.5, -0.5) + vec2f(0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || point.z < 0.0 || point.z > 1.0) { return 1.0; }
    let size = textureDimensions(shadow_atlas);
    let at = clamp(vec2i(uv * vec2f(size)), vec2i(0), vec2i(size) - vec2i(1));
    let depth = textureLoad(shadow_atlas, at, layer, 0);
    return select(0.0, 1.0, point.z - shadow.sun.w <= depth);
}
@fragment fn fs_main(in: Out) -> @location(0) vec4f {
    let pixel = vec2i(in.clip.xy);
    let depth = textureLoad(scene_depth, pixel, 0).x;
    let soft = clamp((depth - in.distance) * 4.0, 0.0, 1.0);
    let r = length(in.uv);
    let petal = smoothstep(1.0, 0.7, length(vec2f(in.uv.x, in.uv.y * 1.35)));
    let dust = exp(-r * r * 5.0) * (0.7 + 0.3 * pow(max(0.0, 1.0 - r), 16.0));
    let steam = exp(-r * r * 3.0);
    let shape = select(select(petal, dust, in.kind > 0.5), steam, in.kind > 1.5);
    if (in.lit > 0.5) {
        let glint = max(1.0 - r, 0.0) * select(0.0, 1.0, depth > dot(in.world - frame.eye.xyz, frame.forward.xyz));
        return vec4f(in.color.rgb * frame.sun_color.rgb * frame.sun_color.w * sunlight(in.world, in.distance) * glint, 0.0);
    }
    let alpha = in.color.a * shape * soft;
    return vec4f(in.color.rgb * alpha, alpha);
}
"#;

pub const CREATURE_WGSL: &str = r#"
struct Frame {
    view_proj: mat4x4f, eye: vec4f, right: vec4f, up: vec4f, forward: vec4f,
    lens: vec4f, sun: vec4f, sun_color: vec4f, extent: vec4f,
    volume_lo: vec4f, volume_hi: vec4f, volume_color: vec4f, volume_control: vec4f,
}
struct Creature { position_scale: vec4f, right: vec4f, up: vec4f, forward: vec4f, motion: vec4f, }
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> creatures: array<Creature>;
@group(0) @binding(2) var<storage, read> vertices: array<vec4f>;
@group(0) @binding(3) var scene_depth: texture_2d<f32>;
struct Out { @builtin(position) clip: vec4f, @location(0) color: vec3f, @location(1) distance: f32, }
@vertex fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> Out {
    let instance = creatures[index];
    var local = vertices[vertex].xyz;
    let phase = frame.extent.z * (4.0 + instance.motion.y * 1.3) + instance.motion.x;
    if (instance.motion.z < 0.5) {
        local.y += sin(phase) * abs(local.x) * 0.5;
    } else {
        local.x += sin(phase + local.z * 4.0) * max(-local.z, 0.0) * 0.2;
    }
    let world = instance.position_scale.xyz + instance.position_scale.w * (instance.right.xyz * local.x + instance.up.xyz * local.y + instance.forward.xyz * local.z);
    var out: Out;
    out.clip = frame.view_proj * vec4f(world, 1.0);
    out.distance = length(world - frame.eye.xyz);
    let light = 0.45 + 0.55 * max(0.0, dot(normalize(instance.up.xyz + vec3f(0.0, 0.3, 0.0)), normalize(frame.sun.xyz)));
    out.color = select(vec3f(0.72, 0.58, 0.43), vec3f(0.28, 0.54, 0.63), instance.motion.z > 0.5) * light * frame.sun_color.rgb;
    return out;
}
@fragment fn fs_main(in: Out) -> @location(0) vec4f {
    if (in.distance > textureLoad(scene_depth, vec2i(in.clip.xy), 0).x) { discard; }
    return vec4f(in.color, 1.0);
}
"#;

pub const VOLUME_WGSL: &str = r#"
struct Frame {
    view_proj: mat4x4f, eye: vec4f, right: vec4f, up: vec4f, forward: vec4f,
    lens: vec4f, sun: vec4f, sun_color: vec4f, extent: vec4f,
    volume_lo: vec4f, volume_hi: vec4f, volume_color: vec4f, volume_control: vec4f,
    room_light: vec4f, room_shape: vec4f, room_lo: vec4f, room_hi: vec4f,
    plume: array<vec4f, 8>,
}
struct Shadow {
    splits: vec4f, filter_params: vec4f, sun: vec4f, texels: vec4f,
    view_proj_0: mat4x4f, view_proj_1: mat4x4f, view_proj_2: mat4x4f,
}
override STEPS: u32 = 20u;
override WARP_OCTAVES: i32 = 4;
override OCTAVES: i32 = 4;
override EARLY_OUT: bool = false;
override SPREAD_BASE: f32 = 1.0;
override SPREAD_GROWTH: f32 = 2.0;
override DRIFT_X_QUAD: f32 = 0.0;
override DRIFT_X_LIN: f32 = 0.0;
override DRIFT_Z_QUAD: f32 = 0.0;
override DRIFT_Z_LIN: f32 = 0.0;
override SWAY_X_WAVE: f32 = 10.0;
override SWAY_X_RATE: f32 = 1.0;
override SWAY_X_AMP: f32 = 0.005;
override SWAY_X_BASE: f32 = 0.5;
override SWAY_X_GROWTH: f32 = 2.0;
override SWAY_Z_WAVE: f32 = 10.0;
override SWAY_Z_RATE: f32 = 1.0;
override SWAY_Z_AMP: f32 = 0.005;
override SWAY_Z_BASE: f32 = 0.5;
override SWAY_Z_GROWTH: f32 = 2.0;
override FADE_IN: f32 = 0.01;
override FADE_OUT_START: f32 = 0.05;
override FADE_OUT_END: f32 = 0.15;
override DEPTH_PER_WIDTH: f32 = 6.0;
override GRAIN_H: f32 = 60.0;
override GRAIN_V: f32 = 40.0;
override LIFT_RATE: f32 = 0.05;
override WARP_AMP: f32 = 2.0;
override WARP_RATE: f32 = 0.2;
override CHURN: f32 = 0.6;
override THRESHOLD_LOW: f32 = 0.4;
override THRESHOLD_HIGH: f32 = 0.85;
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var scene_depth: texture_2d<f32>;
@group(0) @binding(2) var shadow_atlas: texture_depth_2d_array;
@group(0) @binding(3) var<uniform> shadow: Shadow;
@group(0) @binding(4) var room_field: texture_3d<f32>;
@group(0) @binding(5) var room_sampler: sampler;
struct Out { @builtin(position) clip: vec4f, @location(0) uv: vec2f, }
@vertex fn vs_main(@builtin(vertex_index) vertex: u32) -> Out {
    let xy = array<vec2f, 3>(vec2f(-1.0,-1.0),vec2f(3.0,-1.0),vec2f(-1.0,3.0));
    var out: Out;
    out.clip = vec4f(xy[vertex], 0.0, 1.0);
    out.uv = xy[vertex] * vec2f(0.5,-0.5) + vec2f(0.5);
    return out;
}
fn pcg(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let mixed = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (mixed >> 22u) ^ mixed;
}
fn hash3(p: vec3i) -> f32 {
    let seed = select(0u, bitcast<u32>(frame.extent.w), frame.volume_control.w > 0.5);
    let h = pcg(u32(p.x) * 73856093u ^ u32(p.y) * 19349663u ^ u32(p.z) * 83492791u ^ seed);
    return f32(h >> 8u) / 16777216.0;
}
fn noise3(p: vec3f) -> f32 {
    let i = vec3i(floor(p));
    let f = fract(p);
    let t = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(hash3(i), hash3(i + vec3i(1,0,0)), t.x), mix(hash3(i + vec3i(0,1,0)), hash3(i + vec3i(1,1,0)), t.x), t.y);
    let b = mix(mix(hash3(i + vec3i(0,0,1)), hash3(i + vec3i(1,0,1)), t.x), mix(hash3(i + vec3i(0,1,1)), hash3(i + vec3i(1,1,1)), t.x), t.y);
    return mix(a, b, t.z);
}
fn fbm(p: vec3f, octaves: i32) -> f32 {
    var sum = 0.0;
    var amplitude = 0.55;
    var q = p;
    for (var i = 0; i < octaves; i++) {
        sum += noise3(q) * amplitude;
        q = q * 2.07 + vec3f(1.7, 9.2, 3.1);
        amplitude *= 0.5;
    }
    return sum * (1.03125 / (1.1 * (1.0 - pow(0.5, f32(octaves)))));
}
fn density(p: vec3f) -> f32 {
    let rise = p.y - frame.volume_lo.y;
    let lift = p.y - frame.plume[0].y;
    let smoke = frame.volume_control.w > 0.5;
    let width = select(max(frame.volume_hi.z - frame.volume_lo.z, 0.000001), max(frame.volume_hi.x - frame.volume_lo.x, 0.000001), smoke);
    let origin = frame.plume[0].xyz;
    let drift = vec3f(DRIFT_X_QUAD * lift * lift + DRIFT_X_LIN * lift, 0.0, DRIFT_Z_QUAD * lift * lift + DRIFT_Z_LIN * lift);
    let sway = vec3f(sin(lift * SWAY_X_WAVE - frame.extent.z * SWAY_X_RATE) * SWAY_X_AMP * (SWAY_X_BASE + lift * SWAY_X_GROWTH), 0.0, cos(lift * SWAY_Z_WAVE - frame.extent.z * SWAY_Z_RATE) * SWAY_Z_AMP * (SWAY_Z_BASE + lift * SWAY_Z_GROWTH));
    let steam_centre = origin + vec3f(0.0, lift, 0.0) + drift + sway;
    let smoke_rise = clamp(rise / max(frame.volume_hi.y - frame.volume_lo.y, 0.001), 0.0, 1.0);
    let smoke_centre = (frame.volume_lo.xyz + frame.volume_hi.xyz) * 0.5 + vec3f(sin(smoke_rise * 9.0 - frame.extent.z * 1.3) * width * 0.29 * smoke_rise, 0.0, cos(smoke_rise * 7.0 - frame.extent.z) * width * 0.21 * smoke_rise);
    let centre = select(steam_centre, smoke_centre, smoke);
    let radius = select(frame.plume[0].w * (SPREAD_BASE + lift * SPREAD_GROWTH), width * (0.4 + smoke_rise * 0.25), smoke);
    let core = exp(-dot((p - centre).xz, (p - centre).xz) / max(radius * radius, 0.00000001));
    let steam_fade = smoothstep(0.0, FADE_IN, lift) * (1.0 - smoothstep(FADE_OUT_START, FADE_OUT_END, lift));
    if (EARLY_OUT && !smoke && core * steam_fade < 0.002) { return 0.0; }
    let q = vec3f(p.x * GRAIN_H, (p.y - frame.extent.z * LIFT_RATE) * GRAIN_V, p.z * GRAIN_H);
    let warp = vec3f(fbm(q * 0.5 + vec3f(frame.extent.z * WARP_RATE, 0.0, 0.0), WARP_OCTAVES), 0.0, fbm(q * 0.5 + vec3f(3.1, 0.0, frame.extent.z * WARP_RATE), WARP_OCTAVES)) * WARP_AMP;
    let steam_turbulence = smoothstep(THRESHOLD_LOW, THRESHOLD_HIGH, fbm(q + warp + vec3f(0.0, -frame.extent.z * CHURN, 0.0), OCTAVES));
    let smoke_q = p * 18.0 + vec3f(0.0, -frame.extent.z * 0.7, 0.0);
    let smoke_turbulence = smoothstep(0.3, 0.72, noise3(smoke_q) * 0.65 + noise3(smoke_q * 2.07 + vec3f(1.7,9.2,3.1)) * 0.35);
    let turbulence = select(steam_turbulence, 0.25 + 0.75 * smoke_turbulence, smoke);
    let smoke_fade = smoothstep(0.0, 0.1, smoke_rise) * (1.0 - smoothstep(0.85, 1.0, smoke_rise));
    let fade = select(steam_fade, smoke_fade, smoke);
    if (!smoke && core * fade < 0.002) { return 0.0; }
    return core * turbulence * fade * frame.volume_control.x;
}
fn phase(cosine: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cosine, 0.001), 1.5);
}
struct VolumeOut { @location(0) radiance: vec4f, @location(1) distance: f32, }
fn room_density(p: vec3f) -> f32 {
    let drift = vec3f(0.011, 0.0035, -0.004) * frame.extent.z * 1.6;
    let uvw = (p + drift - frame.room_lo.xyz) / max(frame.room_hi.xyz - frame.room_lo.xyz, vec3f(0.0001));
    var field = vec4f(0.4375, 0.0, 0.46875, 0.4375);
    if (all(uvw >= vec3f(0.0)) && all(uvw <= vec3f(1.0))) {
        field = textureSampleLevel(room_field, room_sampler, uvw, 0.0);
    }
    let behind = smoothstep(0.45, 0.02, p.z);
    let top = smoothstep(0.2, 0.46, p.y) * (0.35 + 0.9 * field.z);
    let low = exp(-max(p.y, 0.0) / 0.035) * (0.3 + 0.9 * field.w);
    return frame.volume_control.x * (0.5 + field.x) + behind * (frame.volume_hi.w * field.y + frame.volume_color.w * top) + frame.volume_lo.w * low;
}
fn room_sunlight(world: vec3f, distance: f32) -> f32 {
    if (shadow.splits.w <= 0.0) { return 1.0; }
    var layer = 0;
    var matrix = shadow.view_proj_0;
    if (distance > shadow.splits.y) { layer = 1; matrix = shadow.view_proj_1; }
    if (distance > shadow.splits.z) { layer = 2; matrix = shadow.view_proj_2; }
    let clip = matrix * vec4f(world, 1.0);
    let point = clip.xyz / clip.w;
    let uv = point.xy * vec2f(0.5, -0.5) + vec2f(0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || point.z < 0.0 || point.z > 1.0) { return frame.room_shape.y; }
    let size = textureDimensions(shadow_atlas);
    let at = clamp(vec2i(uv * vec2f(size)), vec2i(0), vec2i(size) - vec2i(1));
    return select(0.0, 1.0, point.z - shadow.sun.w <= textureLoad(shadow_atlas, at, layer, 0));
}
fn room_phase(cosine: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cosine;
    return (1.0 - g * g) / (12.566371 * d * sqrt(d));
}
fn room(low: vec2f, limit: f32, ray: vec3f) -> VolumeOut {
    var out: VolumeOut;
    out.distance = limit;
    let along = max(dot(ray, normalize(frame.forward.xyz)), 0.001);
    let span = min(limit / along, frame.room_shape.x);
    if (span <= 0.0) {
        out.radiance = vec4f(0.0);
        return out;
    }
    let steps = 6u;
    let step_length = span / f32(steps);
    let turn = f32(pcg(bitcast<u32>(frame.extent.z) ^ bitcast<u32>(frame.extent.w)) >> 8u) / 16777216.0;
    let jitter = fract(fract(52.9829189 * fract(dot(floor(low), vec2f(0.06711056, 0.00583715)))) + turn);
    let sun = normalize(frame.sun.xyz);
    let cosine = dot(ray, sun);
    let phase = mix(room_phase(cosine, frame.volume_control.y), room_phase(cosine, -0.55), frame.room_light.w);
    let direct = frame.sun_color.rgb * frame.sun_color.w * frame.room_light.rgb * phase;
    let fog = frame.volume_control.x;
    var light = vec3f(0.0);
    for (var i = 0u; i < steps; i++) {
        let distance = (f32(i) + jitter) * step_length;
        let p = frame.eye.xyz + ray * distance;
        let density = room_density(p);
        if (density <= 0.0) { continue; }
        light += density * exp(-fog * distance) * (direct * room_sunlight(p, distance * along) + frame.volume_color.rgb) * step_length;
    }
    out.radiance = vec4f(light, 1.0 - exp(-fog * span));
    return out;
}
@fragment fn fs_main(in: Out) -> VolumeOut {
    let screen = vec2i(clamp(in.uv * frame.extent.xy, vec2f(0.0), frame.extent.xy - vec2f(1.0)));
    let scene_limit = textureLoad(scene_depth, screen, 0).x;
    let limit = select(1e30, scene_limit, frame.volume_control.w > 0.5);
    let centre_clip = frame.view_proj * vec4f(frame.eye.xyz + frame.forward.xyz, 1.0);
    let centre = centre_clip.xy / centre_clip.w;
    let ray = normalize(frame.forward.xyz + frame.right.xyz * ((in.uv.x * 2.0 - 1.0 - centre.x) * frame.lens.x) + frame.up.xyz * ((1.0 - in.uv.y * 2.0 - centre.y) * frame.lens.y));
    if (frame.volume_control.w > 1.5) { return room(in.clip.xy, scene_limit, ray); }
    let safe = select(ray, select(vec3f(-0.00001), vec3f(0.00001), ray >= vec3f(0.0)), abs(ray) < vec3f(0.00001));
    let t0 = (frame.volume_lo.xyz - frame.eye.xyz) / safe;
    let t1 = (frame.volume_hi.xyz - frame.eye.xyz) / safe;
    let enter = max(max(min(t0.x,t1.x),min(t0.y,t1.y)),max(min(t0.z,t1.z),0.0));
    let leave = min(min(max(t0.x,t1.x),max(t0.y,t1.y)),min(max(t0.z,t1.z),limit));
    var out: VolumeOut;
    out.radiance = vec4f(0.0);
    out.distance = scene_limit;
    if (leave <= enter) { return out; }
    let steps = STEPS;
    let step_length = (leave - enter) / f32(steps);
    let sun = normalize(frame.sun.xyz);
    let scatter = frame.volume_color.rgb * (vec3f(0.32) + frame.sun_color.rgb * frame.sun_color.w * phase(dot(ray,sun), frame.volume_control.y)) * 0.85;
    var transmittance = 1.0;
    var light = vec3f(0.0);
    for (var i = 0u; i < steps; i++) {
        let distance = enter + (f32(i) + 0.5) * step_length;
        let p = frame.eye.xyz + ray * distance;
        let width = select(max(frame.volume_hi.z - frame.volume_lo.z, 0.000001), max(frame.volume_hi.x - frame.volume_lo.x, 0.000001), frame.volume_control.w > 0.5);
        let scale = select(DEPTH_PER_WIDTH, 3.0, frame.volume_control.w > 0.5);
        let absorbed = 1.0 - exp(-density(p) * step_length * scale / width);
        let weight = transmittance * absorbed;
        light += weight * scatter;
        transmittance *= 1.0 - absorbed;
    }
    out.radiance = vec4f(light, 1.0 - transmittance);
    return out;
}
"#;

pub const ROOM_FIELD_WGSL: &str = r#"
struct Field { lo: vec4f, hi: vec4f, size: vec4u, }
@group(0) @binding(0) var<uniform> field: Field;
@group(0) @binding(1) var target_field: texture_storage_3d<rgba8unorm, write>;
fn pcg(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let mixed = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (mixed >> 22u) ^ mixed;
}
fn hash3(p: vec3i) -> f32 {
    let h = pcg(bitcast<u32>(p.x) * 73856093u ^ pcg(bitcast<u32>(p.y) * 19349663u ^ pcg(bitcast<u32>(p.z) * 83492791u ^ field.size.w)));
    return f32(h >> 8u) / 16777216.0;
}
fn vnoise(p: vec3f) -> f32 {
    let i = vec3i(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash3(i), hash3(i + vec3i(1, 0, 0)), u.x);
    let b = mix(hash3(i + vec3i(0, 1, 0)), hash3(i + vec3i(1, 1, 0)), u.x);
    let c = mix(hash3(i + vec3i(0, 0, 1)), hash3(i + vec3i(1, 0, 1)), u.x);
    let d = mix(hash3(i + vec3i(0, 1, 1)), hash3(i + vec3i(1, 1, 1)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}
fn fbm(p: vec3f, octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < octaves; i++) {
        sum += amp * vnoise(q);
        q = q * 2.03 + vec3f(1.7, 9.2, 3.1);
        amp *= 0.5;
    }
    return sum;
}
@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if (any(id >= field.size.xyz)) { return; }
    let p = field.lo.xyz + (vec3f(id) + vec3f(0.5)) / vec3f(field.size.xyz) * (field.hi.xyz - field.lo.xyz);
    let w = p * 2.4;
    let warp = vec3f(fbm(w + vec3f(1.7, 0.0, 0.0), 3), fbm(w + vec3f(0.0, 9.2, 0.0), 3), fbm(w + vec3f(0.0, 0.0, 4.4), 3)) - vec3f(0.5);
    let q = p + warp * 0.18;
    let body = fbm(q * vec3f(2.6, 4.0, 2.6), 3);
    let ridge = 1.0 - abs(fbm(q * vec3f(7.0, 16.0, 7.0) + vec3f(11.0), 4) * 2.0 - 1.0);
    let curl = pow(ridge, 6.0) * smoothstep(0.42, 0.7, body);
    let top = fbm(q * vec3f(3.0, 6.0, 3.0) + vec3f(5.0), 4);
    let low = fbm(q * vec3f(5.0, 12.0, 5.0) + vec3f(3.0), 3);
    textureStore(target_field, vec3i(id), vec4f(body, curl, top, low));
}
"#;

pub const ROOM_FIELD_SPACING: f32 = 0.012;

pub const ROOM_BLUR_WGSL: &str = r#"
struct Frame {
    view_proj: mat4x4f, eye: vec4f, right: vec4f, up: vec4f, forward: vec4f,
    lens: vec4f, sun: vec4f, sun_color: vec4f, extent: vec4f,
    volume_lo: vec4f, volume_hi: vec4f, volume_color: vec4f, volume_control: vec4f,
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(2) var low_color: texture_2d<f32>;
@group(0) @binding(3) var low_depth: texture_2d<f32>;
@vertex fn vs_main(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0,-1.0),vec2f(3.0,-1.0),vec2f(-1.0,3.0));
    return vec4f(xy[vertex], 0.0, 1.0);
}
override low_scale: f32 = 4.0;
@fragment fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let pixel = vec2i(position.xy);
    let last = vec2i(min(vec2f(textureDimensions(low_color)), ceil(frame.extent.xy / low_scale))) - vec2i(1);
    let centre = textureLoad(low_depth, pixel, 0).x;
    let edge = 0.01 + 0.05 * centre;
    var total = vec4f(0.0);
    var weight = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let at = clamp(pixel + vec2i(x, y), vec2i(0), last);
            let depth = textureLoad(low_depth, at, 0).x;
            let w = exp(-f32(x * x + y * y) * 0.5) * select(0.0, 1.0, abs(depth - centre) < edge);
            total += textureLoad(low_color, at, 0) * w;
            weight += w;
        }
    }
    return total / max(weight, 0.000001);
}
"#;

pub const UPSAMPLE_WGSL: &str = r#"
struct Frame {
    view_proj: mat4x4f, eye: vec4f, right: vec4f, up: vec4f, forward: vec4f,
    lens: vec4f, sun: vec4f, sun_color: vec4f, extent: vec4f,
    volume_lo: vec4f, volume_hi: vec4f, volume_color: vec4f, volume_control: vec4f,
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var scene_depth: texture_2d<f32>;
@group(0) @binding(2) var low_color: texture_2d<f32>;
@group(0) @binding(3) var low_depth: texture_2d<f32>;
@vertex fn vs_main(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0,-1.0),vec2f(3.0,-1.0),vec2f(-1.0,3.0));
    return vec4f(xy[vertex], 0.0, 1.0);
}
override low_scale: f32 = 4.0;
@fragment fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = min(vec2f(textureDimensions(low_color)), ceil(frame.extent.xy / low_scale));
    let point = position.xy / frame.extent.xy * size - vec2f(0.5);
    let base = vec2i(floor(point));
    let fraction = fract(point);
    let max_pixel = vec2i(size) - vec2i(1);
    let full = textureLoad(scene_depth, vec2i(position.xy), 0).x;
    var total = 0.0;
    var result = vec4f(0.0);
    var nearest = 1e30;
    var nearest_color = vec4f(0.0);
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let pixel = clamp(base + vec2i(x,y), vec2i(0), max_pixel);
            let depth = textureLoad(low_depth, pixel, 0).x;
            let difference = abs(full - depth);
            let color = textureLoad(low_color, pixel, 0);
            if (difference < nearest) {
                nearest = difference;
                nearest_color = color;
            }
            let spatial = select(1.0 - fraction.x, fraction.x, x == 1) * select(1.0 - fraction.y, fraction.y, y == 1);
            let weight = spatial * exp(-difference * frame.volume_control.z);
            result += color * weight;
            total += weight;
        }
    }
    return select(nearest_color, result / max(total, 0.000001), total > 0.000001);
}
"#;

fn uniform(device: &wgpu::Device, frame: &EffectFrame) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("effects frame"),
        contents: bytemuck::bytes_of(frame),
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

fn storage<T: Pod>(device: &wgpu::Device, label: &'static str, values: &[T]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(values),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

fn buffer_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn shadow_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn pipeline(
    device: &wgpu::Device,
    label: &'static str,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    targets: &[Option<wgpu::ColorTargetState>],
) -> wgpu::RenderPipeline {
    pipeline_with(device, label, source, layout, targets, &[])
}

fn pipeline_with(
    device: &wgpu::Device,
    label: &'static str,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    targets: &[Option<wgpu::ColorTargetState>],
    constants: &[(&str, f64)],
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
            targets,
        }),
        multiview: None,
        cache: None,
    })
}

fn premultiplied_target() -> Option<wgpu::ColorTargetState> {
    Some(wgpu::ColorTargetState {
        format: HDR_FORMAT,
        blend: Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        }),
        write_mask: wgpu::ColorWrites::ALL,
    })
}

fn color_attachment(view: &wgpu::TextureView, clear: bool) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        ops: wgpu::Operations {
            load: if clear {
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
            } else {
                wgpu::LoadOp::Load
            },
            store: wgpu::StoreOp::Store,
        },
        depth_slice: None,
    }
}

pub struct CreatureMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

pub struct EffectCommands<'a> {
    pub device: &'a wgpu::Device,
    pub encoder: &'a mut wgpu::CommandEncoder,
}

#[derive(Clone, Copy)]
pub struct EffectTargets<'a> {
    pub hdr: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub shadow: Option<(&'a wgpu::TextureView, &'a wgpu::Buffer)>,
}

pub struct VolumeTimestamps<'a> {
    pub march: Option<wgpu::RenderPassTimestampWrites<'a>>,
    pub upsample: Option<wgpu::RenderPassTimestampWrites<'a>>,
}

impl CreatureMesh {
    pub fn new(device: &wgpu::Device, vertices: &[CreatureVertex], indices: &[u32]) -> Self {
        let index_count = indices.len() as u32;
        let vertices = storage(device, "creature vertices", vertices);
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("creature indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vertices,
            indices,
            index_count,
        }
    }

    pub fn shadow_buffers(&self) -> (&wgpu::Buffer, &wgpu::Buffer, u32) {
        (&self.vertices, &self.indices, self.index_count)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeCuts {
    pub steps: u32,
    pub warp_octaves: u32,
    pub octaves: u32,
    pub early_out: bool,
}

impl VolumeCuts {
    pub const FULL: Self = Self {
        steps: 20,
        warp_octaves: 4,
        octaves: 4,
        early_out: false,
    };

    pub const DEFAULT: Self = Self {
        early_out: true,
        ..Self::FULL
    };

    fn constants(&self) -> [(&'static str, f64); 4] {
        [
            ("STEPS", f64::from(self.steps.max(1))),
            ("WARP_OCTAVES", f64::from(self.warp_octaves.clamp(1, 4))),
            ("OCTAVES", f64::from(self.octaves.clamp(1, 4))),
            ("EARLY_OUT", f64::from(u8::from(self.early_out))),
        ]
    }
}

impl Default for VolumeCuts {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn volume_targets() -> [Option<wgpu::ColorTargetState>; 2] {
    [
        Some(wgpu::ColorTargetState {
            format: HDR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        }),
        Some(wgpu::ColorTargetState {
            format: SCENE_DEPTH_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        }),
    ]
}

pub struct Effects {
    cuts: VolumeCuts,
    look: Plume,
    cached_volumes: Vec<(VolumeCuts, [u32; 28], wgpu::RenderPipeline)>,
    width: u32,
    height: u32,
    viewport: (u32, u32),
    scale: u32,
    _low_color: wgpu::Texture,
    low_color_view: wgpu::TextureView,
    _low_depth: wgpu::Texture,
    low_depth_view: wgpu::TextureView,
    _low_blur: wgpu::Texture,
    low_blur_view: wgpu::TextureView,
    _empty_shadow: wgpu::Texture,
    empty_shadow_view: wgpu::TextureView,
    empty_shadow_data: wgpu::Buffer,
    particle_layout: wgpu::BindGroupLayout,
    creature_layout: wgpu::BindGroupLayout,
    volume_layout: wgpu::BindGroupLayout,
    upsample_layout: wgpu::BindGroupLayout,
    particle_pipeline: wgpu::RenderPipeline,
    creature_pipeline: wgpu::RenderPipeline,
    upsample_pipeline: wgpu::RenderPipeline,
    room_blur_pipeline: wgpu::RenderPipeline,
    room_field_layout: wgpu::BindGroupLayout,
    room_field_pipeline: wgpu::ComputePipeline,
    room_sampler: wgpu::Sampler,
    _empty_field: wgpu::Texture,
    empty_field_view: wgpu::TextureView,
    room_field: Option<RoomField>,
}

struct RoomField {
    key: ([u32; 3], [u32; 3], u32),
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn field_key(haze: &RoomHaze) -> ([u32; 3], [u32; 3], u32) {
    (
        haze.lo.map(f32::to_bits),
        haze.hi.map(f32::to_bits),
        haze.seed,
    )
}

pub fn room_field_size(haze: &RoomHaze) -> [u32; 3] {
    std::array::from_fn(|axis| {
        (((haze.hi[axis] - haze.lo[axis]) / ROOM_FIELD_SPACING).ceil() as u32).clamp(1, 256)
    })
}

fn field_texture(
    device: &wgpu::Device,
    size: [u32; 3],
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("effects room haze field"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: size[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[],
    })
}

impl Effects {
    pub fn new(device: &wgpu::Device, width: u32, height: u32, scale: u32) -> Result<Self, String> {
        if width == 0 || height == 0 || !matches!(scale, 2 | 4) {
            return Err("effects need nonzero dimensions and half or quarter resolution".into());
        }
        let low_width = width.div_ceil(scale);
        let low_height = height.div_ceil(scale);
        let texture = |label, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: low_width,
                    height: low_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let low_color = texture("effects low color", HDR_FORMAT);
        let low_color_view = low_color.create_view(&Default::default());
        let low_depth = texture("effects low depth", SCENE_DEPTH_FORMAT);
        let low_depth_view = low_depth.create_view(&Default::default());
        let low_blur = texture("effects low blur", HDR_FORMAT);
        let low_blur_view = low_blur.create_view(&Default::default());
        let empty_shadow = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("effects empty shadow"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 3,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let empty_shadow_view = empty_shadow.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let empty_shadow_data = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("effects empty shadow data"),
            contents: &[0; 256],
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let particle_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle bindings"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                texture_entry(1),
                buffer_entry(
                    2,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                shadow_entry(3),
                buffer_entry(
                    4,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
            ],
        });
        let creature_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("creature bindings"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BufferBindingType::Uniform,
                ),
                buffer_entry(
                    1,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer_entry(
                    2,
                    wgpu::ShaderStages::VERTEX,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                texture_entry(3),
            ],
        });
        let volume_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("volume bindings"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                texture_entry(1),
                shadow_entry(2),
                buffer_entry(
                    3,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let upsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upsample bindings"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
            ],
        });
        let particle_pipeline = pipeline(
            device,
            "effects particles",
            PARTICLE_WGSL,
            &particle_layout,
            &[premultiplied_target()],
        );
        let creature_pipeline = pipeline(
            device,
            "effects creatures",
            CREATURE_WGSL,
            &creature_layout,
            &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        );
        let cuts = VolumeCuts::default();
        let look = Plume::default();
        let cached_volumes = vec![(
            cuts,
            look.key(),
            pipeline_with(
                device,
                "effects volume",
                VOLUME_WGSL,
                &volume_layout,
                &volume_targets(),
                &volume_constants(cuts, &look),
            ),
        )];
        let low_scale = [("low_scale", f64::from(scale))];
        let upsample_pipeline = pipeline_with(
            device,
            "effects upsample",
            UPSAMPLE_WGSL,
            &upsample_layout,
            &[premultiplied_target()],
            &low_scale,
        );
        let room_blur_pipeline = pipeline_with(
            device,
            "effects room haze blur",
            ROOM_BLUR_WGSL,
            &upsample_layout,
            &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            &low_scale,
        );
        let room_field_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("room haze field bindings"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Uniform,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D3,
                    },
                    count: None,
                },
            ],
        });
        let room_field_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("effects room haze field"),
            source: wgpu::ShaderSource::Wgsl(ROOM_FIELD_WGSL.into()),
        });
        let room_field_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("effects room haze field"),
                bind_group_layouts: &[&room_field_layout],
                push_constant_ranges: &[],
            });
        let room_field_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("effects room haze field"),
                layout: Some(&room_field_pipeline_layout),
                module: &room_field_module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let room_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("effects room haze field"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let empty_field = field_texture(device, [1, 1, 1], wgpu::TextureUsages::TEXTURE_BINDING);
        let empty_field_view = empty_field.create_view(&Default::default());
        Ok(Self {
            cuts,
            look,
            cached_volumes,
            width,
            height,
            viewport: (width, height),
            scale,
            _low_color: low_color,
            low_color_view,
            _low_depth: low_depth,
            low_depth_view,
            _low_blur: low_blur,
            low_blur_view,
            _empty_shadow: empty_shadow,
            empty_shadow_view,
            empty_shadow_data,
            particle_layout,
            creature_layout,
            volume_layout,
            upsample_layout,
            particle_pipeline,
            creature_pipeline,
            upsample_pipeline,
            room_blur_pipeline,
            room_field_layout,
            room_field_pipeline,
            room_sampler,
            _empty_field: empty_field,
            empty_field_view,
            room_field: None,
        })
    }

    pub fn dimensions(&self) -> (u32, u32, u32) {
        (self.width, self.height, self.scale)
    }

    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    pub fn textures(&self) -> Vec<wgpu::Texture> {
        vec![
            self._low_color.clone(),
            self._low_depth.clone(),
            self._low_blur.clone(),
        ]
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        crate::viewport::check([width, height], [self.width, self.height])?;
        self.viewport = (width, height);
        Ok(())
    }

    fn low_viewport(&self) -> [u32; 2] {
        [
            self.viewport.0.div_ceil(self.scale),
            self.viewport.1.div_ceil(self.scale),
        ]
    }

    pub fn volume_cuts(&self) -> VolumeCuts {
        self.cuts
    }

    pub fn set_volume_cuts(&mut self, device: &wgpu::Device, cuts: VolumeCuts) {
        self.cuts = cuts;
        let look = self.look;
        self.ensure_volume(device, &look);
    }

    pub fn set_plume(&mut self, device: &wgpu::Device, plume: &Plume) {
        self.ensure_volume(device, plume);
    }

    fn ensure_volume(&mut self, device: &wgpu::Device, plume: &Plume) {
        let key = plume.key();
        self.look = *plume;
        if self
            .cached_volumes
            .iter()
            .any(|(known, look, _)| *known == self.cuts && *look == key)
        {
            return;
        }
        if self.cached_volumes.len() >= MAX_VOLUME_PIPELINES {
            self.cached_volumes.remove(0);
        }
        let built = pipeline_with(
            device,
            "effects volume",
            VOLUME_WGSL,
            &self.volume_layout,
            &volume_targets(),
            &volume_constants(self.cuts, plume),
        );
        self.cached_volumes.push((self.cuts, key, built));
    }

    pub fn prepare_room(&mut self, commands: &mut EffectCommands<'_>, haze: &RoomHaze) {
        let key = field_key(haze);
        if self
            .room_field
            .as_ref()
            .is_some_and(|field| field.key == key)
        {
            return;
        }
        let size = room_field_size(haze);
        let texture = field_texture(
            commands.device,
            size,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        );
        let view = texture.create_view(&Default::default());
        let mut values = [0u32; 12];
        for axis in 0..3 {
            values[axis] = haze.lo[axis].to_bits();
            values[4 + axis] = haze.hi[axis].to_bits();
            values[8 + axis] = size[axis];
        }
        values[11] = haze.seed;
        let uniform = commands
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("room haze field box"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = commands
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("room haze field bindings"),
                layout: &self.room_field_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            });
        {
            let mut pass = commands
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("effects room haze field"),
                    timestamp_writes: None,
                });
            pass.set_pipeline(&self.room_field_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(
                size[0].div_ceil(4),
                size[1].div_ceil(4),
                size[2].div_ceil(4),
            );
        }
        self.room_field = Some(RoomField {
            key,
            _texture: texture,
            view,
        });
    }

    pub fn render_volume(
        &mut self,
        commands: &mut EffectCommands<'_>,
        frame: &EffectFrame,
        targets: EffectTargets<'_>,
        timestamps: VolumeTimestamps<'_>,
    ) {
        self.ensure_volume(commands.device, &frame.plume);
        let look_key = frame.plume.key();
        let (full_width, full_height) = self.viewport;
        let (left, top, width, height) = volume_bounds(frame, full_width, full_height).unwrap_or((
            0,
            0,
            full_width,
            full_height,
        ));
        let low_left = left / self.scale;
        let low_top = top / self.scale;
        let low_right = (left + width).div_ceil(self.scale);
        let low_bottom = (top + height).div_ceil(self.scale);
        let uniform = uniform(commands.device, frame);
        let volume_bind = commands
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("volume bindings"),
                layout: &self.volume_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(targets.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(
                            targets
                                .shadow
                                .map_or(&self.empty_shadow_view, |pair| pair.0),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: targets
                            .shadow
                            .map_or(&self.empty_shadow_data, |pair| pair.1)
                            .as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(
                            self.room_field
                                .as_ref()
                                .map_or(&self.empty_field_view, |field| &field.view),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&self.room_sampler),
                    },
                ],
            });
        {
            let mut pass = commands
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("effects volume quarter resolution"),
                    color_attachments: &[
                        Some(color_attachment(&self.low_color_view, true)),
                        Some(wgpu::RenderPassColorAttachment {
                            view: &self.low_depth_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        }),
                    ],
                    depth_stencil_attachment: None,
                    timestamp_writes: timestamps.march,
                    occlusion_query_set: None,
                });
            let volume_pipeline = self
                .cached_volumes
                .iter()
                .find(|(known, look, _)| *known == self.cuts && *look == look_key)
                .map(|(_, _, built)| built)
                .expect("the active volume cuts and plume have a pipeline");
            crate::viewport::apply(&mut pass, self.low_viewport());
            pass.set_pipeline(volume_pipeline);
            pass.set_bind_group(0, &volume_bind, &[]);
            pass.set_scissor_rect(
                low_left,
                low_top,
                low_right - low_left,
                low_bottom - low_top,
            );
            pass.draw(0..3, 0..1);
        }
        let room = frame.volume_control[3] > 1.5;
        let (blur_writes, upsample_writes) = match (room, timestamps.upsample) {
            (true, Some(writes)) => (
                Some(wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: writes.beginning_of_pass_write_index,
                    end_of_pass_write_index: None,
                }),
                Some(wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: writes.end_of_pass_write_index,
                }),
            ),
            (_, writes) => (None, writes),
        };
        if room {
            let blur_bind = commands
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("room haze blur bindings"),
                    layout: &self.upsample_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&self.low_depth_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&self.low_color_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&self.low_depth_view),
                        },
                    ],
                });
            let mut pass = commands
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("effects room haze blur"),
                    color_attachments: &[Some(color_attachment(&self.low_blur_view, true))],
                    depth_stencil_attachment: None,
                    timestamp_writes: blur_writes,
                    occlusion_query_set: None,
                });
            crate::viewport::apply(&mut pass, self.low_viewport());
            pass.set_pipeline(&self.room_blur_pipeline);
            pass.set_bind_group(0, &blur_bind, &[]);
            pass.set_scissor_rect(
                low_left,
                low_top,
                low_right - low_left,
                low_bottom - low_top,
            );
            pass.draw(0..3, 0..1);
        }
        let low_color = if room {
            &self.low_blur_view
        } else {
            &self.low_color_view
        };
        let upsample_bind = commands
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("volume upsample bindings"),
                layout: &self.upsample_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(targets.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(low_color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&self.low_depth_view),
                    },
                ],
            });
        let mut pass = commands
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effects depth aware upsample"),
                color_attachments: &[Some(color_attachment(targets.hdr, false))],
                depth_stencil_attachment: None,
                timestamp_writes: upsample_writes,
                occlusion_query_set: None,
            });
        crate::viewport::apply(&mut pass, [full_width, full_height]);
        pass.set_pipeline(&self.upsample_pipeline);
        pass.set_bind_group(0, &upsample_bind, &[]);
        pass.set_scissor_rect(left, top, width, height);
        pass.draw(0..3, 0..1);
    }

    pub fn render_particles(
        &self,
        commands: &mut EffectCommands<'_>,
        frame: &EffectFrame,
        targets: EffectTargets<'_>,
        particles: &[ParticleInstance],
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        if particles.is_empty() {
            return;
        }
        let uniform = uniform(commands.device, frame);
        let mut ordered = particles.to_vec();
        ordered.sort_by(|left, right| {
            distance_squared(right.position_size, frame.eye)
                .total_cmp(&distance_squared(left.position_size, frame.eye))
        });
        let instances = storage(commands.device, "particle instances", &ordered);
        let bind = commands
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particle bindings"),
                layout: &self.particle_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(targets.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: instances.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(
                            targets
                                .shadow
                                .map_or(&self.empty_shadow_view, |pair| pair.0),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: targets
                            .shadow
                            .map_or(&self.empty_shadow_data, |pair| pair.1)
                            .as_entire_binding(),
                    },
                ],
            });
        let mut pass = commands
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effects particles"),
                color_attachments: &[Some(color_attachment(targets.hdr, false))],
                depth_stencil_attachment: None,
                timestamp_writes: timestamps,
                occlusion_query_set: None,
            });
        crate::viewport::apply(&mut pass, [self.viewport.0, self.viewport.1]);
        pass.set_pipeline(&self.particle_pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..6, 0..particles.len() as u32);
    }

    pub fn render_creatures(
        &self,
        commands: &mut EffectCommands<'_>,
        frame: &EffectFrame,
        targets: EffectTargets<'_>,
        mesh: &CreatureMesh,
        creatures: &[CreatureInstance],
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        if creatures.is_empty() || mesh.index_count == 0 {
            return;
        }
        let uniform = uniform(commands.device, frame);
        let mut ordered = creatures.to_vec();
        ordered.sort_by(|left, right| {
            distance_squared(right.position_scale, frame.eye)
                .total_cmp(&distance_squared(left.position_scale, frame.eye))
        });
        let instances = storage(commands.device, "creature instances", &ordered);
        let bind = commands
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("creature bindings"),
                layout: &self.creature_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: instances.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: mesh.vertices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(targets.depth),
                    },
                ],
            });
        let mut pass = commands
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effects creatures"),
                color_attachments: &[Some(color_attachment(targets.hdr, false))],
                depth_stencil_attachment: None,
                timestamp_writes: timestamps,
                occlusion_query_set: None,
            });
        crate::viewport::apply(&mut pass, [self.viewport.0, self.viewport.1]);
        pass.set_pipeline(&self.creature_pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.index_count, 0, 0..creatures.len() as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_gpu::{Gpu, GpuProfiler};

    const LO: [f32; 3] = [-1.0, 0.0, -1.0];
    const HI: [f32; 3] = [1.0, 1.0, 1.0];

    #[test]
    fn haze_from_zero_amount_has_no_density() {
        let haze = RoomHaze::from_amount(LO, HI, 0.0);
        assert_eq!([haze.fog, haze.smoke, haze.mist, haze.floor], [0.0; 4]);
        let mut frame = EffectFrame::new(8, 8, 0.0, 0);
        frame.set_room_haze(haze);
        assert_eq!(frame.volume_control[0], 0.0);
        assert_eq!(frame.volume_lo[3], 0.0);
        assert_eq!(frame.volume_hi[3], 0.0);
        assert_eq!(frame.volume_color[3], 0.0);
        assert_eq!(RoomHaze::from_amount(LO, HI, -1.0), haze);
        assert_eq!(RoomHaze::from_amount(LO, HI, f32::NAN), haze);
    }

    #[test]
    fn haze_from_amount_takes_the_bounds_and_neutral_defaults() {
        let haze = RoomHaze::from_amount(LO, HI, 0.3);
        let neutral = RoomHaze::new(LO, HI);
        assert!((haze.fog - 0.05).abs() < 1.0e-6);
        assert!((haze.smoke - 6.0).abs() < 1.0e-5);
        assert!((haze.floor - 0.6).abs() < 1.0e-6);
        assert!(haze.mist > 0.0 && haze.mist < 0.1);
        assert_eq!(
            (haze.phase, haze.back, haze.reach, haze.lo, haze.hi),
            (
                neutral.phase,
                neutral.back,
                neutral.reach,
                neutral.lo,
                neutral.hi
            )
        );
        assert_eq!(haze.gold, [1.0; 3]);
        assert_eq!(haze.ambient, [HAZE_AMBIENT_GREY; 3]);
    }

    #[test]
    fn haze_from_amount_grows_and_clamps_at_one() {
        let low = RoomHaze::from_amount(LO, HI, 0.3);
        let full = RoomHaze::from_amount(LO, HI, 1.0);
        assert!(full.fog > low.fog && full.smoke > low.smoke);
        assert!(full.mist > low.mist && full.floor > low.floor);
        assert_eq!(RoomHaze::from_amount(LO, HI, 7.0), full);
    }

    #[test]
    fn plume_depth_follows_box_scale() {
        let small = plume_optical_depth(1.8, 0.1, 0.1, 9.0);
        let large = plume_optical_depth(1.8, 1.0, 1.0, 9.0);
        assert!((small - large).abs() < 1.0e-5);
        assert!(1.0 - (-small).exp() > 0.9);
    }

    #[test]
    fn room_fog_follows_beer_lambert() {
        let through = (0..64).fold(1.0_f32, |through, _| {
            through * (-0.05_f32 * 2.0 / 64.0).exp()
        });
        assert!((fog_transmittance(0.05, 2.0, 3.0) - through).abs() < 1e-6);
        assert!((fog_transmittance(0.05, 9.0, 3.0) - (-0.15_f32).exp()).abs() < 1e-6);
        assert_eq!(fog_transmittance(0.0, 3.0, 3.0), 1.0);
        assert_eq!(fog_transmittance(0.05, 0.0, 3.0), 1.0);
    }

    #[test]
    fn room_in_scatter_integrates_attenuated_light() {
        let steps = 4096;
        let step = 1.3 / steps as f32;
        let marched: f32 = (0..steps)
            .map(|i| 0.07 * (-0.05 * (i as f32 + 0.5) * step).exp() * 2.9 * step)
            .sum();
        assert!((fog_in_scatter(0.07, 0.05, 2.9, 1.3) - marched).abs() < 1e-5);
        assert!((fog_in_scatter(0.07, 0.0, 2.9, 1.3) - 0.07 * 2.9 * 1.3).abs() < 1e-6);
    }

    #[test]
    fn henyey_greenstein_integrates_to_one() {
        let steps = 20_000;
        for g in [-0.55, 0.0, 0.35] {
            let total: f32 = (0..steps)
                .map(|i| {
                    let cosine = -1.0 + 2.0 * (i as f32 + 0.5) / steps as f32;
                    henyey_greenstein(cosine, g) * 2.0 * std::f32::consts::PI * 2.0 / steps as f32
                })
                .sum();
            assert!((total - 1.0).abs() < 1e-3, "g {g}: {total}");
        }
    }

    fn fixture_motes() -> RoomMotes {
        RoomMotes::new(
            [-0.6, 0.0, -0.4],
            [0.7, 0.5, 0.5],
            0.04,
            0.2,
            0.7,
            [1.0, 0.9, 0.7],
        )
    }

    #[test]
    fn motes_repeat_for_seed_and_time() {
        let motes = fixture_motes();
        let first = room_motes(7, 2.5, motes);
        assert!(!first.is_empty());
        assert_eq!(first, room_motes(7, 2.5, motes));
        assert_ne!(first, room_motes(8, 2.5, motes));
        assert_ne!(first, room_motes(7, 3.5, motes));
    }

    #[test]
    fn motes_fill_their_share_of_the_cells() {
        let motes = fixture_motes();
        let cells = (0..3)
            .map(|axis| ((motes.hi[axis] - motes.lo[axis]) / motes.cell).ceil())
            .product::<f32>();
        let share = room_motes(0, 0.0, motes).len() as f32 / cells;
        assert!((share - 0.2).abs() < 0.02, "{share}");
        for mote in room_motes(0, 0.0, motes) {
            assert!((0.00025..=0.00075).contains(&mote.position_size[3]));
        }
    }

    #[test]
    fn plume_footprint_overlaps_its_analytic_shape() {
        let plume = Plume {
            drift: [[-0.8, -0.1], [0.0, 0.0]],
            sway: [[15.0, 1.0, 0.01, 0.4, 5.0], [11.0, 0.8, 0.01, 0.4, 4.0]],
            ..Plume::default()
        };
        let radius = plume.radius;
        let weight = plume.depth_per_width / (radius * 6.0);
        let mut reference = 0;
        let mut overlap = 0;
        let mut displaced = 0;
        for yi in 0..201 {
            let rise = yi as f32 * 0.001;
            let edge = |a: f32, b: f32, x: f32| {
                let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            let fade =
                edge(0.0, plume.fade[0], rise) * (1.0 - edge(plume.fade[1], plume.fade[2], rise));
            let drift = plume.drift[0];
            let sway = plume.sway[0];
            let axis = drift[0] * rise * rise
                + drift[1] * rise
                + (rise * sway[0]).sin() * sway[2] * (sway[3] + rise * sway[4]);
            let width = radius * (plume.spread[0] + rise * plume.spread[1]);
            for xi in 0..161 {
                let x = (xi as f32 - 80.0) * 0.001;
                let core = (-((x - axis) / width).powi(2)).exp();
                let analytic = 1.0 - (-1.8 * core * fade * weight * 0.012).exp();
                let engine = 1.0
                    - (-plume_optical_depth(
                        1.8 * core * fade,
                        0.012,
                        radius * 6.0,
                        plume.depth_per_width,
                    ))
                    .exp();
                let shifted = 1.0
                    - (-plume_optical_depth(
                        1.8 * (-((x - axis - 0.03) / width).powi(2)).exp() * fade,
                        0.012,
                        radius * 6.0,
                        plume.depth_per_width,
                    ))
                    .exp();
                if analytic > 0.05 {
                    reference += 1;
                    overlap += usize::from(engine > 0.05);
                    displaced += usize::from(shifted > 0.05);
                }
            }
        }
        assert!(reference > 400, "{reference}");
        assert!(overlap as f32 / reference as f32 >= 0.8);
        assert!(displaced as f32 / (reference as f32) < 0.8);
    }

    #[test]
    fn particle_and_creature_packing() {
        let specks = [Speck {
            pos: [1.0, 2.0, 3.0],
            vel: [0.0, 1.0, 0.0],
            age: 0.5,
            life: 2.0,
            seed: 42,
        }];
        let petals = pack_particles(&specks, ParticleKind::Petal, 1.0, [1.0; 3]);
        assert_eq!(
            petals,
            pack_particles(&specks, ParticleKind::Petal, 1.0, [1.0; 3])
        );
        assert_eq!(petals[0].position_size[..3], specks[0].pos);
        assert_ne!(
            petals[0].color_rotation[3],
            pack_particles(&specks, ParticleKind::Petal, 2.0, [1.0; 3])[0].color_rotation[3]
        );
        assert_eq!(std::mem::size_of::<ParticleInstance>(), 64);
        let boids = [Boid {
            pos: [2.0, 3.0, 4.0],
            vel: [0.0, 0.0, -2.0],
        }];
        let birds = pack_creatures(&boids, CreatureKind::Bird, 0.3, 4);
        assert_eq!(birds, pack_creatures(&boids, CreatureKind::Bird, 0.3, 4));
        assert_eq!(birds[0].position_scale, [2.0, 3.0, 4.0, 0.3]);
        assert_eq!(birds[0].forward[..3], [0.0, 0.0, -1.0]);
        assert_eq!(std::mem::size_of::<CreatureInstance>(), 80);
    }

    #[test]
    fn shaders_parse() {
        for source in [
            PARTICLE_WGSL,
            CREATURE_WGSL,
            VOLUME_WGSL,
            UPSAMPLE_WGSL,
            ROOM_BLUR_WGSL,
            ROOM_FIELD_WGSL,
        ] {
            naga::front::wgsl::parse_str(source).unwrap();
        }
    }

    #[test]
    fn bilateral_weights_respect_depth() {
        let weights = upsample_weights(2.0, [2.0, 100.0, 2.0, 100.0], [0.5, 0.5], 1.0);
        assert!(weights[0] > 0.49 && weights[2] > 0.49);
        assert!(weights[1] < 1e-6 && weights[3] < 1e-6);
        assert!((weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        let flat = upsample_weights(2.0, [2.0; 4], [0.25, 0.75], 1.0);
        assert!((flat[0] - 0.1875).abs() < 1e-6);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn room_haze_transmittance_matches_analytic_fog() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (width, height) = (256, 128);
        let mut effects = Effects::new(&gpu.device, width, height, 4).unwrap();
        for (depth, ambient) in [(2.0_f32, 0.3_f32), (5.0, 0.3), (1.2, 0.0)] {
            let hdr = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
            let scene_depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("room haze test depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SCENE_DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let depth_view = scene_depth.create_view(&Default::default());
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("room haze test clear"),
                    color_attachments: &[
                        Some(color_attachment(&hdr.view, true)),
                        Some(wgpu::RenderPassColorAttachment {
                            view: &depth_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: f64::from(depth),
                                    g: 0.0,
                                    b: 0.0,
                                    a: 0.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        }),
                    ],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            let mut frame = EffectFrame::new(width, height, 1.0, 17);
            frame.lens = [0.001, 0.001, 0.0, 0.0];
            frame.sun_color = [1.0, 1.0, 1.0, 0.0];
            let haze = RoomHaze {
                fog: 0.05,
                smoke: 0.0,
                mist: 0.0,
                floor: 0.0,
                phase: 0.35,
                back: 0.7,
                gold: [1.0; 3],
                ambient: [ambient; 3],
                reach: 3.0,
                unmapped: 1.0,
                lo: [-1.0, -1.0, -1.0],
                hi: [1.0, 1.0, 1.0],
                seed: 0,
            };
            frame.set_room_haze(haze);
            effects.prepare_room(
                &mut EffectCommands {
                    device: &gpu.device,
                    encoder: &mut encoder,
                },
                &haze,
            );
            frame.eye = [0.0, 1.0, 3.0, 0.0];
            effects.render_volume(
                &mut EffectCommands {
                    device: &gpu.device,
                    encoder: &mut encoder,
                },
                &frame,
                EffectTargets {
                    hdr: &hdr.view,
                    depth: &depth_view,
                    shadow: None,
                },
                VolumeTimestamps {
                    march: None,
                    upsample: None,
                },
            );
            gpu.queue.submit(Some(encoder.finish()));
            let pixels = gpu.readback_rgba16(&hdr).unwrap();
            let at = ((height / 2 * width + width / 2) * 4) as usize;
            let value = |channel: usize| half::f16::from_bits(pixels[at + channel]).to_f32();
            let span = depth.min(3.0);
            let alpha = 1.0 - fog_transmittance(0.05, span, 3.0);
            assert!(
                (value(3) - alpha).abs() < 2e-3,
                "depth {depth}: alpha {} against {alpha}",
                value(3)
            );
            let flat = fog_in_scatter(0.05, 0.05, ambient, span);
            assert!(
                value(0) >= 0.5 * flat - 1e-3 && value(0) <= 1.5 * flat + 1e-3,
                "depth {depth}: {} against {flat}",
                value(0)
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn four_k_effects() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let width = 3840;
        let height = 2160;
        let hdr = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("effects test depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let mut clear = gpu.device.create_command_encoder(&Default::default());
        {
            let _pass = clear.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effects test clear"),
                color_attachments: &[
                    Some(color_attachment(&hdr.view, true)),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &depth_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 100.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        gpu.queue.submit(Some(clear.finish()));
        let mut effects = Effects::new(&gpu.device, width, height, 4).unwrap();
        let mut frame = EffectFrame::new(width, height, 1.0, 17);
        frame.volume_lo = [-0.5, -0.05, -0.5, 0.0];
        frame.volume_hi = [0.5, 0.15, 0.5, 0.0];
        let a = -100.0 / 99.9;
        let b = -10.0 / 99.9;
        frame.view_proj = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, a, -1.0],
            [0.0, 0.0, -3.0 * a + b, 3.0],
        ];
        let petals: Vec<_> = (0..10_000)
            .map(|index| {
                let x = 0.35 + (index % 100) as f32 * 0.002;
                let y = -0.2 + (index / 100) as f32 * 0.004;
                Speck {
                    pos: [x, y, 0.0],
                    vel: [0.0, -0.2, 0.0],
                    age: 0.5,
                    life: 2.0,
                    seed: index,
                }
            })
            .collect();
        let petals = pack_particles(&petals, ParticleKind::Petal, 1.0, [1.0; 3]);
        let boids: Vec<_> = (0..200)
            .map(|index| Boid {
                pos: [
                    -0.6 + (index % 20) as f32 * 0.01,
                    (index / 20) as f32 * 0.01,
                    0.0,
                ],
                vel: [0.0, 0.0, -1.0],
            })
            .collect();
        let birds = pack_creatures(&boids, CreatureKind::Bird, 0.1, 9);
        let mesh = CreatureMesh::new(
            &gpu.device,
            &[
                CreatureVertex {
                    position: [-1.0, 0.0, 0.0, 1.0],
                },
                CreatureVertex {
                    position: [0.0, 0.0, -0.3, 1.0],
                },
                CreatureVertex {
                    position: [1.0, 0.0, 0.0, 1.0],
                },
                CreatureVertex {
                    position: [0.0, 0.0, 0.3, 1.0],
                },
            ],
            &[0, 1, 2, 0, 2, 3],
        );
        let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let march = profiler.pass("volume march");
        let upsample = profiler.pass("volume upsample");
        let creatures = profiler.pass("creatures");
        let particles = profiler.pass("particles");
        let targets = EffectTargets {
            hdr: &hdr.view,
            depth: &depth_view,
            shadow: None,
        };
        let mut commands = EffectCommands {
            device: &gpu.device,
            encoder: &mut encoder,
        };
        effects.render_volume(
            &mut commands,
            &frame,
            targets,
            VolumeTimestamps {
                march: profiler.render_writes(march),
                upsample: profiler.render_writes(upsample),
            },
        );
        effects.render_creatures(
            &mut commands,
            &frame,
            targets,
            &mesh,
            &birds,
            profiler.render_writes(creatures),
        );
        effects.render_particles(
            &mut commands,
            &frame,
            targets,
            &petals,
            profiler.render_writes(particles),
        );
        let slot = profiler.finish(&mut encoder);
        gpu.queue.submit(Some(encoder.finish()));
        if let Some(slot) = slot {
            profiler.submitted(slot);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            for timing in profiler.collect(&gpu.device).into_iter().flatten() {
                eprintln!("{}: {:.3} ms", timing.label, timing.milliseconds);
            }
        }
        let pixels = gpu.readback_rgba16(&hdr).unwrap();
        let lit = |x: usize, y: usize| -> bool {
            let index = (y * width as usize + x) * 4;
            pixels[index] != 0 || pixels[index + 1] != 0 || pixels[index + 2] != 0
        };
        assert!(lit(1920, 1080));
        assert!(lit(2200, 1080));
        assert!(lit(1600, 1050));
        assert!(!lit(20, 20));
    }

    #[test]
    fn steam_clock_scales_only_time() {
        let base = EffectFrame::new(8, 8, 3.0, 5);
        let mut same = base;
        same.scale_time(1.0);
        assert_eq!(bytemuck::bytes_of(&same), bytemuck::bytes_of(&base));
        let mut half = base;
        half.scale_time(0.5);
        assert_eq!(half.extent, [8.0, 8.0, 1.5, f32::from_bits(5)]);
        let mut frozen = base;
        frozen.scale_time(-2.0);
        assert_eq!(frozen.extent[2], 0.0);
    }

    fn steam_pixels(gpu: &Gpu, effects: &mut Effects, frame: &EffectFrame) -> Vec<u16> {
        let (width, height) = (256, 128);
        let hdr = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("steam clock test depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("steam clock test clear"),
                color_attachments: &[
                    Some(color_attachment(&hdr.view, true)),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &depth_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 100.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        effects.render_volume(
            &mut EffectCommands {
                device: &gpu.device,
                encoder: &mut encoder,
            },
            frame,
            EffectTargets {
                hdr: &hdr.view,
                depth: &depth_view,
                shadow: None,
            },
            VolumeTimestamps {
                march: None,
                upsample: None,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));
        gpu.readback_rgba16(&hdr).unwrap()
    }

    fn test_plume() -> Plume {
        Plume {
            radius: 0.15,
            ..Plume::at([0.0, -0.05, 0.0])
        }
    }

    fn steam_frame(time: f32) -> EffectFrame {
        let mut frame = EffectFrame::new(256, 128, time, 17);
        frame.set_volume(
            VolumeKind::Plume(test_plume()),
            [-0.5, -0.05, -0.5],
            [0.5, 0.15, 0.5],
            [0.82, 0.85, 0.88],
            1.6,
            0.25,
        );
        let a = -100.0 / 99.9;
        let b = -10.0 / 99.9;
        frame.view_proj = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, a, -1.0],
            [0.0, 0.0, -3.0 * a + b, 3.0],
        ];
        frame
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_steam_at_a_given_time_is_the_same_pixels_under_every_cut() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut effects = Effects::new(&gpu.device, 256, 128, 4).unwrap();
        let cuts = [
            VolumeCuts::FULL,
            VolumeCuts::default(),
            VolumeCuts {
                steps: 6,
                warp_octaves: 1,
                octaves: 1,
                early_out: true,
            },
        ];
        for cut in cuts {
            effects.set_volume_cuts(&gpu.device, cut);
            let first = steam_pixels(&gpu, &mut effects, &steam_frame(2.0));
            let again = steam_pixels(&gpu, &mut effects, &steam_frame(2.0));
            let later = steam_pixels(&gpu, &mut effects, &steam_frame(2.5));
            assert!(first.iter().any(|v| *v != 0), "{cut:?} draws nothing");
            assert_eq!(first, again, "{cut:?}");
            assert_ne!(first, later, "{cut:?}");
        }
        effects.set_volume_cuts(&gpu.device, VolumeCuts::FULL);
        let reference = steam_pixels(&gpu, &mut effects, &steam_frame(2.0));
        let mut fresh = Effects::new(&gpu.device, 256, 128, 4).unwrap();
        assert_eq!(
            reference,
            steam_pixels(&gpu, &mut fresh, &steam_frame(2.0)).as_slice()
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_early_out_leaves_the_steam_byte_identical() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut effects = Effects::new(&gpu.device, 256, 128, 4).unwrap();
        effects.set_volume_cuts(&gpu.device, VolumeCuts::FULL);
        let today = steam_pixels(&gpu, &mut effects, &steam_frame(2.0));
        effects.set_volume_cuts(
            &gpu.device,
            VolumeCuts {
                early_out: true,
                ..VolumeCuts::FULL
            },
        );
        assert_eq!(today, steam_pixels(&gpu, &mut effects, &steam_frame(2.0)));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn steam_at_half_scale_equals_steam_at_half_the_time() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (width, height) = (256, 128);
        let mut effects = Effects::new(&gpu.device, width, height, 4).unwrap();
        let frame_at = |time: f32, scale: f32| {
            let mut frame = EffectFrame::new(width, height, time, 17);
            frame.set_volume(
                VolumeKind::Plume(test_plume()),
                [-0.5, -0.05, -0.5],
                [0.5, 0.15, 0.5],
                [0.82, 0.85, 0.88],
                1.6,
                0.25,
            );
            let a = -100.0 / 99.9;
            let b = -10.0 / 99.9;
            frame.view_proj = [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, a, -1.0],
                [0.0, 0.0, -3.0 * a + b, 3.0],
            ];
            frame.scale_time(scale);
            frame
        };
        let halved = steam_pixels(&gpu, &mut effects, &frame_at(2.0, 0.5));
        let earlier = steam_pixels(&gpu, &mut effects, &frame_at(1.0, 1.0));
        let later = steam_pixels(&gpu, &mut effects, &frame_at(2.0, 1.0));
        assert!(halved.iter().any(|v| *v != 0), "the steam draws nothing");
        assert_eq!(halved, earlier);
        assert_ne!(halved, later);
    }
}
