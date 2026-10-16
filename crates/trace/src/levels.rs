use crate::bvh::{Bounds, Bvh, Node};
use crate::detail::{Instance, Pose, Transform, TriangleDetail};

pub(crate) const INSTANCE_LEAF: u32 = 1;
const PARALLEL_TRIANGLES: usize = 1 << 15;

#[derive(Clone, Debug)]
pub(crate) enum Materials {
    One(u32),
    Each(Vec<u32>),
}

impl Materials {
    fn of(triangles: &[TriangleDetail], order: &[u32]) -> Self {
        let first = order.first().map_or(0, |&j| triangles[j as usize].material);
        if order
            .iter()
            .all(|&j| triangles[j as usize].material == first)
        {
            Self::One(first)
        } else {
            Self::Each(
                order
                    .iter()
                    .map(|&j| triangles[j as usize].material)
                    .collect(),
            )
        }
    }

    fn at(&self, index: usize) -> u32 {
        match self {
            Self::One(material) => *material,
            Self::Each(materials) => materials[index],
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Mesh {
    pub triangles: Vec<TriangleDetail>,
    pub order: Vec<u32>,
    pub nodes: Vec<Node>,
    pub root: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Placed {
    pub mesh: usize,
    pub base: u32,
    pub transform: Transform,
    pub word: u32,
    pub materials: Materials,
    pub bounds: Bounds,
    pub inverse: [f32; 12],
}

#[derive(Clone, Debug)]
pub(crate) struct Levels {
    pub meshes: Vec<Mesh>,
    pub placed: Vec<Option<Placed>>,
    pub world: Option<(Bounds, u32)>,
    pub world_nodes: Vec<Node>,
}

pub(crate) struct Laid {
    pub order: Vec<u32>,
    pub nodes: Vec<Node>,
    pub levels: Option<Levels>,
    pub bottom_builds: usize,
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
        {
            hash = (hash ^ u64::from(value.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

fn same_geometry(a: &[TriangleDetail], b: &[TriangleDetail]) -> bool {
    let bits = |t: &TriangleDetail| {
        let mut out = [0_u32; 24];
        for (slot, value) in t
            .vertices
            .iter()
            .chain(&t.normals)
            .flatten()
            .chain(t.uvs.iter().flatten())
            .enumerate()
        {
            out[slot] = value.to_bits();
        }
        out
    };
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| bits(x) == bits(y))
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

pub(crate) fn lay(
    detailed: &[TriangleDetail],
    world: usize,
    instances: &[Instance],
    starts: &[usize],
    words: &[u32],
) -> Result<Laid, String> {
    if instances.is_empty() {
        let triangles: Vec<_> = detailed
            .iter()
            .copied()
            .map(TriangleDetail::triangle)
            .collect();
        let bvh = Bvh::build(&triangles);
        return Ok(Laid {
            order: bvh.order,
            nodes: bvh.nodes,
            levels: None,
            bottom_builds: usize::from(!detailed.is_empty()),
        });
    }
    let world_triangles: Vec<_> = detailed[..world]
        .iter()
        .copied()
        .map(TriangleDetail::triangle)
        .collect();
    let world_bvh = Bvh::build(&world_triangles);
    let mut meshes: Vec<Mesh> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut placed = Vec::with_capacity(instances.len());
    let mut bottom_builds = usize::from(world > 0);
    let mut base = world as u32;
    for ((instance, &start), &word) in instances.iter().zip(starts).zip(words) {
        let len = instance.triangles.len();
        if len == 0 {
            placed.push(None);
            continue;
        }
        let hash = geometry_hash(&instance.triangles);
        let found = meshes.iter().zip(&hashes).position(|(mesh, &h)| {
            h == hash && mesh.order.len() == len && {
                let ordered: Vec<TriangleDetail> = mesh
                    .order
                    .iter()
                    .map(|&j| instance.triangles[j as usize])
                    .collect();
                same_geometry(&ordered, &mesh.triangles)
            }
        });
        let mesh = match found {
            Some(index) => index,
            None => {
                let triangles: Vec<_> = instance
                    .triangles
                    .iter()
                    .copied()
                    .map(TriangleDetail::triangle)
                    .collect();
                let bvh = Bvh::build(&triangles);
                bottom_builds += 1;
                meshes.push(Mesh {
                    triangles: bvh
                        .order
                        .iter()
                        .map(|&j| instance.triangles[j as usize])
                        .collect(),
                    order: bvh.order,
                    nodes: bvh.nodes,
                    root: 0,
                });
                hashes.push(hash);
                meshes.len() - 1
            }
        };
        let mut bounds = Bounds::empty();
        for triangle in &detailed[start..start + len] {
            for &vertex in &triangle.vertices {
                bounds.point(vertex);
            }
        }
        placed.push(Some(Placed {
            mesh,
            base,
            transform: instance.transform,
            word,
            materials: Materials::of(&instance.triangles, &meshes[mesh].order),
            bounds,
            inverse: Pose::new(instance.transform)?.inverse_rows(),
        }));
        base += len as u32;
    }
    let world_root = world_bvh.nodes.first().map(bounds_of);
    let mut levels = Levels {
        meshes,
        placed,
        world: world_root.map(|bounds| (bounds, 0)),
        world_nodes: Vec::new(),
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
    let mut order: Vec<u32> = world_bvh.order.clone();
    for (placed, &start) in levels.placed.iter().zip(starts) {
        if let Some(placed) = placed {
            order.extend(
                levels.meshes[placed.mesh]
                    .order
                    .iter()
                    .map(|&j| start as u32 + j),
            );
        }
    }
    let mut nodes = levels.top();
    nodes.extend_from_slice(&levels.world_nodes);
    for mesh in &levels.meshes {
        nodes.extend_from_slice(&mesh.nodes);
    }
    Ok(Laid {
        order,
        nodes,
        levels: Some(levels),
        bottom_builds,
    })
}

struct Group {
    bounds: Bounds,
    root: u32,
    base: u32,
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
                inverse: None,
            });
        }
        for placed in self.placed.iter().flatten() {
            groups.push(Group {
                bounds: placed.bounds,
                root: self.meshes[placed.mesh].root,
                base: placed.base,
                inverse: Some(placed.inverse),
            });
        }
        groups
    }

    fn group_count(&self) -> usize {
        usize::from(self.world.is_some()) + self.placed.iter().flatten().count()
    }

    pub(crate) fn top_len(&self) -> usize {
        let groups = self.group_count();
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
}

pub(crate) fn pose_into<T: Send>(
    pose: &Pose,
    source: &[TriangleDetail],
    materials: &Materials,
    out: &mut [T],
    pack: impl Fn(&TriangleDetail) -> T + Sync,
) -> Bounds {
    let work = |offset: usize, out: &mut [T]| {
        let mut bounds = Bounds::empty();
        for (k, slot) in out.iter_mut().enumerate() {
            let mut triangle = pose.triangle(&source[offset + k]);
            triangle.material = materials.at(offset + k);
            for &vertex in &triangle.vertices {
                bounds.point(vertex);
            }
            *slot = pack(&triangle);
        }
        bounds
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    if source.len() < PARALLEL_TRIANGLES || threads < 2 {
        return work(0, out);
    }
    let chunk = source.len().div_ceil(threads);
    let parts: Vec<Bounds> = std::thread::scope(|scope| {
        let handles: Vec<_> = out
            .chunks_mut(chunk)
            .enumerate()
            .map(|(index, part)| {
                let work = &work;
                scope.spawn(move || work(index * chunk, part))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("a posing thread panicked"))
            .collect()
    });
    let mut bounds = Bounds::empty();
    for part in parts {
        if part.lo[0] <= part.hi[0] {
            bounds.grow(part);
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bvh::{Ray, triangle_hit};
    use crate::detail::InstanceSurface;

    fn instance(transform: Transform, triangles: Vec<TriangleDetail>) -> Instance {
        Instance {
            triangles,
            transform,
            surface: InstanceSurface::default(),
            shadow_only: false,
            two_sided: true,
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
                    if let Some(hit) = triangle_hit(&world[slot as usize].triangle(), ray, limit) {
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
                        material: 0,
                    }
                })
                .collect()
        };
        let shared = blob(60);
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
            instance(turned(2.0, [0.0, -2.0, -1.0], 1.5), blob(40)),
        ];
        let mut detailed = world.clone();
        let mut starts = Vec::new();
        for instance in &instances {
            starts.push(detailed.len());
            detailed.extend(instance.transformed().unwrap());
        }
        let laid = lay(&detailed, world.len(), &instances, &starts, &[0, 0, 0]).unwrap();
        assert_eq!(laid.bottom_builds, 3);
        let slots: Vec<TriangleDetail> = laid
            .order
            .iter()
            .map(|&index| detailed[index as usize])
            .collect();
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
            let brute = slots
                .iter()
                .enumerate()
                .filter_map(|(slot, t)| {
                    triangle_hit(&t.triangle(), ray, f32::INFINITY)
                        .map(|hit| (slot as u32, hit.distance))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            assert_eq!(
                walk(&laid.nodes, &slots, ray).map(|h| h.0),
                brute.map(|h| h.0)
            );
        }
    }
}
