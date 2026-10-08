use std::collections::{BTreeMap, HashMap};

use crate::math::{add, dot, length, normalize, scale, sub};
use crate::mesh::Mesh;

const BLOCK: i32 = 8;
const SIDE: usize = BLOCK as usize + 1;
const REACH: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub voxel: f32,
    pub relax: u32,
}

impl Surface {
    pub fn new(voxel: f32, relax: u32) -> Self {
        Self { voxel, relax }
    }

    pub fn mesh<F: Fn([f32; 3]) -> f32>(&self, field: &F, lo: [f32; 3], hi: [f32; 3]) -> Mesh {
        mesh(field, lo, hi, self.voxel, self.relax)
    }
}

pub fn smooth_min(a: f32, b: f32, blend: f32) -> f32 {
    if blend <= 0.0 {
        return a.min(b);
    }
    let h = (blend - (a - b).abs()).max(0.0) / blend;
    a.min(b) - h * h * blend * 0.25
}

pub fn gradient<F: Fn([f32; 3]) -> f32>(field: &F, p: [f32; 3], eps: f32) -> [f32; 3] {
    let axis = |i: usize| {
        let mut plus = p;
        let mut minus = p;
        plus[i] += eps;
        minus[i] -= eps;
        (field(plus) - field(minus)) / (2.0 * eps)
    };
    [axis(0), axis(1), axis(2)]
}

struct Grid {
    origin: [f32; 3],
    voxel: f32,
    blocks: BTreeMap<[i32; 3], Vec<f32>>,
}

impl Grid {
    fn point(&self, cell: [i32; 3]) -> [f32; 3] {
        [
            self.origin[0] + cell[0] as f32 * self.voxel,
            self.origin[1] + cell[1] as f32 * self.voxel,
            self.origin[2] + cell[2] as f32 * self.voxel,
        ]
    }

    fn value(&self, block: &[f32], local: [i32; 3]) -> f32 {
        block[(local[2] as usize * SIDE + local[1] as usize) * SIDE + local[0] as usize]
    }
}

pub fn mesh<F: Fn([f32; 3]) -> f32>(
    field: &F,
    lo: [f32; 3],
    hi: [f32; 3],
    voxel: f32,
    relax: u32,
) -> Mesh {
    let voxel = voxel.max(1e-6);
    let pad = 2.0 * voxel;
    let origin = sub(lo, [pad; 3]);
    let counts = [0, 1, 2].map(|i| (((hi[i] - lo[i] + 2.0 * pad) / voxel).ceil() as i32).max(1));
    let block_counts = counts.map(|c| (c + BLOCK - 1) / BLOCK);
    let reach = REACH * 3f32.sqrt() * (BLOCK as f32 * 0.5 + 1.0) * voxel;
    let mut grid = Grid {
        origin,
        voxel,
        blocks: BTreeMap::new(),
    };
    for bz in 0..block_counts[2] {
        for by in 0..block_counts[1] {
            for bx in 0..block_counts[0] {
                let base = [bx * BLOCK, by * BLOCK, bz * BLOCK];
                let center = grid.point(base.map(|v| v + BLOCK / 2));
                if field(center).abs() > reach {
                    continue;
                }
                let mut values = Vec::with_capacity(SIDE * SIDE * SIDE);
                for z in 0..SIDE as i32 {
                    for y in 0..SIDE as i32 {
                        for x in 0..SIDE as i32 {
                            values.push(field(grid.point([base[0] + x, base[1] + y, base[2] + z])));
                        }
                    }
                }
                grid.blocks.insert([bx, by, bz], values);
            }
        }
    }
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut cells: Vec<([i32; 3], [i32; 3])> = Vec::new();
    let mut index: HashMap<[i32; 3], u32> = HashMap::new();
    for (key, values) in &grid.blocks {
        for z in 0..BLOCK {
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let local = [x, y, z];
                    let mut corners = [0.0f32; 8];
                    let mut inside = 0;
                    for (corner, slot) in corners.iter_mut().enumerate() {
                        let offset = [
                            (corner & 1) as i32,
                            ((corner >> 1) & 1) as i32,
                            (corner >> 2) as i32,
                        ];
                        *slot = grid.value(values, [x + offset[0], y + offset[1], z + offset[2]]);
                        if *slot < 0.0 {
                            inside += 1;
                        }
                    }
                    if inside == 0 || inside == 8 {
                        continue;
                    }
                    let cell = [key[0] * BLOCK + x, key[1] * BLOCK + y, key[2] * BLOCK + z];
                    let corner_at = |c: usize| -> [f32; 3] {
                        [(c & 1) as f32, ((c >> 1) & 1) as f32, (c >> 2) as f32]
                    };
                    let mut sum = [0.0f32; 3];
                    let mut count = 0.0f32;
                    for (a, b) in EDGES {
                        let (va, vb) = (corners[a], corners[b]);
                        if (va < 0.0) == (vb < 0.0) {
                            continue;
                        }
                        let t = va / (va - vb);
                        let pa = corner_at(a);
                        let pb = corner_at(b);
                        sum = add(sum, add(pa, scale(sub(pb, pa), t)));
                        count += 1.0;
                    }
                    let unit = scale(sum, 1.0 / count);
                    let position = add(grid.point(cell), scale(unit, voxel));
                    index.insert(cell, positions.len() as u32);
                    positions.push(position);
                    cells.push((*key, local));
                }
            }
        }
    }
    let mut quads: Vec<[u32; 4]> = Vec::new();
    for (key, local) in &cells {
        let values = &grid.blocks[key];
        let cell = [
            key[0] * BLOCK + local[0],
            key[1] * BLOCK + local[1],
            key[2] * BLOCK + local[2],
        ];
        let start = grid.value(values, *local);
        for axis in 0..3 {
            let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
            let mut end_local = *local;
            end_local[axis] += 1;
            let end = grid.value(values, end_local);
            if (start < 0.0) == (end < 0.0) {
                continue;
            }
            let step = |cell: [i32; 3], along: usize| {
                let mut out = cell;
                out[along] -= 1;
                out
            };
            let around = [cell, step(cell, b), step(step(cell, b), c), step(cell, c)];
            let found = around.map(|cell| index.get(&cell).copied());
            if let [Some(q0), Some(q1), Some(q2), Some(q3)] = found {
                if start < 0.0 {
                    quads.push([q0, q1, q2, q3]);
                } else {
                    quads.push([q0, q3, q2, q1]);
                }
            }
        }
    }
    let eps = voxel * 0.1;
    let mut anchors = positions.clone();
    for (p, anchor) in positions.iter_mut().zip(&anchors) {
        *p = project(field, *p, *anchor, voxel, eps, 3);
    }
    if relax > 0 {
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); positions.len()];
        for quad in &quads {
            for i in 0..4 {
                let (a, b) = (quad[i], quad[(i + 1) % 4]);
                neighbours[a as usize].push(b);
                neighbours[b as usize].push(a);
            }
        }
        for list in &mut neighbours {
            list.sort_unstable();
            list.dedup();
        }
        anchors.clone_from(&positions);
        for _ in 0..relax {
            let previous = positions.clone();
            for (i, list) in neighbours.iter().enumerate() {
                if list.is_empty() {
                    continue;
                }
                let mut sum = [0.0f32; 3];
                for &n in list {
                    sum = add(sum, previous[n as usize]);
                }
                let average = scale(sum, 1.0 / list.len() as f32);
                let moved = add(previous[i], scale(sub(average, previous[i]), 0.5));
                positions[i] = project(field, moved, anchors[i], voxel, eps, 1);
            }
        }
    }
    let normals = positions
        .iter()
        .map(|p| normalize(gradient(field, *p, eps)))
        .collect::<Vec<_>>();
    let mut indices = Vec::with_capacity(quads.len() * 6);
    for q in &quads {
        let p = q.map(|i| positions[i as usize]);
        if length(sub(p[0], p[2])) <= length(sub(p[1], p[3])) {
            indices.extend([q[0], q[1], q[2], q[0], q[2], q[3]]);
        } else {
            indices.extend([q[0], q[1], q[3], q[1], q[2], q[3]]);
        }
    }
    let uvs = planar_uvs(&positions);
    Mesh::new(positions, normals, Vec::new(), uvs, indices)
}

const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

fn project<F: Fn([f32; 3]) -> f32>(
    field: &F,
    mut p: [f32; 3],
    anchor: [f32; 3],
    voxel: f32,
    eps: f32,
    steps: u32,
) -> [f32; 3] {
    for _ in 0..steps {
        let d = field(p);
        let g = gradient(field, p, eps);
        let g2 = dot(g, g);
        if g2 <= 1e-12 {
            break;
        }
        p = sub(p, scale(g, d / g2));
        let offset = sub(p, anchor);
        let far = length(offset);
        if far > voxel {
            p = add(anchor, scale(offset, voxel / far));
        }
    }
    p
}

pub(crate) fn planar_uvs(positions: &[[f32; 3]]) -> Vec<[f32; 2]> {
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for p in positions {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-9);
    positions
        .iter()
        .map(|p| [(p[0] - lo[0]) / span, (hi[1] - p[1]) / span])
        .collect()
}
