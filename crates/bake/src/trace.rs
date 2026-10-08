use pfx_gpu::pace::{Pacer, Slice, Stats, Turns, Work};
use pfx_gpu::{Gpu, wgpu};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::bvh::{Bvh, Hit, Ray, Triangle};
use pfx_trace::shapes::{Shape, intersect};
use pfx_trace::{Camera, Scene, Sun, Trace};
use serde::{Deserialize, Serialize};

use crate::batch::{Batch, FACES_PER_BATCH, FaceRecord};
use crate::grid::{AXES, Grid, GridSpec, Probe, ProbeExtra};

const FACE: usize = 4;
const COSINE_NORMAL: f32 = std::f32::consts::PI / 3.253_611_3;
const INSIDE: u16 = 6;
const WELD: f64 = 1e5;
const ROUNDS: u32 = 4;
const MAX_ROUNDS: u32 = 64;
const SPREAD: f32 = 0.04;
const OUTLIER: f32 = 4.0;
const FLOOR: f32 = 0.1;
const REACH: f32 = 0.8;
const RANGE: f32 = 0.15;
const PASSES: usize = 2;
const V1_PROBES: usize = 64;
const BATCH_LABEL: &str = "bake probe batch";
const SAMPLES_LABEL: &str = "bake probes";

type Lobes = [[f32; 3]; 6];
type Estimates = Vec<(Lobes, [f32; 6])>;
const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
];

#[derive(Clone)]
pub struct Anchor {
    pub hour: f32,
    pub sky: Sky,
    pub sun: Sun,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rounds {
    pub enabled: u32,
    pub active: Vec<u32>,
}

impl Rounds {
    pub fn run(&self) -> u32 {
        self.active.len() as u32
    }

    pub fn converged(&self) -> Vec<u32> {
        let mut before = self.enabled;
        self.active
            .iter()
            .map(|&after| {
                let done = before.saturating_sub(after);
                before = after;
                done
            })
            .collect()
    }
}

pub struct Baked {
    pub grid: Grid,
    pub rounds: Rounds,
}

pub struct Paced<'a> {
    pub pacer: &'a mut Pacer,
    pub between: &'a mut dyn FnMut(f64) -> Result<bool, String>,
}

impl Paced<'_> {
    pub(crate) fn quiet<T>(work: impl FnOnce(&mut Paced<'_>) -> T) -> T {
        let mut pacer = Pacer::default();
        let mut turns = Turns::default();
        let mut between = |ms: f64| Ok(turns.add(ms));
        work(&mut Paced {
            pacer: &mut pacer,
            between: &mut between,
        })
    }

    pub(crate) fn run_gpu(
        &mut self,
        label: &str,
        gpu: &Gpu,
        count: u32,
        mut encode: impl FnMut(&mut wgpu::CommandEncoder, u32, u32),
    ) -> Result<Stats, String> {
        if count == 0 {
            return Ok(Stats::default());
        }
        let mut failed = None::<String>;
        let between = &mut *self.between;
        let stats = self.pacer.run(
            label,
            &gpu.device,
            &gpu.queue,
            Work::Dispatches { count },
            |encoder, slice| {
                if let Slice::Dispatches { start, count } = slice {
                    encode(encoder, start, count);
                }
            },
            |ms| match between(ms) {
                Ok(turned) => turned,
                Err(error) => {
                    failed.get_or_insert(error);
                    false
                }
            },
        )?;
        failed.map_or(Ok(stats), Err)
    }
}

pub struct BakeScene {
    pub triangles: Vec<Triangle>,
    pub shapes: Vec<Shape>,
    pub materials: Vec<Material>,
    pub anchors: Vec<Anchor>,
}

fn direction(face: usize, x: usize, y: usize) -> ([f32; 3], f32) {
    let (forward, right, up) = FACES[face];
    let u = ((x as f32 + 0.5) / FACE as f32 * 2.0) - 1.0;
    let v = 1.0 - (y as f32 + 0.5) / FACE as f32 * 2.0;
    let squared = 1.0 + u * u + v * v;
    let dir =
        [0, 1, 2].map(|axis| (forward[axis] + right[axis] * u + up[axis] * v) / squared.sqrt());
    let solid_angle = (4.0 / (FACE * FACE) as f32) / squared.powf(1.5) * COSINE_NORMAL;
    (dir, solid_angle)
}

struct ProbeMeasure {
    mean: [f32; 6],
    second: [f32; 6],
    backfaces: u16,
    nearest_backface: Option<(f32, [f32; 3])>,
}

fn nearest_hit(scene: &BakeScene, bvh: &Bvh, ray: Ray) -> Option<Hit> {
    let mesh = bvh.intersect(&scene.triangles, ray, 65504.0);
    intersect(&scene.shapes, ray, mesh.map_or(65504.0, |hit| hit.distance)).or(mesh)
}

fn oriented_hit(scene: &BakeScene, bvh: &Bvh, sides: &[i8], ray: Ray) -> Option<(Hit, f32)> {
    let mesh = bvh.intersect(&scene.triangles, ray, 65504.0);
    match intersect(&scene.shapes, ray, mesh.map_or(65504.0, |hit| hit.distance)) {
        Some(hit) => Some((hit, 1.0)),
        None => mesh.map(|hit| (hit, f32::from(sides[hit.index as usize]))),
    }
}

fn root(parent: &mut [u32], mut index: u32) -> u32 {
    while parent[index as usize] != index {
        let next = parent[parent[index as usize] as usize];
        parent[index as usize] = next;
        index = next;
    }
    index
}

fn closed_sides(triangles: &[Triangle]) -> Vec<i8> {
    let key = |v: [f32; 3]| v.map(|value| (f64::from(value) * WELD).round() as i64);
    let mut points: Vec<[i64; 3]> = triangles
        .iter()
        .flat_map(|triangle| triangle.vertices.map(key))
        .collect();
    points.sort_unstable();
    points.dedup();
    let corners: Vec<[u32; 3]> = triangles
        .iter()
        .map(|triangle| {
            triangle
                .vertices
                .map(|v| points.binary_search(&key(v)).unwrap() as u32)
        })
        .collect();
    let mut edges: Vec<(u32, u32, u32)> = corners
        .iter()
        .enumerate()
        .flat_map(|(index, c)| (0..3).map(move |k| (c[k], c[(k + 1) % 3], index as u32)))
        .collect();
    edges.sort_unstable();
    let span = |a: u32, b: u32| {
        let start = edges.partition_point(|edge| (edge.0, edge.1) < (a, b));
        let end = edges.partition_point(|edge| (edge.0, edge.1) <= (a, b));
        &edges[start..end]
    };
    let mut parent: Vec<u32> = (0..triangles.len() as u32).collect();
    let mut closed = vec![true; triangles.len()];
    for (index, c) in corners.iter().enumerate() {
        if c[0] == c[1] || c[1] == c[2] || c[2] == c[0] {
            closed[index] = false;
            continue;
        }
        for k in 0..3 {
            let (a, b) = (c[k], c[(k + 1) % 3]);
            let back = span(b, a);
            if span(a, b).len() != 1 || back.len() != 1 {
                closed[index] = false;
                continue;
            }
            let (x, y) = (
                root(&mut parent, index as u32),
                root(&mut parent, back[0].2),
            );
            if x != y {
                parent[x.max(y) as usize] = x.min(y);
            }
        }
    }
    let mut whole = vec![true; triangles.len()];
    let mut volume = vec![0.0f64; triangles.len()];
    for (index, triangle) in triangles.iter().enumerate() {
        let group = root(&mut parent, index as u32) as usize;
        whole[group] &= closed[index];
        let [a, b, c] = triangle.vertices.map(|v| v.map(f64::from));
        volume[group] += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    (0..triangles.len())
        .map(|index| {
            let group = root(&mut parent, index as u32) as usize;
            if !whole[group] || volume[group].abs() < 1e-15 {
                0
            } else if volume[group] > 0.0 {
                1
            } else {
                -1
            }
        })
        .collect()
}

fn measure_probe(
    scene: &BakeScene,
    bvh: &Bvh,
    sides: &[i8],
    origin: [f32; 3],
    spacing: f32,
) -> ProbeMeasure {
    let mut measure = ProbeMeasure {
        mean: [0.0; 6],
        second: [0.0; 6],
        backfaces: 0,
        nearest_backface: None,
    };
    let limit = spacing * 4.0;
    for face in 0..6 {
        for y in [1, 2] {
            for x in [1, 2] {
                let ray = Ray {
                    origin,
                    direction: direction(face, x, y).0,
                };
                let hit = oriented_hit(scene, bvh, sides, ray);
                let distance = hit.map_or(limit, |(hit, _)| hit.distance.min(limit));
                measure.mean[face] += distance * 0.25;
                measure.second[face] += distance * distance * 0.25;
                if let Some((hit, side)) = hit
                    && side
                        * (0..3)
                            .map(|axis| hit.normal[axis] * ray.direction[axis])
                            .sum::<f32>()
                        > 0.01
                {
                    measure.backfaces += 1;
                    if measure
                        .nearest_backface
                        .is_none_or(|(old, _)| hit.distance < old)
                    {
                        measure.nearest_backface = Some((hit.distance, ray.direction));
                    }
                }
            }
        }
    }
    measure
}

fn probe_geometry(
    scene: &BakeScene,
    bvh: &Bvh,
    sides: &[i8],
    position: [f32; 3],
    spacing: f32,
) -> (ProbeExtra, [f32; 6]) {
    let mut measure = measure_probe(scene, bvh, sides, position, spacing);
    let mut offset = [0.0; 3];
    if measure.backfaces > INSIDE
        && let Some((distance, direction)) = measure.nearest_backface
        && distance < spacing * 0.5
    {
        let reach = (distance + spacing * 0.1).min(spacing * 0.5);
        offset = direction.map(|value| value * reach);
        let relocated = std::array::from_fn(|axis| position[axis] + offset[axis]);
        let next = measure_probe(scene, bvh, sides, relocated, spacing);
        if next.backfaces < measure.backfaces {
            measure = next;
        } else {
            offset = [0.0; 3];
        }
    }
    let extra = ProbeExtra {
        second: measure.second,
        offset,
        enabled: measure.backfaces <= INSIDE,
        backfaces: measure.backfaces,
    };
    (extra, measure.mean)
}

pub fn bake(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
) -> Result<Vec<Grid>, String> {
    (0..scene.anchors.len())
        .map(|index| bake_anchor(gpu, scene, spec, samples, seed, index))
        .collect()
}

pub fn bake_v1(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
) -> Result<Vec<Grid>, String> {
    (0..scene.anchors.len())
        .map(|index| bake_anchor_v1(gpu, scene, spec, samples, seed, index))
        .collect()
}

fn bake_anchor_v1(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    anchor_index: usize,
) -> Result<Grid, String> {
    if samples == 0 {
        return Err("bake needs at least one sample per face texel".into());
    }
    let dims = spec.dimensions()?;
    let count = dims.iter().map(|&v| v as usize).product::<usize>();
    let bvh = Bvh::build(&scene.triangles);
    let batch = Batch::new(gpu, scene, anchor_index)?;
    let mut probes = Vec::with_capacity(count);
    let mut turns = Turns::default();
    for start in (0..count).step_by(V1_PROBES) {
        let end = (start + V1_PROBES).min(count);
        let mut records = Vec::with_capacity((end - start) * FACES.len());
        for index in start..end {
            let position = spec.position(dims, index);
            let mut probe = Probe::default();
            for (lobe, &direction) in AXES.iter().enumerate() {
                let ray = Ray {
                    origin: position,
                    direction,
                };
                probe.visibility[lobe] =
                    nearest_hit(scene, &bvh, ray).map_or(65504.0, |hit| hit.distance);
            }
            probes.push(probe);
            for (face, &basis) in FACES.iter().enumerate() {
                records.push(FaceRecord::new(
                    position,
                    basis,
                    ray_seed(seed, anchor_index, index, face),
                ));
            }
        }
        let image = turns.time(|| batch.sample(gpu, &records, samples))?;
        for (index, probe) in probes.iter_mut().enumerate().take(end).skip(start) {
            for face in 0..FACES.len() {
                let face_index = (index - start) * FACES.len() + face;
                for y in 0..FACE {
                    for x in 0..FACE {
                        let tile_x = face_index % 16;
                        let tile_y = face_index / 16;
                        let offset = ((tile_y * FACE + y) * 64 + tile_x * FACE + x) * 16;
                        accumulate(&mut probe.lobes, &image, offset, face, x, y);
                    }
                }
            }
        }
    }
    Grid::new(spec, probes)
}

pub fn bake_scaled(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    scales: &[f32],
) -> Result<Vec<Grid>, String> {
    if scales.len() != scene.anchors.len() {
        return Err("bake needs one emission scale per anchor".into());
    }
    scales
        .iter()
        .enumerate()
        .map(|(index, &scale)| {
            bake_anchor(
                gpu,
                &with_emission(scene, scale)?,
                spec,
                samples,
                seed,
                index,
            )
        })
        .collect()
}

pub fn with_emission(scene: &BakeScene, scale: f32) -> Result<BakeScene, String> {
    if !scale.is_finite() || scale < 0.0 {
        return Err("emission scale must be finite and nonnegative".into());
    }
    Ok(BakeScene {
        triangles: scene.triangles.clone(),
        shapes: scene.shapes.clone(),
        materials: scene
            .materials
            .iter()
            .map(|material| Material {
                emission: material.emission.map(|value| value * scale),
                ..*material
            })
            .collect(),
        anchors: scene.anchors.clone(),
    })
}

pub fn bake_emitters(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    direct: bool,
) -> Result<Grid, String> {
    Paced::quiet(|paced| bake_emitters_paced(gpu, scene, spec, samples, seed, direct, paced))
}

pub fn bake_emitters_paced(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    direct: bool,
    paced: &mut Paced<'_>,
) -> Result<Grid, String> {
    let hour = scene.anchors.first().map_or(12.0, |anchor| anchor.hour);
    let dark = BakeScene {
        triangles: scene.triangles.clone(),
        shapes: scene.shapes.clone(),
        materials: scene.materials.clone(),
        anchors: vec![Anchor {
            hour,
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0; 4]],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [0.0; 3],
                intensity: 0.0,
            },
        }],
    };
    bake_layer(gpu, &dark, spec, samples, seed, 0, direct, paced).map(|baked| baked.grid)
}

pub fn bake_anchor(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    anchor_index: usize,
) -> Result<Grid, String> {
    bake_anchor_rounds(gpu, scene, spec, samples, seed, anchor_index).map(|baked| baked.grid)
}

pub fn bake_anchor_rounds(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    anchor_index: usize,
) -> Result<Baked, String> {
    Paced::quiet(|paced| bake_anchor_paced(gpu, scene, spec, samples, seed, anchor_index, paced))
}

pub fn bake_anchor_paced(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    anchor_index: usize,
    paced: &mut Paced<'_>,
) -> Result<Baked, String> {
    bake_layer(gpu, scene, spec, samples, seed, anchor_index, true, paced)
}

fn check(scene: &BakeScene, samples: u32) -> Result<(), String> {
    if samples == 0 {
        return Err("bake needs at least one sample per face texel".into());
    }
    if scene.anchors.is_empty()
        || scene
            .anchors
            .iter()
            .any(|a| !a.hour.is_finite() || !(0.0..=24.0).contains(&a.hour))
        || scene.anchors.windows(2).any(|a| a[0].hour >= a[1].hour)
    {
        return Err("bake anchors must be sorted and distinct".into());
    }
    Ok(())
}

fn geometry(
    scene: &BakeScene,
    spec: GridSpec,
    mut between_probes: impl FnMut() -> Result<(), String>,
) -> Result<(Vec<Probe>, Vec<ProbeExtra>), String> {
    let dims = spec.dimensions()?;
    let count = dims.iter().map(|&v| v as usize).product::<usize>();
    let bvh = Bvh::build(&scene.triangles);
    let sides = closed_sides(&scene.triangles);
    let mut probes = Vec::with_capacity(count);
    let mut extras = Vec::with_capacity(count);
    for index in 0..count {
        between_probes()?;
        let (extra, mean) = probe_geometry(
            scene,
            &bvh,
            &sides,
            spec.position(dims, index),
            spec.spacing,
        );
        probes.push(Probe {
            visibility: mean,
            ..Probe::default()
        });
        extras.push(extra);
    }
    Ok((probes, extras))
}

fn relocated(spec: GridSpec, dims: [u32; 3], index: usize, extra: &ProbeExtra) -> [f32; 3] {
    let position = spec.position(dims, index);
    std::array::from_fn(|axis| position[axis] + extra.offset[axis])
}

fn round_seed(seed: u32, round: u32) -> u32 {
    seed.wrapping_add(round.wrapping_mul(0x27d4_eb2f))
}

fn luminance(rgb: [f32; 3]) -> f32 {
    rgb[0] + rgb[1] + rgb[2]
}

fn kept(rounds: &[Lobes]) -> Vec<Lobes> {
    let sums: [f32; 6] =
        std::array::from_fn(|lobe| rounds.iter().map(|round| luminance(round[lobe])).sum());
    let others = (rounds.len() - 1).max(1) as f32;
    rounds
        .iter()
        .map(|round| {
            let rest: [f32; 6] =
                std::array::from_fn(|lobe| (sums[lobe] - luminance(round[lobe])) / others);
            let floor = rest.iter().fold(0.0f32, |a, &b| a.max(b)) * FLOOR + 1e-6;
            std::array::from_fn(|lobe| {
                let cap = rest[lobe] * OUTLIER + floor;
                let value = luminance(round[lobe]);
                if value > cap {
                    round[lobe].map(|channel| channel * cap / value)
                } else {
                    round[lobe]
                }
            })
        })
        .collect()
}

fn mean(rounds: &[Lobes]) -> Lobes {
    let share = 1.0 / rounds.len().max(1) as f32;
    std::array::from_fn(|lobe| {
        std::array::from_fn(|channel| {
            rounds.iter().map(|round| round[lobe][channel]).sum::<f32>() * share
        })
    })
}

fn estimate(rounds: &[Lobes]) -> (Lobes, [f32; 6]) {
    if rounds.is_empty() {
        return ([[0.0; 3]; 6], [0.0; 6]);
    }
    let kept = kept(rounds);
    let count = kept.len() as f32;
    let average = mean(&kept);
    let error = std::array::from_fn(|lobe| {
        let centre = luminance(average[lobe]);
        let variance = kept
            .iter()
            .map(|round| (luminance(round[lobe]) - centre).powi(2))
            .sum::<f32>()
            / (count - 1.0).max(1.0);
        (variance / count).sqrt()
    });
    (average, error)
}

fn converged(rounds: &[Lobes]) -> bool {
    let (average, error) = estimate(rounds);
    let level = average.map(luminance);
    let floor = level.iter().fold(0.0f32, |a, &b| a.max(b)) * FLOOR;
    (0..6).all(|lobe| error[lobe] <= SPREAD * level[lobe].max(floor))
}

fn converge(
    enabled: &[bool],
    mut sample: impl FnMut(u32, &[usize]) -> Result<Vec<Lobes>, String>,
) -> Result<(Estimates, Rounds), String> {
    let mut rounds: Vec<Vec<Lobes>> = vec![Vec::new(); enabled.len()];
    let mut active: Vec<usize> = (0..enabled.len()).filter(|&i| enabled[i]).collect();
    let mut report = Rounds {
        enabled: active.len() as u32,
        active: Vec::new(),
    };
    let mut round = 0;
    while !active.is_empty() && round < MAX_ROUNDS {
        let fresh = sample(round, &active)?;
        for (&index, lobes) in active.iter().zip(fresh) {
            rounds[index].push(lobes);
        }
        round += 1;
        if round >= ROUNDS {
            active.retain(|&index| !converged(&rounds[index]));
        }
        report.active.push(active.len() as u32);
    }
    Ok((rounds.iter().map(|list| estimate(list)).collect(), report))
}

fn smooth(
    dims: [u32; 3],
    spacing: f32,
    probes: &mut [Probe],
    extras: &[ProbeExtra],
    errors: &[[f32; 6]],
) {
    for _ in 0..PASSES {
        let source: Vec<Lobes> = probes.iter().map(|probe| probe.lobes).collect();
        for index in 0..probes.len() {
            if !extras[index].enabled {
                continue;
            }
            let xyz = [
                index as u32 % dims[0],
                index as u32 / dims[0] % dims[1],
                index as u32 / dims[0] / dims[1],
            ];
            let mut neighbours = Vec::with_capacity(6);
            for axis in 0..3 {
                for sign in [-1i32, 1] {
                    let step = xyz[axis] as i32 + sign;
                    if step < 0 || step >= dims[axis] as i32 {
                        continue;
                    }
                    let mut other = xyz;
                    other[axis] = step as u32;
                    let other = GridSpec::index(dims, other);
                    let outgoing = axis * 2 + usize::from(sign < 0);
                    let incoming = axis * 2 + usize::from(sign > 0);
                    if extras[other].enabled
                        && probes[index].visibility[outgoing] >= spacing * REACH
                        && probes[other].visibility[incoming] >= spacing * REACH
                    {
                        neighbours.push(other);
                    }
                }
            }
            let level = source[index]
                .iter()
                .map(|&rgb| luminance(rgb))
                .fold(0.0f32, f32::max)
                * FLOOR;
            for (lobe, &centre) in source[index].iter().enumerate() {
                let own = luminance(centre);
                let mut sum = centre;
                let mut total = 1.0;
                for &other in &neighbours {
                    let value = luminance(source[other][lobe]);
                    let scale = (own.max(value).max(level).max(1e-6) * RANGE).max(
                        2.0 * (errors[index][lobe].powi(2) + errors[other][lobe].powi(2)).sqrt(),
                    );
                    let weight = (-((value - own) / scale).powi(2)).exp();
                    for (channel, sum) in sum.iter_mut().enumerate() {
                        *sum += source[other][lobe][channel] * weight;
                    }
                    total += weight;
                }
                probes[index].lobes[lobe] = sum.map(|value| value / total);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn bake_layer(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    anchor_index: usize,
    direct: bool,
    paced: &mut Paced<'_>,
) -> Result<Baked, String> {
    check(scene, samples)?;
    let dims = spec.dimensions()?;
    let (mut probes, extras) = geometry(scene, spec, || (paced.between)(f64::NAN).map(|_| ()))?;
    let mut batch = Batch::new(gpu, scene, anchor_index)?;
    if !direct {
        batch = batch.without_direct_emission();
    }
    let per_round = samples.div_ceil(ROUNDS);
    let enabled: Vec<bool> = extras.iter().map(|extra| extra.enabled).collect();
    let most = FACES_PER_BATCH / FACES.len();
    let mut full_cost: Option<f64> = None;
    let (estimates, rounds) = converge(&enabled, |round, active| {
        let mut out = Vec::with_capacity(active.len());
        let mut at = 0;
        while at < active.len() {
            let probes = paced
                .pacer
                .learned_units(BATCH_LABEL)
                .map_or(V1_PROBES, |units| units as usize)
                .clamp(1, most);
            let chunk = &active[at..(at + probes).min(active.len())];
            at += chunk.len();
            let records = face_records(
                chunk,
                round,
                |index| relocated(spec, dims, index, &extras[index]),
                seed,
                anchor_index,
            );
            batch.load(gpu, &records)?;
            let target = paced.pacer.target_ms();
            paced
                .pacer
                .teach(SAMPLES_LABEL, first_samples(full_cost, target, per_round));
            let stats = paced.run_gpu(SAMPLES_LABEL, gpu, per_round, |encoder, start, count| {
                batch.encode(gpu, encoder, records.len(), start, count);
            })?;
            let per_sample = stats.milliseconds.iter().sum::<f64>() / f64::from(per_round);
            let next = batch_probes(chunk.len(), probes, per_sample, target, most);
            if chunk.len() == probes {
                full_cost = Some(per_sample * next as f64 / probes as f64);
            }
            paced.pacer.teach(BATCH_LABEL, next as u32);
            let image = batch.read(gpu)?;
            out.extend((0..chunk.len()).map(|slot| lobes_at(&image, slot)));
        }
        Ok(out)
    })?;
    let errors: Vec<[f32; 6]> = estimates.iter().map(|(_, error)| *error).collect();
    for (probe, (lobes, _)) in probes.iter_mut().zip(estimates) {
        probe.lobes = lobes;
    }
    smooth(dims, spec.spacing, &mut probes, &extras, &errors);
    Ok(Baked {
        grid: Grid::with_extras(spec, probes, extras)?,
        rounds,
    })
}

fn batch_probes(
    used: usize,
    asked: usize,
    per_sample_ms: f64,
    target_ms: f64,
    most: usize,
) -> usize {
    if !per_sample_ms.is_finite() || per_sample_ms <= 0.0 {
        return asked.clamp(1, most);
    }
    if per_sample_ms > target_ms * 1.15 {
        ((used as f64 * target_ms / per_sample_ms).floor() as usize).clamp(1, asked.max(1))
    } else if per_sample_ms < target_ms / 2.0 && used == asked {
        (asked * 2).min(most)
    } else {
        asked.clamp(1, most)
    }
}

fn first_samples(full_cost: Option<f64>, target_ms: f64, per_round: u32) -> u32 {
    full_cost
        .filter(|ms| ms.is_finite() && *ms > 0.0)
        .map_or(1, |ms| (target_ms / ms).floor() as u32)
        .clamp(1, per_round.max(1))
}

fn face_records(
    chunk: &[usize],
    round: u32,
    origin: impl Fn(usize) -> [f32; 3],
    seed: u32,
    anchor_index: usize,
) -> Vec<FaceRecord> {
    let mut records = Vec::with_capacity(chunk.len() * FACES.len());
    for &index in chunk {
        let origin = origin(index);
        for (face, &basis) in FACES.iter().enumerate() {
            records.push(FaceRecord::new(
                origin,
                basis,
                ray_seed(round_seed(seed, round), anchor_index, index, face),
            ));
        }
    }
    records
}

fn lobes_at(image: &[u8], slot: usize) -> Lobes {
    let mut lobes = [[0.0; 3]; 6];
    for face in 0..FACES.len() {
        let face_index = slot * FACES.len() + face;
        for y in 0..FACE {
            for x in 0..FACE {
                let tile_x = face_index % 16;
                let tile_y = face_index / 16;
                let offset = ((tile_y * FACE + y) * 64 + tile_x * FACE + x) * 16;
                accumulate(&mut lobes, image, offset, face, x, y);
            }
        }
    }
    lobes
}

fn ray_seed(seed: u32, anchor_index: usize, index: usize, face: usize) -> u32 {
    seed.wrapping_add((anchor_index as u32).wrapping_mul(0x9e37_79b9))
        .wrapping_add((index as u32).wrapping_mul(0x85eb_ca6b))
        .wrapping_add(face as u32)
}

fn accumulate(lobes: &mut Lobes, image: &[u8], offset: usize, face: usize, x: usize, y: usize) {
    let rgb: [f32; 3] = [0, 1, 2].map(|channel| {
        f32::from_le_bytes(
            image[offset + channel * 4..offset + channel * 4 + 4]
                .try_into()
                .unwrap(),
        )
    });
    let (dir, solid_angle) = direction(face, x, y);
    for (lobe, axis_direction) in AXES.iter().enumerate() {
        let cosine = (0..3)
            .map(|axis| dir[axis] * axis_direction[axis])
            .sum::<f32>()
            .max(0.0);
        for (channel, value) in rgb.iter().enumerate() {
            lobes[lobe][channel] += value.max(0.0) * cosine * solid_angle;
        }
    }
}

pub fn bake_anchor_slow(
    gpu: &Gpu,
    scene: &BakeScene,
    spec: GridSpec,
    samples: u32,
    seed: u32,
    anchor_index: usize,
) -> Result<Grid, String> {
    check(scene, samples)?;
    let anchor = scene
        .anchors
        .get(anchor_index)
        .ok_or("bake anchor is missing")?;
    let dims = spec.dimensions()?;
    let (mut probes, extras) = geometry(scene, spec, || Ok(()))?;
    let mut trace_scene = Scene {
        triangles: scene.triangles.clone(),
        shapes: scene.shapes.clone(),
        materials: scene.materials.clone(),
        sky: anchor.sky.clone(),
        camera: Camera {
            origin: [0.0; 3],
            forward: FACES[0].0,
            right: FACES[0].1,
            up: FACES[0].2,
        },
        sun: anchor.sun,
    };
    let mut trace = Trace::new(gpu, &trace_scene, FACE as u32, FACE as u32)?;
    let per_round = samples.div_ceil(ROUNDS);
    let enabled: Vec<bool> = extras.iter().map(|extra| extra.enabled).collect();
    let (estimates, _) = converge(&enabled, |round, active| {
        let mut out = Vec::with_capacity(active.len());
        for &index in active {
            let origin = relocated(spec, dims, index, &extras[index]);
            let mut lobes = [[0.0; 3]; 6];
            for (face, &(forward, right, up)) in FACES.iter().enumerate() {
                trace_scene.camera = Camera {
                    origin,
                    forward,
                    right,
                    up,
                };
                trace.set_camera(trace_scene.camera)?;
                trace.sample(
                    gpu,
                    per_round,
                    ray_seed(round_seed(seed, round), anchor_index, index, face),
                )?;
                let image = trace.readback_color(gpu)?;
                for y in 0..FACE {
                    for x in 0..FACE {
                        accumulate(&mut lobes, &image, (y * FACE + x) * 16, face, x, y);
                    }
                }
            }
            out.push(lobes);
        }
        Ok(out)
    })?;
    let errors: Vec<[f32; 6]> = estimates.iter().map(|(_, error)| *error).collect();
    for (probe, (lobes, _)) in probes.iter_mut().zip(estimates) {
        probe.lobes = lobes;
    }
    smooth(dims, spec.spacing, &mut probes, &extras, &errors);
    Grid::with_extras(spec, probes, extras)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

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
    fn a_uniform_sky_integrates_to_pi_on_every_lobe() {
        for (lobe, axis) in AXES.iter().enumerate() {
            let mut irradiance = 0.0f64;
            for face in 0..FACES.len() {
                for y in 0..FACE {
                    for x in 0..FACE {
                        let (dir, solid_angle) = direction(face, x, y);
                        let cosine = (0..3).map(|k| dir[k] * axis[k]).sum::<f32>().max(0.0);
                        irradiance += f64::from(cosine * solid_angle);
                    }
                }
            }
            assert!(
                (irradiance - std::f64::consts::PI).abs() < 1e-5,
                "lobe {lobe} integrates a unit sky to {irradiance}"
            );
        }
    }

    #[test]
    fn backface_majority_disables_or_relocates_inside_probes() {
        let scene = BakeScene {
            triangles: Vec::new(),
            shapes: vec![Shape::RoundedBox {
                center: [0.0; 3],
                half: [0.2; 3],
                radius: 0.0,
                material: 0,
            }],
            materials: vec![Material::default()],
            anchors: Vec::new(),
        };
        let bvh = Bvh::build(&scene.triangles);
        let (buried, _) = probe_geometry(&scene, &bvh, &[], [0.0; 3], 0.1);
        assert!(!buried.enabled);
        assert!(buried.backfaces > 12);
        let (near, _) = probe_geometry(&scene, &bvh, &[], [0.19, 0.0, 0.0], 0.1);
        assert!(near.enabled);
        assert!(near.offset.iter().map(|v| v * v).sum::<f32>().sqrt() <= 0.05);
        let (outside, _) = probe_geometry(&scene, &bvh, &[], [0.3, 0.0, 0.0], 0.1);
        assert!(outside.enabled);
        assert_eq!(outside.offset, [0.0; 3]);
    }

    fn cube(triangles: &mut Vec<Triangle>, center: [f32; 3], half: f32, inward: bool) {
        let corner = |x: f32, y: f32, z: f32| {
            [
                center[0] + x * half,
                center[1] + y * half,
                center[2] + z * half,
            ]
        };
        let faces = [
            [
                corner(1.0, -1.0, -1.0),
                corner(1.0, 1.0, -1.0),
                corner(1.0, 1.0, 1.0),
                corner(1.0, -1.0, 1.0),
            ],
            [
                corner(-1.0, -1.0, -1.0),
                corner(-1.0, -1.0, 1.0),
                corner(-1.0, 1.0, 1.0),
                corner(-1.0, 1.0, -1.0),
            ],
            [
                corner(-1.0, 1.0, -1.0),
                corner(-1.0, 1.0, 1.0),
                corner(1.0, 1.0, 1.0),
                corner(1.0, 1.0, -1.0),
            ],
            [
                corner(-1.0, -1.0, -1.0),
                corner(1.0, -1.0, -1.0),
                corner(1.0, -1.0, 1.0),
                corner(-1.0, -1.0, 1.0),
            ],
            [
                corner(-1.0, -1.0, 1.0),
                corner(1.0, -1.0, 1.0),
                corner(1.0, 1.0, 1.0),
                corner(-1.0, 1.0, 1.0),
            ],
            [
                corner(-1.0, -1.0, -1.0),
                corner(-1.0, 1.0, -1.0),
                corner(1.0, 1.0, -1.0),
                corner(1.0, -1.0, -1.0),
            ],
        ];
        for [a, b, c, d] in faces {
            if inward {
                quad(triangles, a, d, c, b, 0);
            } else {
                quad(triangles, a, b, c, d, 0);
            }
        }
    }

    #[test]
    fn only_closed_consistent_meshes_carry_a_side() {
        let mut triangles = Vec::new();
        cube(&mut triangles, [0.0; 3], 0.2, false);
        cube(&mut triangles, [1.0, 0.0, 0.0], 0.2, true);
        quad(
            &mut triangles,
            [-1.0, -0.5, -1.0],
            [-1.0, -0.5, 1.0],
            [2.0, -0.5, 1.0],
            [2.0, -0.5, -1.0],
            0,
        );
        let mut broken = Vec::new();
        cube(&mut broken, [0.0, 1.0, 0.0], 0.2, false);
        broken[0].vertices.swap(1, 2);
        triangles.extend(broken);
        let sides = closed_sides(&triangles);
        assert_eq!(&sides[..12], &[1; 12]);
        assert_eq!(&sides[12..24], &[-1; 12]);
        assert_eq!(&sides[24..], &[0; 14]);
        for (center, inward) in [([0.0; 3], false), ([1.0, 0.0, 0.0], true)] {
            let mut triangles = Vec::new();
            cube(&mut triangles, center, 0.2, inward);
            let scene = BakeScene {
                triangles,
                shapes: Vec::new(),
                materials: vec![Material::default()],
                anchors: Vec::new(),
            };
            let bvh = Bvh::build(&scene.triangles);
            let sides = closed_sides(&scene.triangles);
            let (inside, _) = probe_geometry(&scene, &bvh, &sides, center, 0.1);
            assert!(!inside.enabled && inside.backfaces == 24);
            let beside = [center[0] + 0.3, center[1], center[2]];
            let (outside, _) = probe_geometry(&scene, &bvh, &sides, beside, 0.1);
            assert!(outside.enabled && outside.backfaces == 0);
        }
    }

    #[test]
    fn a_room_with_open_walls_keeps_its_probes_whatever_their_winding() {
        let mut triangles = Vec::new();
        for (a, b, c, d) in [
            (
                [-1.0, 0.0, -1.0],
                [1.0, 0.0, -1.0],
                [1.0, 0.0, 1.0],
                [-1.0, 0.0, 1.0],
            ),
            (
                [1.0, 0.0, -1.0],
                [1.0, 2.0, -1.0],
                [1.0, 2.0, 1.0],
                [1.0, 0.0, 1.0],
            ),
            (
                [-1.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 2.0, 1.0],
                [-1.0, 2.0, 1.0],
            ),
        ] {
            quad(&mut triangles, a, b, c, d, 0);
        }
        let scene = BakeScene {
            triangles,
            shapes: Vec::new(),
            materials: vec![Material::default()],
            anchors: Vec::new(),
        };
        let bvh = Bvh::build(&scene.triangles);
        let sides = closed_sides(&scene.triangles);
        assert!(sides.iter().all(|&side| side == 0));
        let (probe, _) = probe_geometry(&scene, &bvh, &sides, [0.5, 0.1, 0.5], 0.5);
        assert!(probe.enabled && probe.backfaces == 0 && probe.offset == [0.0; 3]);
    }

    #[test]
    fn rounds_cap_a_firefly_keep_sparse_light_and_stop_once_they_agree() {
        let mut calls = [0u32; 4];
        let (estimates, report) = converge(&[true, true, true, false], |round, active| {
            Ok(active
                .iter()
                .map(|&index| {
                    calls[index] += 1;
                    let mut lobes = [[1.0; 3]; 6];
                    if index == 1 && round == 1 {
                        lobes[2] = [500.0; 3];
                    }
                    if index == 2 {
                        lobes = [[if round % 2 == 0 { 0.0 } else { 2.0 }; 3]; 6];
                    }
                    lobes
                })
                .collect())
        })
        .unwrap();
        let lobes: Vec<Lobes> = estimates.iter().map(|(lobes, _)| *lobes).collect();
        assert_eq!(calls, [ROUNDS, MAX_ROUNDS, MAX_ROUNDS, 0]);
        assert_eq!(report.enabled, 3);
        assert_eq!(report.run(), MAX_ROUNDS);
        assert_eq!(report.active[..3], [3, 3, 3]);
        assert_eq!(report.active[3], 2);
        assert!(report.active[4..].iter().all(|&count| count == 2));
        let done = report.converged();
        assert_eq!(done.iter().sum::<u32>(), 1);
        assert_eq!(done[3], 1);
        assert_eq!(lobes[0], [[1.0; 3]; 6]);
        assert!(lobes[1][2].iter().all(|&v| (1.04..1.06).contains(&v)));
        assert_eq!(lobes[1][0], [1.0; 3]);
        assert_eq!(lobes[2], [[1.0; 3]; 6]);
        assert_eq!(lobes[3], [[0.0; 3]; 6]);
        assert_eq!(estimates[0].1, [0.0; 6]);
        assert!(estimates[2].1.iter().all(|&error| error > 0.3));
    }

    #[test]
    fn smoothing_evens_noise_keeps_real_steps_and_stops_at_surfaces() {
        let spacing = 0.02;
        let open = Probe {
            visibility: [spacing * 4.0; 6],
            ..Probe::default()
        };
        let extra = ProbeExtra {
            second: [0.0; 6],
            offset: [0.0; 3],
            enabled: true,
            backfaces: 0,
        };
        let row = |values: [f32; 5]| -> Vec<Probe> {
            values
                .iter()
                .map(|&value| Probe {
                    lobes: [[value; 3]; 6],
                    ..open.clone()
                })
                .collect()
        };
        let extras = vec![extra; 5];
        let exact = [[0.0; 6]; 5];
        let mut noisy = row([1.0, 1.0, 1.1, 1.0, 1.0]);
        smooth([5, 1, 1], spacing, &mut noisy, &extras, &exact);
        assert!((noisy[2].lobes[0][0] - 1.0).abs() < 0.05);
        let mut rough = row([1.0, 1.0, 1.6, 1.0, 1.0]);
        smooth([5, 1, 1], spacing, &mut rough, &extras, &exact);
        assert!(rough[2].lobes[0][0] > 1.55);
        let mut measured = row([1.0, 1.0, 1.6, 1.0, 1.0]);
        smooth([5, 1, 1], spacing, &mut measured, &extras, &[[1.0; 6]; 5]);
        assert!(measured[2].lobes[0][0] < 1.35);
        let mut step = row([1.0, 1.0, 1.0, 5.0, 5.0]);
        smooth([5, 1, 1], spacing, &mut step, &extras, &[[0.3; 6]; 5]);
        assert!(step[2].lobes[0][0] < 1.001 && step[3].lobes[0][0] > 4.999);
        let mut wall = row([1.0, 1.0, 1.1, 1.0, 1.0]);
        wall[2].visibility[0] = spacing * 0.3;
        wall[2].visibility[1] = spacing * 0.3;
        smooth([5, 1, 1], spacing, &mut wall, &extras, &[[0.3; 6]; 5]);
        assert_eq!(wall[2].lobes[0][0], 1.1);
    }

    #[test]
    fn batches_shrink_when_one_sample_overruns_and_grow_back_when_cheap() {
        assert_eq!(batch_probes(512, 512, 16.0, 3.5, 512), 112);
        assert_eq!(batch_probes(112, 112, 3.0, 3.5, 512), 112);
        assert_eq!(batch_probes(100, 100, 5.0, 3.5, 512), 70);
        assert_eq!(batch_probes(112, 112, 1.0, 3.5, 512), 224);
        assert_eq!(batch_probes(40, 112, 1.0, 3.5, 512), 112);
        assert_eq!(batch_probes(300, 300, 1.0, 3.5, 512), 512);
        assert_eq!(batch_probes(4, 4, 400.0, 3.5, 512), 1);
        assert_eq!(batch_probes(64, 64, f64::NAN, 3.5, 512), 64);
        assert_eq!(first_samples(None, 3.5, 16), 1);
        assert_eq!(first_samples(Some(0.5), 3.5, 16), 7);
        assert_eq!(first_samples(Some(0.1), 3.5, 16), 16);
        assert_eq!(first_samples(Some(9.0), 3.5, 16), 1);
        assert_eq!(first_samples(Some(f64::NAN), 3.5, 4), 1);
    }

    #[test]
    fn sampling_shader_parses() {
        let module = naga::front::wgsl::parse_str(crate::PROBE_WGSL).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    fn room() -> BakeScene {
        let mut triangles = Vec::new();
        quad(
            &mut triangles,
            [-1.5, 0.0, -1.5],
            [1.5, 0.0, -1.5],
            [1.5, 0.0, 1.5],
            [-1.5, 0.0, 1.5],
            0,
        );
        quad(
            &mut triangles,
            [-1.5, 2.0, -1.5],
            [0.0, 2.0, -1.5],
            [0.0, 2.0, 1.5],
            [-1.5, 2.0, 1.5],
            1,
        );
        quad(
            &mut triangles,
            [-1.5, 0.0, -1.5],
            [-1.5, 2.0, -1.5],
            [-1.5, 2.0, 1.5],
            [-1.5, 0.0, 1.5],
            1,
        );
        quad(
            &mut triangles,
            [1.5, 0.0, -1.5],
            [1.5, 2.0, -1.5],
            [1.5, 2.0, 1.5],
            [1.5, 0.0, 1.5],
            1,
        );
        quad(
            &mut triangles,
            [-1.5, 0.0, -1.5],
            [1.5, 0.0, -1.5],
            [1.5, 2.0, -1.5],
            [-1.5, 2.0, -1.5],
            1,
        );
        quad(
            &mut triangles,
            [-1.5, 0.0, 1.5],
            [1.5, 0.0, 1.5],
            [1.5, 2.0, 1.5],
            [-1.5, 2.0, 1.5],
            1,
        );
        BakeScene {
            triangles,
            shapes: Vec::new(),
            materials: vec![
                Material {
                    base: [0.9; 3],
                    specular: 0.0,
                    roughness: 1.0,
                    ..Material::default()
                },
                Material {
                    base: [0.01; 3],
                    specular: 0.0,
                    roughness: 1.0,
                    ..Material::default()
                },
            ],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 4,
                    height: 2,
                    texels: vec![[0.0; 4]; 8],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 20.0,
                },
            }],
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn room_sunlight_determinism_and_timing() {
        let started = Instant::now();
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let scene = room();
        let spec = GridSpec {
            min: [-0.75, 0.25, -0.75],
            max: [0.75, 1.75, 0.75],
            spacing: 0.5,
        };
        let slow_started = Instant::now();
        let slow = bake_anchor_slow(&gpu, &scene, spec, 4, 73, 0).unwrap();
        let slow_seconds = slow_started.elapsed().as_secs_f64();
        let batch_started = Instant::now();
        let first = bake(&gpu, &scene, spec, 4, 73).unwrap();
        let batch_seconds = batch_started.elapsed().as_secs_f64();
        let second = bake(&gpu, &scene, spec, 4, 73).unwrap();
        assert_eq!(first[0].dims, [4; 3]);
        assert_eq!(first[0].bytes(), slow.bytes());
        assert_eq!(first[0].bytes(), second[0].bytes());
        let dark = &first[0].probes[GridSpec::index([4; 3], [0, 0, 2])];
        let bright = &first[0].probes[GridSpec::index([4; 3], [3, 0, 2])];
        assert!(
            bright.lobes[3][0] > dark.lobes[3][0] * 2.0 + 0.01,
            "bright={} dark={}",
            bright.lobes[3][0],
            dark.lobes[3][0]
        );
        let multi_spec = GridSpec {
            max: [1.25, 1.75, 0.75],
            ..spec
        };
        let multi_slow = bake_anchor_slow(&gpu, &scene, multi_spec, 1, 73, 0).unwrap();
        let multi_batch = bake_anchor(&gpu, &scene, multi_spec, 1, 73, 0).unwrap();
        assert_eq!(multi_batch.bytes(), multi_slow.bytes());
        eprintln!(
            "bake room GPU test: slow {slow_seconds:.3} s, batch {batch_seconds:.3} s, total {:.3} s",
            started.elapsed().as_secs_f64(),
        );
    }
    fn fixed_64_bake(
        gpu: &Gpu,
        scene: &BakeScene,
        spec: GridSpec,
        samples: u32,
        seed: u32,
    ) -> Grid {
        let dims = spec.dimensions().unwrap();
        let (mut probes, extras) = geometry(scene, spec, || Ok(())).unwrap();
        let batch = Batch::new(gpu, scene, 0).unwrap();
        let mut turns = Turns::default();
        let enabled: Vec<bool> = extras.iter().map(|extra| extra.enabled).collect();
        let (estimates, _) = converge(&enabled, |round, active| {
            let mut out = Vec::new();
            for chunk in active.chunks(64) {
                let records = face_records(
                    chunk,
                    round,
                    |index| relocated(spec, dims, index, &extras[index]),
                    seed,
                    0,
                );
                let image = turns.time(|| batch.sample(gpu, &records, samples.div_ceil(ROUNDS)))?;
                out.extend((0..chunk.len()).map(|slot| lobes_at(&image, slot)));
            }
            Ok(out)
        })
        .unwrap();
        let errors: Vec<[f32; 6]> = estimates.iter().map(|(_, error)| *error).collect();
        for (probe, (lobes, _)) in probes.iter_mut().zip(estimates) {
            probe.lobes = lobes;
        }
        smooth(dims, spec.spacing, &mut probes, &extras, &errors);
        Grid::with_extras(spec, probes, extras).unwrap()
    }

    fn paced_bake(
        gpu: &Gpu,
        scene: &BakeScene,
        spec: GridSpec,
        fixed: Option<u32>,
    ) -> (Grid, Vec<f64>, f64) {
        let mut pacer = Pacer::default();
        pacer.set_fixed_units(fixed);
        let mut slices = Vec::new();
        let mut turns = Turns::default();
        let mut between = |ms: f64| {
            if ms.is_finite() {
                slices.push(ms);
            }
            Ok(turns.add(ms))
        };
        let started = Instant::now();
        let grid = bake_anchor_paced(
            gpu,
            scene,
            spec,
            64,
            73,
            0,
            &mut Paced {
                pacer: &mut pacer,
                between: &mut between,
            },
        )
        .unwrap()
        .grid;
        let seconds = started.elapsed().as_secs_f64();
        assert!(pacer.pacing().slices <= slices.len());
        (grid, slices, seconds)
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn paced_and_fixed_batches_bake_the_same_bytes() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let scene = room();
        let spec = GridSpec {
            min: [-1.25, 0.25, -1.25],
            max: [1.25, 1.75, 1.25],
            spacing: 0.25,
        };
        let started = Instant::now();
        let fixed = fixed_64_bake(&gpu, &scene, spec, 64, 73);
        let fixed_seconds = started.elapsed().as_secs_f64();
        assert!(fixed.probes.len() > FACES_PER_BATCH / FACES.len());
        let reference = fixed.bytes();
        for samples in [1, 3, 16] {
            let (grid, slices, _) = paced_bake(&gpu, &scene, spec, Some(samples));
            assert!(grid.bytes() == reference, "{samples} samples per dispatch");
            assert!(!slices.is_empty());
        }
        let (paced, slices, paced_seconds) = paced_bake(&gpu, &scene, spec, None);
        assert!(paced.bytes() == reference, "paced dispatches");
        assert!(bake_anchor(&gpu, &scene, spec, 64, 73, 0).unwrap().bytes() == reference);
        let mut sorted = slices.clone();
        sorted.sort_by(f64::total_cmp);
        eprintln!(
            "probe bake, {} probes, 64 samples: 64 probes a dispatch {:.3} s; paced {} dispatches, first {:.2} ms, median {:.2} ms, longest {:.2} ms, {:.3} s",
            fixed.probes.len(),
            fixed_seconds,
            sorted.len(),
            slices[0],
            sorted[sorted.len() / 2],
            sorted[sorted.len() - 1],
            paced_seconds,
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn emission_scale_zero_removes_the_lamps_bounce() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut triangles = Vec::new();
        quad(
            &mut triangles,
            [-1.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 0.0, -1.0],
            [-1.0, 0.0, -1.0],
            0,
        );
        quad(
            &mut triangles,
            [-0.2, 1.0, -0.2],
            [0.2, 1.0, -0.2],
            [0.2, 1.0, 0.2],
            [-0.2, 1.0, 0.2],
            1,
        );
        let scene = BakeScene {
            triangles,
            shapes: Vec::new(),
            materials: vec![
                Material {
                    base: [0.8; 3],
                    specular: 0.0,
                    roughness: 1.0,
                    ..Material::default()
                },
                Material {
                    base: [0.0; 3],
                    emission: [5.0, 3.5, 2.25],
                    ..Material::default()
                },
            ],
            anchors: vec![Anchor {
                hour: 22.0,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[0.0; 4]],
                },
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [0.0; 3],
                    intensity: 0.0,
                },
            }],
        };
        let spec = GridSpec {
            min: [0.0, 0.2, 0.0],
            max: [0.0, 0.6, 0.0],
            spacing: 0.2,
        };
        let off = bake_scaled(&gpu, &scene, spec, 8, 11, &[0.0]).unwrap();
        let on = bake_scaled(&gpu, &scene, spec, 8, 11, &[1.0]).unwrap();
        let double = bake_scaled(&gpu, &scene, spec, 8, 11, &[2.0]).unwrap();
        let layer = bake_emitters(&gpu, &scene, spec, 8, 11, true).unwrap();
        let bounce = bake_emitters(&gpu, &scene, spec, 8, 11, false).unwrap();
        assert!(
            off[0].probes.iter().all(|probe| probe
                .lobes
                .iter()
                .flatten()
                .all(|&value| value == 0.0))
        );
        assert_eq!(layer.bytes(), on[0].bytes());
        for ((one, two), (direct, bounced)) in on[0]
            .probes
            .iter()
            .zip(&double[0].probes)
            .zip(layer.probes.iter().zip(&bounce.probes))
        {
            for lobe in 0..6 {
                for channel in 0..3 {
                    let value = one.lobes[lobe][channel];
                    assert!((two.lobes[lobe][channel] - 2.0 * value).abs() <= value * 0.01 + 1e-4);
                }
            }
            let up = direct.lobes[2][0];
            let down = direct.lobes[3][0];
            println!(
                "probe: up {up:.4} down {down:.4}, bounce only up {:.4} down {:.4}",
                bounced.lobes[2][0], bounced.lobes[3][0]
            );
            assert!(down > 0.0 && up > down);
            assert!(bounced.lobes[2][0] < up * 0.5);
            assert!((bounced.lobes[3][0] - down).abs() <= down * 0.05);
        }
    }
}
