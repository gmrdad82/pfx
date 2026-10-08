use crate::math::{add, basis, length, normalize, scale, sub};
use crate::mesh::Mesh;
use std::f32::consts::TAU;

pub const TREE_SWAY_WGSL: &str = r#"
fn tree_sway(rest: vec3f, pivot: vec3f, level: f32, stiffness: f32, wind_time: f32, strength: f32, direction: vec3f) -> vec3f {
    let d = normalize(direction.xz + vec2f(0.00001, 0.0));
    let arm = length(rest - pivot);
    let phase = wind_time * (0.7 + level * 0.41) + pivot.x * 0.83 + pivot.z * 0.67;
    let bend = sin(phase) * strength * (1.0 - stiffness) * arm * (0.012 + 0.009 * level);
    return rest + vec3f(d.x * bend, -abs(bend) * 0.12, d.y * bend);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeSpec {
    pub height: f32,
    pub trunk_radius: f32,
    pub branching: u32,
    pub crown_width: f32,
    pub crown_height: f32,
    pub blossom_density: f32,
    pub bloom: f32,
    pub leaf_density: f32,
}

impl TreeSpec {
    pub fn cherry() -> Self {
        Self {
            height: 12.0,
            trunk_radius: 0.35,
            branching: 7,
            crown_width: 4.5,
            crown_height: 4.2,
            blossom_density: 1.0,
            bloom: 1.0,
            leaf_density: 0.45,
        }
    }
    pub fn plum() -> Self {
        Self {
            height: 8.0,
            trunk_radius: 0.27,
            branching: 6,
            crown_width: 3.2,
            crown_height: 3.3,
            blossom_density: 0.85,
            bloom: 1.0,
            leaf_density: 0.55,
        }
    }
    pub fn magnolia() -> Self {
        Self {
            height: 9.0,
            trunk_radius: 0.32,
            branching: 5,
            crown_width: 3.8,
            crown_height: 4.0,
            blossom_density: 0.65,
            bloom: 1.0,
            leaf_density: 0.7,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwayVertex {
    pub pivot: [f32; 3],
    pub level: f32,
    pub stiffness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
    pub position: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub size: [f32; 2],
    pub color: [f32; 4],
    pub atlas: [f32; 4],
    pub sway: SwayVertex,
    pub blossom: bool,
    pub lod: CardLod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardLod {
    Always,
    Individual,
    Cluster,
}

#[derive(Clone, Debug)]
pub struct AlphaAtlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PetalSource {
    pub position: [f32; 3],
    pub rate: f32,
    pub seed: u64,
    pub sway: SwayVertex,
}

#[derive(Clone, Debug)]
pub struct Tree {
    pub seed: u64,
    pub spec: TreeSpec,
    pub wood: Mesh,
    pub sway: Vec<SwayVertex>,
    pub cards: Vec<Card>,
    pub atlas: AlphaAtlas,
    pub petal_sources: Vec<PetalSource>,
    pub flower_count: usize,
    pub max_error: f32,
}

pub fn tree_sway(
    rest: [f32; 3],
    pivot: [f32; 3],
    level: f32,
    stiffness: f32,
    wind_time: f32,
    strength: f32,
    direction: [f32; 3],
) -> [f32; 3] {
    let d = normalize([direction[0] + 0.00001, 0.0, direction[2]]);
    let arm = length(sub(rest, pivot));
    let phase = wind_time * (0.7 + level * 0.41) + pivot[0] * 0.83 + pivot[2] * 0.67;
    let bend = phase.sin() * strength * (1.0 - stiffness) * arm * (0.012 + 0.009 * level);
    add(rest, [d[0] * bend, -bend.abs() * 0.12, d[2] * bend])
}

struct Random(u64);
impl Random {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0.wrapping_mul(0x2545f4914f6cdd1d) >> 40) as u32) as f32 / 16_777_216.0
    }
    fn between(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

struct Tube<'a> {
    positions: &'a mut Vec<[f32; 3]>,
    normals: &'a mut Vec<[f32; 3]>,
    tangents: &'a mut Vec<[f32; 4]>,
    uvs: &'a mut Vec<[f32; 2]>,
    indices: &'a mut Vec<u32>,
    sway: &'a mut Vec<SwayVertex>,
    error: f32,
}

struct TubeShape {
    path: [[f32; 3]; 3],
    radius: f32,
    tip: f32,
    pivot: [f32; 3],
    level: f32,
    stiffness: f32,
    uv_start: f32,
    collar: bool,
}

impl Tube<'_> {
    fn add(&mut self, shape: TubeShape) {
        let TubeShape {
            path,
            radius,
            tip,
            pivot,
            level,
            stiffness,
            uv_start,
            collar,
        } = shape;
        let [start, control, end] = path;
        let n = segments(radius, self.error);
        let path_length = length(sub(control, start)) + length(sub(end, control));
        let bend = length(add(sub(start, scale(control, 2.0)), end));
        let curvature_rings = (bend / (4.0 * self.error)).sqrt().ceil() as usize;
        let rings = ((path_length / (self.error * 12.0).max(0.12)).ceil() as usize)
            .max(curvature_rings)
            .clamp(4, 256);
        let base = self.positions.len() as u32;
        let mut previous_side = [0.0; 3];
        for j in 0..=rings {
            let t = j as f32 / rings as f32;
            let smooth = t * t * (3.0 - 2.0 * t);
            let center = add(
                add(
                    scale(start, (1.0 - t) * (1.0 - t)),
                    scale(control, 2.0 * t * (1.0 - t)),
                ),
                scale(end, t * t),
            );
            let axis = normalize(add(
                scale(sub(control, start), 1.0 - t),
                scale(sub(end, control), t),
            ));
            let side = if j == 0 {
                basis(axis).0
            } else {
                normalize(sub(
                    previous_side,
                    scale(axis, crate::math::dot(previous_side, axis)),
                ))
            };
            previous_side = side;
            let other = crate::math::cross(axis, side);
            let flare = if collar {
                radius * 0.12 * (1.0 - t * 7.0).max(0.0).powi(2)
            } else {
                0.0
            };
            let r = radius + (tip - radius) * smooth + flare;
            for i in 0..=n {
                let a = TAU * i as f32 / n as f32;
                let normal = add(scale(side, a.cos()), scale(other, a.sin()));
                let tangent = add(scale(side, -a.sin()), scale(other, a.cos()));
                self.positions.push(add(center, scale(normal, r)));
                self.normals.push(normal);
                self.tangents
                    .push([tangent[0], tangent[1], tangent[2], 1.0]);
                self.uvs
                    .push([i as f32 / n as f32, uv_start + path_length * t]);
                self.sway.push(SwayVertex {
                    pivot,
                    level,
                    stiffness,
                });
            }
        }
        for j in 0..rings {
            for i in 0..n {
                let a = base + (j * (n + 1) + i) as u32;
                let b = a + (n + 1) as u32;
                self.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
    }
}

pub fn segments(radius: f32, error: f32) -> usize {
    let r = radius.max(1e-5);
    let e = error.clamp(1e-5, r);
    ((TAU / (2.0 * (1.0 - e / r).acos())).ceil() as usize).clamp(12, 256)
}

#[derive(Clone, Copy)]
enum Species {
    Cherry,
    Plum,
    Magnolia,
}

impl Species {
    fn from_spec(spec: TreeSpec) -> Self {
        match spec.branching {
            0..=5 => Self::Magnolia,
            6 => Self::Plum,
            _ => Self::Cherry,
        }
    }

    fn children(self, level: u8) -> usize {
        match (self, level) {
            (Self::Cherry, 1 | 2) => 4,
            (Self::Plum, 1 | 2) => 4,
            (Self::Magnolia, 2) => 4,
            (Self::Magnolia, 3) => 2,
            (Self::Magnolia, _) => 3,
            (Self::Cherry, 4) => 2,
            _ => 3,
        }
    }
}

struct Limb {
    path: [[f32; 3]; 3],
    parent: Option<usize>,
    children: Vec<usize>,
    level: u8,
    radius: f32,
    tip: f32,
    uv: f32,
}

fn limb(
    limbs: &mut Vec<Limb>,
    parent: Option<usize>,
    start: [f32; 3],
    end: [f32; 3],
    lift: f32,
    level: u8,
) -> usize {
    let control = add(scale(start, 0.53), scale(end, 0.47));
    let control = add(control, [0.0, lift, 0.0]);
    let uv = parent.map_or(0.0, |index| {
        let branch: &Limb = &limbs[index];
        branch.uv
            + length(sub(branch.path[1], branch.path[0]))
            + length(sub(branch.path[2], branch.path[1]))
    });
    let index = limbs.len();
    limbs.push(Limb {
        path: [start, control, end],
        parent,
        children: Vec::new(),
        level,
        radius: 0.0,
        tip: 0.0,
        uv,
    });
    if let Some(parent) = parent {
        limbs[parent].children.push(index);
    }
    index
}

fn grow(
    limbs: &mut Vec<Limb>,
    parent: usize,
    heading: f32,
    species: Species,
    spec: TreeSpec,
    rng: &mut Random,
) {
    let level = limbs[parent].level;
    if level == 4 && !matches!(species, Species::Cherry) || level == 5 {
        return;
    }
    let count = species.children(level);
    let start = limbs[parent].path[2];
    for child in 0..count {
        let fan = child as f32 - (count - 1) as f32 * 0.5;
        let spread = if level == 1 { 0.84 } else { 1.06 };
        let angle = heading + fan * spread + rng.between(-0.48, 0.48);
        let horizontal = match level {
            1 => spec.crown_width * 0.33,
            2 => spec.crown_width * 0.22,
            3 => spec.crown_width * 0.095,
            _ => spec.crown_width * 0.055,
        } * rng.between(0.78, 1.17)
            * if matches!(species, Species::Magnolia) {
                0.75
            } else {
                1.0
            };
        let base_rise = match level {
            1 => spec.crown_height * 0.23,
            2 => spec.crown_height * 0.13,
            3 => spec.crown_height * 0.045,
            _ => spec.crown_height * 0.035,
        };
        let rise = match species {
            Species::Cherry => base_rise * rng.between(-0.65, 1.75),
            Species::Plum => base_rise * rng.between(-0.95, 1.85),
            Species::Magnolia => base_rise * rng.between(0.35, 2.1),
        };
        let end = add(
            start,
            [angle.cos() * horizontal, rise, angle.sin() * horizontal],
        );
        let index = limb(limbs, Some(parent), start, end, rise * 0.11, level + 1);
        grow(limbs, index, angle, species, spec, rng);
    }
}

fn split_scaffold(
    limbs: &mut Vec<Limb>,
    parent: usize,
    heading: f32,
    species: Species,
    spec: TreeSpec,
    rng: &mut Random,
) {
    let [start, control, end] = limbs[parent].path;
    let t = rng.between(0.40, 0.63);
    let left_control = add(scale(start, 1.0 - t), scale(control, t));
    let right_control = add(scale(control, 1.0 - t), scale(end, t));
    let middle = add(scale(left_control, 1.0 - t), scale(right_control, t));
    limbs[parent].path = [start, left_control, middle];
    let continuation = limb(limbs, Some(parent), middle, end, 0.0, 1);
    limbs[continuation].path[1] = right_control;
    let turn = if rng.next() < 0.5 { -1.0 } else { 1.0 };
    let angle = heading + turn * rng.between(0.67, 1.20);
    let reach = spec.crown_width * rng.between(0.20, 0.32);
    let rise = spec.crown_height
        * match species {
            Species::Magnolia => rng.between(0.17, 0.33),
            _ => rng.between(-0.06, 0.22),
        };
    let side_end = add(middle, [angle.cos() * reach, rise, angle.sin() * reach]);
    let side = limb(limbs, Some(parent), middle, side_end, rise * 0.12, 2);
    grow(limbs, continuation, heading, species, spec, rng);
    grow(limbs, side, angle, species, spec, rng);
}

#[derive(Clone, Copy)]
struct BloomContext {
    species: Species,
    spec: TreeSpec,
    seed: u64,
}

pub const TWIG_TIP_RADIUS: f32 = 0.001;

fn atlas_rect(index: f32) -> [f32; 4] {
    [index / 6.0, 0.0, 1.0 / 6.0, 1.0]
}

fn bloom_cluster(
    cards: &mut Vec<Card>,
    petal_sources: &mut Vec<PetalSource>,
    branch: &Limb,
    context: BloomContext,
    index: usize,
    center: [f32; 3],
    rng: &mut Random,
) -> usize {
    let BloomContext {
        species,
        spec,
        seed,
    } = context;
    let count = if matches!(species, Species::Magnolia) {
        1
    } else {
        7 + (rng.next() * 4.0) as usize
    };
    let sway = SwayVertex {
        pivot: branch.path[0],
        level: 4.0,
        stiffness: 0.12,
    };
    let diameter = match species {
        Species::Cherry => rng.between(0.025, 0.040),
        Species::Plum => rng.between(0.020, 0.025),
        Species::Magnolia => rng.between(0.10, 0.20),
    };
    let size = diameter * 0.5;
    let count = ((count as f32 * spec.bloom).round() as usize)
        .min(count)
        .min((count as f32 * spec.blossom_density).round() as usize);
    let facing = normalize([
        rng.between(-1.0, 1.0),
        rng.between(0.25, 1.0),
        rng.between(-1.0, 1.0),
    ]);
    if count > 0 {
        let (right, up) = basis(facing);
        cards.push(Card {
            position: center,
            right,
            up,
            size: match species {
                Species::Cherry => [0.105, 0.105],
                Species::Plum => [0.038, 0.038],
                Species::Magnolia => [0.13, 0.13],
            }
            .map(|size| size * spec.bloom.sqrt()),
            color: match species {
                Species::Cherry => [2.1, 1.72, 1.89, 1.0],
                Species::Plum => [1.9, 1.24, 1.45, 1.0],
                Species::Magnolia => [2.0, 1.65, 1.83, 1.0],
            },
            atlas: atlas_rect(4.0),
            sway,
            blossom: true,
            lod: CardLod::Cluster,
        });
    }
    for flower in 0..count {
        let angle = TAU * (flower as f32 / count as f32 + rng.between(-0.09, 0.09));
        let distance = match species {
            Species::Cherry => rng.between(0.035, 0.095),
            Species::Plum => rng.between(0.002, 0.006),
            Species::Magnolia => rng.between(0.015, 0.045),
        };
        let position = add(
            center,
            [
                angle.cos() * distance,
                rng.between(-0.25, 0.45) * distance,
                angle.sin() * distance,
            ],
        );
        let facing = normalize([
            angle.cos() + rng.between(-0.45, 0.45),
            rng.between(0.2, 1.1),
            angle.sin() + rng.between(-0.45, 0.45),
        ]);
        let (right, up) = basis(facing);
        let shade = rng.between(0.0, 1.0);
        let color = match species {
            Species::Cherry => [1.0, 0.69 + 0.28 * shade, 0.79 + 0.18 * shade, 1.0],
            Species::Plum => [
                0.85 + 0.15 * shade,
                0.36 + 0.27 * shade,
                0.53 + 0.30 * shade,
                1.0,
            ],
            Species::Magnolia => [
                0.91 + 0.09 * shade,
                0.71 + 0.26 * shade,
                0.79 + 0.20 * shade,
                1.0,
            ],
        };
        cards.push(Card {
            position,
            right,
            up,
            size: [size, size],
            color: [color[0] * 2.0, color[1] * 2.0, color[2] * 2.0, 1.0],
            atlas: atlas_rect(match species {
                Species::Cherry => 0.0,
                Species::Plum => 1.0,
                Species::Magnolia => 2.0,
            }),
            sway,
            blossom: true,
            lod: CardLod::Individual,
        });
    }
    if rng.next() < spec.leaf_density * 0.54 {
        let normal = normalize([rng.between(-1.0, 1.0), 0.6, rng.between(-1.0, 1.0)]);
        let (right, up) = basis(normal);
        cards.push(Card {
            position: add(center, [0.0, -0.018, 0.0]),
            right,
            up,
            size: [0.028, 0.04],
            color: [0.38, 0.57, 0.27, 1.0],
            atlas: atlas_rect(3.0),
            sway,
            blossom: false,
            lod: CardLod::Always,
        });
    }
    petal_sources.push(PetalSource {
        position: center,
        rate: spec.blossom_density * spec.bloom * 0.18,
        seed: seed ^ (index as u64).wrapping_mul(0x9e3779b97f4a7c15),
        sway,
    });
    count
}

fn topology(spec: TreeSpec, species: Species, rng: &mut Random) -> Vec<Limb> {
    let mut limbs = Vec::new();
    let root_end = [
        0.0,
        spec.height
            * if matches!(species, Species::Magnolia) {
                0.16
            } else {
                0.30
            },
        0.0,
    ];
    let mut spine = limb(&mut limbs, None, [0.0; 3], root_end, 0.0, 0);
    for branch in 0..spec.branching {
        let start = limbs[spine].path[2];
        let angle = TAU * (branch as f32 / spec.branching as f32 + rng.between(-0.055, 0.055));
        let reach = if matches!(species, Species::Magnolia) {
            spec.crown_width * (0.54 - branch as f32 * 0.055) * rng.between(0.85, 1.16)
        } else {
            spec.crown_width * rng.between(0.28, 0.58)
        };
        let rise = if matches!(species, Species::Magnolia) {
            spec.crown_height * (0.18 + branch as f32 * 0.055) * rng.between(0.82, 1.16)
        } else {
            spec.crown_height * rng.between(0.10, 0.49)
        };
        let end = add(start, [angle.cos() * reach, rise, angle.sin() * reach]);
        let scaffold = limb(&mut limbs, Some(spine), start, end, rise * 0.16, 1);
        split_scaffold(&mut limbs, scaffold, angle, species, spec, rng);
        let next = add(
            start,
            [
                rng.between(-0.05, 0.05),
                spec.height
                    * if matches!(species, Species::Magnolia) {
                        0.12
                    } else {
                        0.085
                    },
                rng.between(-0.05, 0.05),
            ],
        );
        spine = limb(&mut limbs, Some(spine), start, next, 0.0, 0);
    }
    let apex_heading = rng.between(0.0, TAU);
    grow(&mut limbs, spine, apex_heading, species, spec, rng);
    for index in (0..limbs.len()).rev() {
        let flow = limbs[index]
            .children
            .iter()
            .map(|child| limbs[*child].radius * limbs[*child].radius)
            .sum::<f32>()
            .sqrt();
        limbs[index].tip = flow;
        limbs[index].radius = (flow * flow
            + if limbs[index].children.is_empty() {
                1.0
            } else {
                0.16
            })
        .sqrt();
    }
    limbs
}

impl Tree {
    pub fn new(seed: u64, spec: TreeSpec, screen_error_world: f32) -> Self {
        let height = spec.height.max(0.5);
        let blossom_density = if spec.blossom_density.is_finite() {
            spec.blossom_density.clamp(0.0, 4.0)
        } else {
            0.0
        };
        let leaf_density = if spec.leaf_density.is_finite() {
            spec.leaf_density.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let bloom = if spec.bloom.is_finite() {
            spec.bloom.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let spec = TreeSpec {
            height,
            trunk_radius: spec.trunk_radius.max(0.01).min(height * 0.1),
            branching: spec.branching.clamp(1, 32),
            crown_width: spec.crown_width.max(0.1),
            crown_height: spec.crown_height.max(0.1),
            blossom_density,
            bloom,
            leaf_density,
        };
        let error = if screen_error_world.is_finite() {
            screen_error_world.clamp(0.001, 0.1)
        } else {
            0.01
        };
        let mut rng = Random(if seed == 0 { 0x9e3779b97f4a7c15 } else { seed });
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut tangents = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        let mut sway = Vec::new();
        let mut cards = Vec::new();
        let mut petal_sources = Vec::new();
        let mut flower_count = 0;
        let species = Species::from_spec(spec);
        let limbs = topology(spec, species, &mut rng);
        let multiplier = spec.trunk_radius / limbs[0].radius;
        let mut tube = Tube {
            positions: &mut positions,
            normals: &mut normals,
            tangents: &mut tangents,
            uvs: &mut uvs,
            indices: &mut indices,
            sway: &mut sway,
            error,
        };
        for (index, branch) in limbs.iter().enumerate() {
            let radius = if branch.level == 5 {
                (branch.radius * multiplier).min(0.005)
            } else {
                branch.radius * multiplier
            };
            let tip = if branch.children.is_empty() {
                TWIG_TIP_RADIUS
            } else {
                branch.tip * multiplier
            };
            let hidden_twig =
                matches!(species, Species::Cherry) && spec.bloom >= 0.85 && branch.level >= 3;
            if !hidden_twig && (branch.level < 3 || radius * 2.0 >= error) {
                let pivot = branch
                    .parent
                    .map_or([0.0; 3], |parent| limbs[parent].path[0]);
                let stiffness = match branch.level {
                    0 => 0.96,
                    1 => 0.69,
                    2 => 0.41,
                    _ => 0.18,
                };
                tube.add(TubeShape {
                    path: branch.path,
                    radius,
                    tip,
                    pivot,
                    level: branch.level as f32,
                    stiffness,
                    uv_start: branch.uv,
                    collar: branch.level > 0,
                });
            }
            if branch.level >= 3 || matches!(species, Species::Cherry) && branch.level == 2 {
                let spurs = match species {
                    Species::Cherry if branch.level == 2 => 12,
                    Species::Cherry if branch.level == 5 => 6,
                    Species::Cherry => 18,
                    Species::Plum => 13,
                    Species::Magnolia => 3,
                };
                for spur in 0..spurs {
                    if spur + 1 != spurs && rng.next() > spec.blossom_density.min(1.0) {
                        continue;
                    }
                    let t = (spur as f32 + 1.0) / spurs as f32;
                    let center = add(
                        add(
                            scale(branch.path[0], (1.0 - t).powi(2)),
                            scale(branch.path[1], 2.0 * t * (1.0 - t)),
                        ),
                        scale(branch.path[2], t * t),
                    );
                    let mut spur_rng = Random(seed.wrapping_add(
                        (index as u64 * 32 + spur as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15),
                    ));
                    let center = if matches!(species, Species::Cherry) {
                        let reach = if branch.level == 2 { 0.19 } else { 0.27 };
                        add(
                            center,
                            [
                                spur_rng.between(-reach, reach),
                                spur_rng.between(-reach * 0.55, reach * 0.85),
                                spur_rng.between(-reach, reach),
                            ],
                        )
                    } else {
                        center
                    };
                    flower_count += bloom_cluster(
                        &mut cards,
                        &mut petal_sources,
                        branch,
                        BloomContext {
                            species,
                            spec,
                            seed,
                        },
                        index * spurs + spur,
                        center,
                        &mut spur_rng,
                    );
                }
                if branch.children.is_empty() && spec.blossom_density > 0.0 && spec.bloom > 0.0 {
                    let facing = normalize(sub(branch.path[2], branch.path[1]));
                    let (right, up) = basis(facing);
                    let size = match species {
                        Species::Cherry => 0.018,
                        Species::Plum => 0.011,
                        Species::Magnolia => 0.075,
                    };
                    cards.push(Card {
                        position: branch.path[2],
                        right,
                        up,
                        size: [size, size],
                        color: [2.0, 1.65, 1.8, 1.0],
                        atlas: atlas_rect(match species {
                            Species::Cherry => 0.0,
                            Species::Plum => 1.0,
                            Species::Magnolia => 2.0,
                        }),
                        sway: SwayVertex {
                            pivot: branch.path[0],
                            level: branch.level as f32,
                            stiffness: 0.12,
                        },
                        blossom: true,
                        lod: CardLod::Always,
                    });
                    flower_count += 1;
                }
                if branch.children.is_empty() && (spec.blossom_density == 0.0 || spec.bloom == 0.0)
                {
                    let (right, up) = basis(normalize(sub(branch.path[2], branch.path[1])));
                    cards.push(Card {
                        position: branch.path[2],
                        right,
                        up,
                        size: [0.006, 0.012],
                        color: [0.72, 0.3, 0.38, 1.0],
                        atlas: atlas_rect(5.0),
                        sway: SwayVertex {
                            pivot: branch.path[0],
                            level: 4.0,
                            stiffness: 0.12,
                        },
                        blossom: false,
                        lod: CardLod::Always,
                    });
                }
            }
        }
        let wood = Mesh::new(positions, normals, tangents, uvs, indices);
        Self {
            seed,
            spec,
            wood,
            sway,
            cards,
            atlas: atlas(),
            petal_sources,
            flower_count,
            max_error: error,
        }
    }
}

fn flower_sample(u: f32, v: f32, plum: bool) -> [f32; 3] {
    let mut edge = f32::INFINITY;
    for petal in 0..5 {
        let angle = TAU * petal as f32 / 5.0 - std::f32::consts::FRAC_PI_2;
        let outward = u * angle.cos() + v * angle.sin();
        let across = -u * angle.sin() + v * angle.cos();
        let width = if plum { 0.34 } else { 0.39 };
        let shape = ((outward - 0.45) / 0.51).powi(2) + (across / width).powi(2);
        let notch = outward > 0.79 && across.abs() < (outward - 0.79) * 0.48;
        if !notch {
            edge = edge.min(shape);
        }
    }
    let alpha = ((1.0 - edge) * 13.0).clamp(0.0, 1.0);
    let radius = (u * u + v * v).sqrt();
    let center = ((0.22 - radius) * 19.0).clamp(0.0, 1.0) * alpha;
    let mut stamens: f32 = 0.0;
    for i in 0..9 {
        let angle = TAU * i as f32 / 9.0;
        let tip = [angle.cos() * 0.23, angle.sin() * 0.23];
        let distance = ((u - tip[0]).powi(2) + (v - tip[1]).powi(2)).sqrt();
        stamens = stamens.max(((0.045 - distance) * 28.0).clamp(0.0, 1.0));
    }
    [alpha, center, stamens * alpha]
}

pub fn atlas() -> AlphaAtlas {
    let width = 384;
    let height = 64;
    let mut pixels = vec![0; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let tile = x / 64;
            let u = ((x % 64) as f32 + 0.5) / 32.0 - 1.0;
            let v = (y as f32 + 0.5) / 32.0 - 1.0;
            let sample = match tile {
                0 => flower_sample(u, v, false),
                1 => flower_sample(u, v, true),
                2 => {
                    let mut coverage: f32 = 0.0;
                    for petal in 0..6 {
                        let angle = TAU * petal as f32 / 6.0 - std::f32::consts::FRAC_PI_2;
                        let outward = u * angle.cos() + v * angle.sin();
                        let across = -u * angle.sin() + v * angle.cos();
                        let ellipse = ((outward - 0.37) / 0.62).powi(2) + (across / 0.29).powi(2);
                        coverage = coverage.max(((1.0 - ellipse) * 11.0).clamp(0.0, 1.0));
                    }
                    let center = ((0.27 - (u * u + v * v).sqrt()) * 12.0).clamp(0.0, 1.0);
                    [coverage, center * coverage, center * coverage * 0.55]
                }
                3 => {
                    let shape = (u / 0.52).powi(2) + ((v + 0.04) / 0.92).powi(2);
                    [((1.0 - shape) * 12.0).clamp(0.0, 1.0), 0.0, 0.0]
                }
                4 => {
                    let mut result = [0.0_f32; 3];
                    for (cx, cy) in [(-0.42, -0.25), (0.42, -0.15), (0.0, 0.39), (-0.04, -0.36)] {
                        let flower = flower_sample((u - cx) * 1.9, (v - cy) * 1.9, false);
                        for channel in 0..3 {
                            result[channel] = result[channel].max(flower[channel]);
                        }
                    }
                    result
                }
                _ => {
                    let shape = (u / 0.29).powi(2) + ((v + 0.05) / 0.70).powi(2);
                    [((1.0 - shape) * 14.0).clamp(0.0, 1.0), 0.12, 0.0]
                }
            };
            let offset = (y * width + x) * 4;
            pixels[offset] = (sample[0] * 255.0) as u8;
            pixels[offset + 1] = (sample[1] * 255.0) as u8;
            pixels[offset + 2] = (sample[2] * 255.0) as u8;
            pixels[offset + 3] = pixels[offset];
        }
    }
    AlphaAtlas {
        width: width as u32,
        height: height as u32,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_radii_and_envelope() {
        for spec in [TreeSpec::cherry(), TreeSpec::plum(), TreeSpec::magnolia()] {
            let mut rng = Random(31);
            let limbs = topology(spec, Species::from_spec(spec), &mut rng);
            assert!(limbs.iter().any(|branch| branch.level == 4));
            for branch in &limbs {
                assert!(branch.radius >= branch.tip);
                let children_area = branch
                    .children
                    .iter()
                    .map(|child| limbs[*child].radius.powi(2))
                    .sum::<f32>();
                assert!((branch.tip.powi(2) - children_area).abs() < 1e-4);
                if branch.children.is_empty() {
                    assert_eq!(branch.tip, 0.0);
                }
            }
            let scale_factor = spec.trunk_radius / limbs[0].radius;
            for branch in &limbs {
                let mut positions = Vec::new();
                let mut normals = Vec::new();
                let mut tangents = Vec::new();
                let mut uvs = Vec::new();
                let mut indices = Vec::new();
                let mut sway = Vec::new();
                let mut tube = Tube {
                    positions: &mut positions,
                    normals: &mut normals,
                    tangents: &mut tangents,
                    uvs: &mut uvs,
                    indices: &mut indices,
                    sway: &mut sway,
                    error: 0.01,
                };
                let radius = branch.radius * scale_factor;
                let tip = branch.tip * scale_factor;
                tube.add(TubeShape {
                    path: branch.path,
                    radius,
                    tip,
                    pivot: branch.path[0],
                    level: branch.level as f32,
                    stiffness: 0.5,
                    uv_start: branch.uv,
                    collar: branch.level > 0,
                });
                let ring_size = segments(radius, 0.01) + 1;
                let rings = positions.len() / ring_size - 1;
                for (vertex, point) in positions.iter().enumerate() {
                    let t = (vertex / ring_size) as f32 / rings as f32;
                    let center = add(
                        add(
                            scale(branch.path[0], (1.0 - t).powi(2)),
                            scale(branch.path[1], 2.0 * t * (1.0 - t)),
                        ),
                        scale(branch.path[2], t * t),
                    );
                    let smooth = t * t * (3.0 - 2.0 * t);
                    let flare = if branch.level > 0 {
                        radius * 0.12 * (1.0 - t * 7.0).max(0.0).powi(2)
                    } else {
                        0.0
                    };
                    let envelope = radius + (tip - radius) * smooth + flare;
                    assert!(length(sub(*point, center)) <= envelope + 1e-5);
                }
            }
        }
    }

    #[test]
    fn flower_sizes_and_spurs() {
        for (spec, low, high, minimum) in [
            (TreeSpec::cherry(), 0.025, 0.040, 10_000),
            (TreeSpec::plum(), 0.020, 0.025, 6_000),
            (TreeSpec::magnolia(), 0.10, 0.20, 500),
        ] {
            let tree = Tree::new(23, spec, 0.001);
            assert!(tree.flower_count >= minimum, "{}", tree.flower_count);
            let flowers: Vec<_> = tree
                .cards
                .iter()
                .filter(|card| card.blossom && card.lod != CardLod::Cluster)
                .collect();
            assert_eq!(flowers.len(), tree.flower_count);
            for flower in flowers {
                let diameter = flower.size[0] * 2.0;
                assert!((low..=high).contains(&diameter), "{diameter}");
            }
        }
        let mut rng = Random(19);
        let spec = TreeSpec::cherry();
        let branch = Limb {
            path: [[0.0; 3]; 3],
            parent: None,
            children: Vec::new(),
            level: 4,
            radius: 1.0,
            tip: 0.0,
            uv: 0.0,
        };
        let mut cards = Vec::new();
        let mut sources = Vec::new();
        for index in 0..100 {
            let start = cards.len();
            let count = bloom_cluster(
                &mut cards,
                &mut sources,
                &branch,
                BloomContext {
                    species: Species::Cherry,
                    spec,
                    seed: 19,
                },
                index,
                [0.0; 3],
                &mut rng,
            );
            assert!((7..=10).contains(&count));
            assert_eq!(
                cards[start..]
                    .iter()
                    .filter(|card| card.lod == CardLod::Individual)
                    .count(),
                count
            );
        }
    }

    #[test]
    fn terminal_nodes_have_buds_or_flowers() {
        for mut spec in [TreeSpec::cherry(), TreeSpec::plum(), TreeSpec::magnolia()] {
            let blooming = Tree::new(23, spec, 0.001);
            spec.blossom_density = 0.0;
            let dormant = Tree::new(23, spec, 0.001);
            let mut rng = Random(23);
            let limbs = topology(spec, Species::from_spec(spec), &mut rng);
            for branch in limbs
                .iter()
                .filter(|branch| branch.children.is_empty() && branch.level >= 3)
            {
                assert!(
                    dormant.cards.iter().any(
                        |card| card.position == branch.path[2] && card.atlas == atlas_rect(5.0)
                    )
                );
                assert!(
                    blooming
                        .cards
                        .iter()
                        .any(|card| card.blossom && card.position == branch.path[2])
                );
            }
        }
        assert_eq!(TWIG_TIP_RADIUS * 2.0, 0.002);
    }

    #[test]
    fn cluster_cards_are_available_at_every_build_detail() {
        let near = Tree::new(23, TreeSpec::cherry(), 0.001);
        let far = Tree::new(23, TreeSpec::cherry(), 0.01);
        assert_eq!(near.flower_count, far.flower_count);
        assert_eq!(near.cards.len(), far.cards.len());
        assert_eq!(near.cards, far.cards);
        assert!(far.cards.iter().any(|card| card.atlas == atlas_rect(4.0)));
    }

    #[test]
    fn bloom_density_is_monotone() {
        let mut spec = TreeSpec::cherry();
        let mut previous = 0;
        for bloom in [0.0, 0.25, 0.5, 0.75, 1.0] {
            spec.bloom = bloom;
            let tree = Tree::new(23, spec, 0.01);
            assert!(tree.flower_count >= previous);
            previous = tree.flower_count;
        }
        assert!(previous > 100_000);
    }
    #[test]
    fn deterministic_and_continuous() {
        let a = Tree::new(17, TreeSpec::cherry(), 0.01);
        let b = Tree::new(17, TreeSpec::cherry(), 0.01);
        assert_eq!(a.wood.bytes(), b.wood.bytes());
        assert_eq!(a.cards, b.cards);
        assert_ne!(a.cards, Tree::new(18, TreeSpec::cherry(), 0.01).cards);
        assert_eq!(a.wood.positions.len(), a.sway.len());
        assert!(!a.petal_sources.is_empty());
    }
    #[test]
    fn tessellation_bound() {
        for radius in [0.01, 0.05, 0.3] {
            for error in [0.001, 0.01] {
                let n = segments(radius, error);
                let actual = radius * (1.0 - (std::f32::consts::PI / n as f32).cos());
                assert!(actual <= error + 1e-6);
            }
        }
        let start = [0.0, 0.0, 0.0];
        let control = [3.0, 5.0, 0.0];
        let end = [6.0, 0.0, 0.0];
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut tangents = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        let mut sway = Vec::new();
        let error = 0.005;
        let mut tube = Tube {
            positions: &mut positions,
            normals: &mut normals,
            tangents: &mut tangents,
            uvs: &mut uvs,
            indices: &mut indices,
            sway: &mut sway,
            error,
        };
        tube.add(TubeShape {
            path: [start, control, end],
            radius: 0.3,
            tip: 0.03,
            pivot: start,
            level: 1.0,
            stiffness: 0.5,
            uv_start: 0.0,
            collar: false,
        });
        let ring_size = segments(0.3, error) + 1;
        let rings = positions.len() / ring_size - 1;
        for j in 0..rings {
            let t0 = j as f32 / rings as f32;
            let t1 = (j + 1) as f32 / rings as f32;
            let t = (t0 + t1) * 0.5;
            let curve = add(
                add(
                    scale(start, (1.0 - t) * (1.0 - t)),
                    scale(control, 2.0 * t * (1.0 - t)),
                ),
                scale(end, t * t),
            );
            let a = add(
                add(
                    scale(start, (1.0 - t0) * (1.0 - t0)),
                    scale(control, 2.0 * t0 * (1.0 - t0)),
                ),
                scale(end, t0 * t0),
            );
            let b = add(
                add(
                    scale(start, (1.0 - t1) * (1.0 - t1)),
                    scale(control, 2.0 * t1 * (1.0 - t1)),
                ),
                scale(end, t1 * t1),
            );
            assert!(length(sub(curve, scale(add(a, b), 0.5))) <= error + 1e-6);
        }
    }
    #[test]
    fn radius_matches_at_tube_joint() {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut tangents = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        let mut sway = Vec::new();
        let mut tube = Tube {
            positions: &mut positions,
            normals: &mut normals,
            tangents: &mut tangents,
            uvs: &mut uvs,
            indices: &mut indices,
            sway: &mut sway,
            error: 0.01,
        };
        let a = [0.0, 0.0, 0.0];
        let b = [0.0, 2.0, 0.0];
        let c = [0.0, 4.0, 0.0];
        tube.add(TubeShape {
            path: [a, [0.0, 1.0, 0.0], b],
            radius: 0.3,
            tip: 0.2,
            pivot: a,
            level: 0.0,
            stiffness: 0.9,
            uv_start: 0.0,
            collar: false,
        });
        let first = tube.positions.len();
        tube.add(TubeShape {
            path: [b, [0.0, 3.0, 0.0], c],
            radius: 0.2,
            tip: 0.04,
            pivot: b,
            level: 1.0,
            stiffness: 0.5,
            uv_start: 0.0,
            collar: false,
        });
        let end_radius = length(sub(positions[first - 1], b));
        let start_radius = length(sub(positions[first], b));
        assert!((end_radius - start_radius).abs() < 1e-6);
    }
    #[test]
    fn sway_stiffness() {
        let p = [1.0, 6.0, 1.0];
        let pivot = [0.0, 5.0, 0.0];
        let trunk = tree_sway(p, pivot, 0.0, 0.95, 1.0, 2.0, [1.0, 0.0, 0.3]);
        let twig = tree_sway(p, pivot, 2.0, 0.18, 1.0, 2.0, [1.0, 0.0, 0.3]);
        assert!(length(sub(trunk, p)) < length(sub(twig, p)));
        let phase: f32 = 1.0 * (0.7 + 2.0 * 0.41);
        let bend = phase.sin() * 2.0 * (1.0 - 0.18) * length(sub(p, pivot)) * (0.012 + 0.009 * 2.0);
        let d = normalize([1.00001, 0.0, 0.3]);
        let reference = add(p, [d[0] * bend, -bend.abs() * 0.12, d[2] * bend]);
        for (actual, expected) in twig.into_iter().zip(reference) {
            assert!((actual - expected).abs() < 1e-6);
        }
        naga::front::wgsl::parse_str(TREE_SWAY_WGSL).unwrap();
    }
}
