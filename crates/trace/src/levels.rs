use std::sync::Arc;

use pfx_core::memory;

use crate::bvh::{Bounds, Bvh, Node};
use crate::detail::{Instance, Pose, Transform, TriangleDetail};
use crate::motion::{Decomposed, decompose_steps, rows, swept};

pub(crate) const INSTANCE_LEAF: u32 = 1;

#[derive(Clone, Debug)]
pub(crate) struct Mesh {
    pub triangles: Vec<TriangleDetail>,
    pub order: Vec<u32>,
    pub nodes: Vec<Node>,
    pub root: u32,
    pub first: u32,
    pub bounds: Bounds,
}

#[derive(Clone, Debug)]
pub(crate) struct Placed {
    pub mesh: usize,
    pub base: u32,
    pub record: u32,
    pub transform: Transform,
    pub word: u32,
    pub material: Option<u32>,
    pub bounds: Bounds,
    pub inverse: [f32; 12],
    pub motion: Vec<Decomposed>,
    pub deform: usize,
}

impl Placed {
    pub(crate) fn moving(&self) -> bool {
        !self.motion.is_empty() || self.deform > 0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Levels {
    pub meshes: Vec<Mesh>,
    pub placed: Vec<Option<Placed>>,
    pub world: Option<(Bounds, u32)>,
    pub world_nodes: Vec<Node>,
    pub room: usize,
}

pub(crate) struct Laid {
    pub order: Vec<u32>,
    pub nodes: Vec<Node>,
    pub levels: Option<Levels>,
    pub bottom_builds: usize,
    pub slots: usize,
}

fn geometry_hash(triangles: &[TriangleDetail]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for triangle in triangles {
        for value in triangle
            .vertices
            .iter()
            .chain(&triangle.normals)
            .flatten()
            .chain(triangle.uvs.iter().flatten())
            .chain(triangle.uvs1.iter().flatten())
        {
            hash = (hash ^ u64::from(value.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

fn same_geometry(a: &TriangleDetail, b: &TriangleDetail) -> bool {
    let bits = |t: &TriangleDetail| {
        let mut out = [0_u32; 30];
        for (slot, value) in t
            .vertices
            .iter()
            .chain(&t.normals)
            .flatten()
            .chain(t.uvs.iter().flatten())
            .chain(t.uvs1.iter().flatten())
            .enumerate()
        {
            out[slot] = value.to_bits();
        }
        out
    };
    bits(a) == bits(b)
}

fn shifted(nodes: &[Node], offset: u32) -> Vec<Node> {
    nodes
        .iter()
        .map(|node| {
            let mut node = *node;
            if node.count == 0 {
                node.first += offset;
            }
            node
        })
        .collect()
}

fn bounds_of(node: &Node) -> Bounds {
    Bounds {
        lo: [node.lo[0], node.lo[1], node.lo[2]],
        hi: [node.hi[0], node.hi[1], node.hi[2]],
    }
}

pub(crate) fn placed_bounds(pose: &Pose, local: Bounds) -> Bounds {
    let mut bounds = Bounds::empty();
    for corner in 0..8 {
        let point = std::array::from_fn(|axis| {
            if corner >> axis & 1 == 0 {
                local.lo[axis]
            } else {
                local.hi[axis]
            }
        });
        bounds.point(pose.point(point));
    }
    bounds
}

fn one_material(triangles: &[TriangleDetail]) -> Option<u32> {
    let first = triangles.first()?.material;
    triangles
        .iter()
        .all(|triangle| triangle.material == first)
        .then_some(first)
}

fn refit(nodes: &mut [Node], index: usize, boxes: &[Bounds]) -> Bounds {
    let node = nodes[index];
    let bounds = if node.count > 0 {
        let mut bounds = Bounds::empty();
        for slot in node.first..node.first + node.count {
            bounds.grow(boxes[slot as usize]);
        }
        bounds
    } else {
        let mut bounds = refit(nodes, index + 1, boxes);
        bounds.grow(refit(nodes, node.first as usize, boxes));
        bounds
    };
    nodes[index].lo[..3].copy_from_slice(&bounds.lo);
    nodes[index].hi[..3].copy_from_slice(&bounds.hi);
    bounds
}

fn deformed_boxes(
    shapes: &[Vec<TriangleDetail>],
    triangles: &[TriangleDetail],
    order: &[u32],
) -> Vec<Bounds> {
    order
        .iter()
        .map(|&j| {
            let mut bounds = Bounds::empty();
            for shape in shapes
                .iter()
                .map(|shape| &shape[j as usize])
                .chain([&triangles[j as usize]])
            {
                for &vertex in &shape.vertices {
                    bounds.point(vertex);
                }
            }
            bounds
        })
        .collect()
}

pub(crate) fn motion_steps(transforms: &[Transform]) -> Vec<Decomposed> {
    if transforms.is_empty() {
        return Vec::new();
    }
    decompose_steps(&transforms.iter().map(rows).collect::<Vec<_>>())
}

pub(crate) fn place(
    meshes: &[Mesh],
    mesh: usize,
    instance: &Instance,
    word: u32,
    record: u32,
    moving: bool,
) -> Result<Placed, String> {
    let pose = Pose::new(instance.transform)?;
    let motion = instance
        .motion
        .as_ref()
        .filter(|motion| moving && motion.moves(&instance.transform));
    let steps = motion_steps(motion.map_or(&[][..], |motion| &motion.transforms));
    let local = meshes[mesh].bounds;
    let bounds = if steps.is_empty() {
        placed_bounds(&pose, local)
    } else {
        swept(local, &steps)
    };
    Ok(Placed {
        mesh,
        base: meshes[mesh].first,
        record,
        transform: instance.transform,
        word,
        material: one_material(&instance.triangles),
        bounds,
        inverse: pose.inverse_rows(),
        motion: steps,
        deform: motion.map_or(0, |motion| motion.shapes.len()),
    })
}

struct Known {
    source: Arc<[TriangleDetail]>,
    hash: u64,
    mesh: usize,
}

pub(crate) fn lay(
    world: &[TriangleDetail],
    instances: &[Instance],
    words: &[u32],
    unique: &[bool],
    moving: bool,
    room: usize,
) -> Result<Laid, String> {
    let mut world_triangles = memory::vec(world.len(), "the tracer's world triangles")?;
    world_triangles.extend(world.iter().copied().map(TriangleDetail::triangle));
    let world_bvh = Bvh::try_build(&world_triangles)?;
    drop(world_triangles);
    if instances.is_empty() {
        return Ok(Laid {
            order: world_bvh.order,
            nodes: world_bvh.nodes,
            levels: None,
            bottom_builds: usize::from(!world.is_empty()),
            slots: world.len(),
        });
    }
    let mut meshes: Vec<Mesh> = Vec::new();
    let mut known: Vec<Known> = Vec::new();
    let mut placed = Vec::with_capacity(instances.len());
    let mut bottom_builds = usize::from(!world.is_empty());
    let mut next = world.len() as u32;
    let mut record = 0u32;
    for ((instance, &word), &alone) in instances.iter().zip(words).zip(unique) {
        let len = instance.triangles.len();
        if len == 0 {
            placed.push(None);
            continue;
        }
        Pose::new(instance.transform)?;
        let motion = instance
            .motion
            .as_ref()
            .filter(|motion| moving && motion.moves(&instance.transform));
        let deform = motion.map_or(0, |motion| motion.shapes.len());
        let alone = alone || deform > 0;
        let material = one_material(&instance.triangles);
        let fits = |mesh: &Mesh| {
            material.is_some()
                || mesh
                    .order
                    .iter()
                    .zip(&mesh.triangles)
                    .all(|(&j, triangle)| {
                        instance.triangles[j as usize].material == triangle.material
                    })
        };
        let shared = known
            .iter()
            .any(|entry| Arc::ptr_eq(&entry.source, &instance.triangles));
        let mut hash = None;
        let found = if alone {
            None
        } else if let Some(entry) = known.iter().find(|entry| {
            Arc::ptr_eq(&entry.source, &instance.triangles) && fits(&meshes[entry.mesh])
        }) {
            Some(entry.mesh)
        } else {
            let hashed = *hash.get_or_insert_with(|| geometry_hash(&instance.triangles));
            known
                .iter()
                .find(|entry| {
                    let mesh = &meshes[entry.mesh];
                    entry.hash == hashed
                        && mesh.order.len() == len
                        && fits(mesh)
                        && mesh
                            .order
                            .iter()
                            .zip(&mesh.triangles)
                            .all(|(&j, triangle)| {
                                same_geometry(&instance.triangles[j as usize], triangle)
                            })
                })
                .map(|entry| entry.mesh)
        };
        let mesh = match found {
            Some(index) => index,
            None => {
                let mut triangles = memory::vec(len, "the tracer's mesh triangles")?;
                triangles.extend(
                    instance
                        .triangles
                        .iter()
                        .copied()
                        .map(TriangleDetail::triangle),
                );
                let mut bvh = Bvh::try_build(&triangles)?;
                drop(triangles);
                if let Some(motion) = motion.filter(|_| deform > 0) {
                    let boxes = deformed_boxes(&motion.shapes, &instance.triangles, &bvh.order);
                    refit(&mut bvh.nodes, 0, &boxes);
                }
                bottom_builds += 1;
                let bounds = bvh.nodes.first().map_or(Bounds::empty(), bounds_of);
                let mut ordered = memory::vec(len, "the tracer's mesh triangles")?;
                ordered.extend(bvh.order.iter().map(|&j| instance.triangles[j as usize]));
                memory::more(&mut meshes, 1, "the tracer's meshes")?;
                meshes.push(Mesh {
                    triangles: ordered,
                    order: bvh.order,
                    nodes: bvh.nodes,
                    root: 0,
                    first: next,
                    bounds,
                });
                next = next
                    .checked_add(len as u32)
                    .ok_or("trace scene has too many triangles")?;
                meshes.len() - 1
            }
        };
        if !alone && !shared {
            known.push(Known {
                source: instance.triangles.clone(),
                hash: hash.unwrap_or_else(|| geometry_hash(&instance.triangles)),
                mesh,
            });
        }
        record += 1;
        placed.push(Some(place(&meshes, mesh, instance, word, record, moving)?));
    }
    let world_root = world_bvh.nodes.first().map(bounds_of);
    let mut levels = Levels {
        meshes,
        placed,
        world: world_root.map(|bounds| (bounds, 0)),
        world_nodes: Vec::new(),
        room,
    };
    let mut offset = levels.top_len() as u32;
    if let Some(bounds) = world_root {
        levels.world = Some((bounds, offset));
        levels.world_nodes = shifted(&world_bvh.nodes, offset);
        offset += world_bvh.nodes.len() as u32;
    }
    for mesh in &mut levels.meshes {
        mesh.root = offset;
        mesh.nodes = shifted(&mesh.nodes, offset);
        offset += mesh.nodes.len() as u32;
    }
    let mut nodes = levels.top();
    nodes.resize(levels.top_len(), bytemuck::Zeroable::zeroed());
    nodes.extend_from_slice(&levels.world_nodes);
    for mesh in &levels.meshes {
        nodes.extend_from_slice(&mesh.nodes);
    }
    Ok(Laid {
        order: world_bvh.order,
        nodes,
        levels: Some(levels),
        bottom_builds,
        slots: next as usize,
    })
}

struct Group {
    bounds: Bounds,
    root: u32,
    base: u32,
    record: u32,
    inverse: Option<[f32; 12]>,
}

impl Levels {
    fn groups(&self) -> Vec<Group> {
        let mut groups = Vec::new();
        if let Some((bounds, root)) = self.world {
            groups.push(Group {
                bounds,
                root,
                base: 0,
                record: 0,
                inverse: None,
            });
        }
        for placed in self.placed.iter().flatten() {
            groups.push(Group {
                bounds: placed.bounds,
                root: self.meshes[placed.mesh].root,
                base: placed.base,
                record: placed.record,
                inverse: Some(placed.inverse),
            });
        }
        groups
    }

    fn group_count(&self) -> usize {
        usize::from(self.world.is_some()) + self.placed.iter().flatten().count()
    }

    pub(crate) fn top_len(&self) -> usize {
        let groups = self.group_count() + self.room;
        if groups == 0 { 0 } else { 3 * groups - 1 }
    }

    pub(crate) fn top(&self) -> Vec<Node> {
        let groups = self.groups();
        if groups.is_empty() {
            return Vec::new();
        }
        let boxes: Vec<Bounds> = groups.iter().map(|group| group.bounds).collect();
        let bvh = Bvh::build_instances(&boxes);
        let tree = bvh.nodes.len() as u32;
        let mut nodes: Vec<Node> = bvh
            .nodes
            .iter()
            .map(|node| {
                let mut node = *node;
                if node.count > 0 {
                    let index = bvh.order[node.first as usize] as usize;
                    let group = &groups[index];
                    node.first = group.root;
                    node.count = group.base;
                    node.lo[3] = f32::from_bits(group.record);
                    node._pad = [
                        INSTANCE_LEAF,
                        if group.inverse.is_some() {
                            tree + index as u32
                        } else {
                            0
                        },
                    ];
                }
                node
            })
            .collect();
        for group in &groups {
            let r = group
                .inverse
                .unwrap_or([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
            nodes.push(Node {
                lo: [r[0], r[1], r[2], r[3]],
                hi: [r[4], r[5], r[6], r[7]],
                first: r[8].to_bits(),
                count: r[9].to_bits(),
                _pad: [r[10].to_bits(), r[11].to_bits()],
            });
        }
        nodes
    }

    pub(crate) fn instance_count(&self) -> usize {
        self.placed.len()
    }

    pub(crate) fn set_motion(&mut self, index: usize, transforms: &[Transform]) {
        let Some(Some(placed)) = self.placed.get_mut(index) else {
            return;
        };
        let still = transforms
            .iter()
            .all(|transform| *transform == placed.transform);
        if still && placed.motion.is_empty() {
            return;
        }
        let local = self.meshes[placed.mesh].bounds;
        placed.motion = if still {
            Vec::new()
        } else {
            motion_steps(transforms)
        };
        if !placed.motion.is_empty() {
            placed.bounds = swept(local, &placed.motion);
        } else if let Ok(pose) = Pose::new(placed.transform) {
            placed.bounds = placed_bounds(&pose, local);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bvh::{Ray, triangle_hit};
    use crate::detail::InstanceSurface;

    fn instance(transform: Transform, triangles: Arc<[TriangleDetail]>) -> Instance {
        Instance {
            triangles,
            transform,
            surface: InstanceSurface::default(),
            shadow_only: false,
            two_sided: true,
            motion: None,
        }
    }

    fn walk(nodes: &[Node], world: &[TriangleDetail], ray: Ray) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        let mut stack = vec![(0u32, None::<(u32, Ray)>)];
        while let Some((index, inside)) = stack.pop() {
            let node = nodes[index as usize];
            let local = inside.map_or(ray, |(_, r)| r);
            let limit = best.map_or(f32::INFINITY, |(_, t)| t);
            if !hits_box(&node, local, limit) {
                continue;
            }
            if node._pad[0] == INSTANCE_LEAF && inside.is_none() {
                let local = if node._pad[1] == 0 {
                    ray
                } else {
                    let m = nodes[node._pad[1] as usize];
                    let r = [
                        m.lo,
                        m.hi,
                        [
                            f32::from_bits(m.first),
                            f32::from_bits(m.count),
                            f32::from_bits(m._pad[0]),
                            f32::from_bits(m._pad[1]),
                        ],
                    ];
                    let apply = |p: [f32; 3], w: f32| {
                        r.map(|row| row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3] * w)
                    };
                    Ray {
                        origin: apply(ray.origin, 1.0),
                        direction: apply(ray.direction, 0.0),
                    }
                };
                stack.push((node.first, Some((node.count, local))));
            } else if node.count == 0 {
                stack.push((node.first, inside));
                stack.push((index + 1, inside));
            } else {
                let base = inside.map_or(0, |(base, _)| base);
                for j in 0..node.count {
                    let slot = base + node.first + j;
                    let limit = best.map_or(f32::INFINITY, |(_, t)| t);
                    if let Some(hit) = triangle_hit(&world[slot as usize].triangle(), local, limit)
                    {
                        best = Some((slot, hit.distance));
                    }
                }
            }
        }
        best
    }

    fn hits_box(node: &Node, ray: Ray, limit: f32) -> bool {
        let mut near = 0.0f32;
        let mut far = limit;
        for axis in 0..3 {
            let d = ray.direction[axis];
            if d.abs() < 1e-12 {
                if ray.origin[axis] < node.lo[axis] || ray.origin[axis] > node.hi[axis] {
                    return false;
                }
            } else {
                let a = (node.lo[axis] - ray.origin[axis]) / d;
                let b = (node.hi[axis] - ray.origin[axis]) / d;
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return false;
                }
            }
        }
        true
    }

    #[test]
    fn two_levels_find_the_hits_of_the_flattened_triangles() {
        let mut state = 29u32;
        let mut random = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut blob = |count: usize| -> Vec<TriangleDetail> {
            (0..count)
                .map(|_| {
                    let x = random() * 2.0 - 1.0;
                    let y = random() * 2.0 - 1.0;
                    let z = random() * 2.0 - 1.0;
                    TriangleDetail {
                        vertices: [
                            [x, y, z],
                            [x + random() * 0.4 + 0.05, y, z],
                            [x, y + random() * 0.4 + 0.05, z + 0.1],
                        ],
                        normals: [[0.0, 0.0, 1.0]; 3],
                        uvs: [[0.0; 2]; 3],
                        uvs1: [[0.0; 2]; 3],
                        material: 0,
                    }
                })
                .collect()
        };
        let shared: Arc<[TriangleDetail]> = blob(60).into();
        let world = blob(30);
        let turned = |angle: f32, at: [f32; 3], scale: f32| -> Transform {
            let (s, c) = angle.sin_cos();
            [
                [c * scale, 0.0, -s * scale, 0.0],
                [0.0, scale, 0.0, 0.0],
                [s * scale, 0.0, c * scale, 0.0],
                [at[0], at[1], at[2], 1.0],
            ]
        };
        let instances = vec![
            instance(turned(0.3, [2.0, 0.0, 0.0], 1.0), shared.clone()),
            instance(turned(-1.2, [-2.0, 0.5, 1.0], 0.7), shared.clone()),
            instance(turned(2.0, [0.0, -2.0, -1.0], 1.5), blob(40).into()),
            instance(
                turned(2.5, [1.0, 1.5, -2.0], 0.5),
                shared.iter().copied().collect(),
            ),
        ];
        let laid = lay(&world, &instances, &[0; 4], &[false; 4], false, 0).unwrap();
        assert_eq!(laid.bottom_builds, 3);
        assert_eq!(laid.slots, 30 + 60 + 40);
        let levels = laid.levels.as_ref().unwrap();
        let mut slots: Vec<TriangleDetail> = laid
            .order
            .iter()
            .map(|&index| world[index as usize])
            .collect();
        for mesh in &levels.meshes {
            assert_eq!(mesh.first as usize, slots.len());
            slots.extend_from_slice(&mesh.triangles);
        }
        let mut placed = world.clone();
        for instance in &instances {
            placed.extend(instance.transformed().unwrap());
        }
        for _ in 0..3000 {
            let ray = Ray {
                origin: [
                    random() * 10.0 - 5.0,
                    random() * 10.0 - 5.0,
                    random() * 10.0 - 5.0,
                ],
                direction: crate::math::normalize([
                    random() * 2.0 - 1.0,
                    random() * 2.0 - 1.0,
                    random() * 2.0 - 1.0,
                ]),
            };
            let brute = placed
                .iter()
                .filter_map(|t| triangle_hit(&t.triangle(), ray, f32::INFINITY))
                .map(|hit| hit.distance)
                .min_by(f32::total_cmp);
            let walked = walk(&laid.nodes, &slots, ray).map(|h| h.1);
            match (walked, brute) {
                (Some(a), Some(b)) => assert!((a - b).abs() <= 1e-4 * b.max(1.0), "{a} {b}"),
                (a, b) => assert_eq!(a, b),
            }
        }
    }

    #[test]
    fn moving_and_deforming_instances_are_bounded_over_the_whole_shutter() {
        let triangle = |lift: f32| TriangleDetail {
            vertices: [[-0.5, 0.0, 0.0], [0.5, 0.0, 0.0], [0.0, 1.0 + lift, 0.0]],
            normals: [[0.0, 0.0, 1.0]; 3],
            uvs: [[0.0; 2]; 3],
            uvs1: [[0.0; 2]; 3],
            material: 0,
        };
        let turned = |angle: f32, x: f32| -> Transform {
            let (s, c) = angle.sin_cos();
            [
                [c, s, 0.0, 0.0],
                [-s, c, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [x, 0.0, 0.0, 1.0],
            ]
        };
        let steps = [turned(-1.0, -2.0), turned(0.0, 0.0), turned(1.0, 2.0)];
        let mut spun = instance(steps[1], vec![triangle(0.0)].into());
        spun.motion = Some(crate::motion::InstanceMotion {
            transforms: steps.to_vec(),
            shapes: Vec::new(),
        });
        let mut grown = instance(turned(0.0, 5.0), vec![triangle(0.0)].into());
        grown.motion = Some(crate::motion::InstanceMotion {
            transforms: Vec::new(),
            shapes: vec![
                vec![triangle(-0.5)],
                vec![triangle(0.0)],
                vec![triangle(1.5)],
            ],
        });
        let instances = vec![spun, grown];
        let laid = lay(&[], &instances, &[0, 0], &[false, false], true, 0).unwrap();
        let levels = laid.levels.unwrap();
        let spun = levels.placed[0].as_ref().unwrap();
        let motion = crate::motion::decompose_steps(&steps.map(|step| rows(&step)));
        for k in 0..=50 {
            let at = crate::motion::interpolate(&motion, k as f32 / 50.0);
            for vertex in triangle(0.0).vertices {
                let p = crate::motion::point(&at, vertex);
                for axis in 0..3 {
                    assert!(p[axis] >= spun.bounds.lo[axis] - 1e-3, "{k} {p:?}");
                    assert!(p[axis] <= spun.bounds.hi[axis] + 1e-3, "{k} {p:?}");
                }
            }
        }
        let grown = levels.placed[1].as_ref().unwrap();
        assert_eq!((spun.deform, grown.deform), (0, 3));
        assert!(grown.motion.is_empty());
        assert!(levels.meshes[grown.mesh].bounds.hi[1] >= 2.5);
        assert!(grown.bounds.hi[1] >= 2.5);
        assert_ne!(spun.mesh, grown.mesh);
    }
}
