use crate::math::{add, cross, dot, normalize, scale, sub};

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    pub fn empty() -> Self {
        Self {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }

    pub fn point(p: [f32; 3]) -> Self {
        Self { min: p, max: p }
    }

    pub fn include(&mut self, p: [f32; 3]) {
        for ((low, high), value) in self.min.iter_mut().zip(self.max.iter_mut()).zip(p) {
            *low = low.min(value);
            *high = high.max(value);
        }
    }

    pub fn from_points(points: &[[f32; 3]]) -> Self {
        let mut bounds = Self::empty();
        for p in points {
            bounds.include(*p);
        }
        bounds
    }

    pub fn center(self) -> [f32; 3] {
        if !self.min[0].is_finite() {
            return [0.0; 3];
        }
        scale(add(self.min, self.max), 0.5)
    }

    pub fn radius(self) -> f32 {
        if !self.min[0].is_finite() {
            return 0.0;
        }
        let half = scale(sub(self.max, self.min), 0.5);
        crate::math::length(half)
    }

    pub fn extent(self) -> f32 {
        if !self.min[0].is_finite() {
            return 0.0;
        }
        let span = sub(self.max, self.min);
        span[0].max(span[1]).max(span[2])
    }
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub bounds: Bounds,
}

impl Mesh {
    pub fn new(
        positions: Vec<[f32; 3]>,
        mut normals: Vec<[f32; 3]>,
        mut tangents: Vec<[f32; 4]>,
        mut uvs: Vec<[f32; 2]>,
        indices: Vec<u32>,
    ) -> Self {
        let count = positions.len();
        normals.resize(count, [0.0, 0.0, 1.0]);
        uvs.resize(count, [0.0, 0.0]);
        let bounds = Bounds::from_points(&positions);
        let indices = indices
            .chunks_exact(3)
            .filter(|tri| tri.iter().all(|index| (*index as usize) < count))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        if tangents.len() != count {
            tangents = tangents_from(&positions, &normals, &uvs, &indices);
        }
        Self {
            positions,
            normals,
            tangents,
            uvs,
            indices,
            bounds,
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, self.positions.len() as u32);
        push_u32(&mut out, self.indices.len() as u32);
        for p in &self.positions {
            push_f32s(&mut out, p);
        }
        for n in &self.normals {
            push_f32s(&mut out, n);
        }
        for t in &self.tangents {
            push_f32s(&mut out, t);
        }
        for uv in &self.uvs {
            push_f32s(&mut out, uv);
        }
        for index in &self.indices {
            push_u32(&mut out, *index);
        }
        out
    }
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f32s(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

pub(crate) struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Builder {
    pub(crate) fn new() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        }
    }

    pub(crate) fn vertex(&mut self, position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> u32 {
        let index = self.positions.len() as u32;
        self.positions.push(position);
        self.normals.push(normalize(normal));
        self.uvs.push(uv);
        index
    }

    pub(crate) fn tri(&mut self, a: u32, b: u32, c: u32) {
        if a == b || b == c || c == a {
            return;
        }
        let pa = self.positions[a as usize];
        let pb = self.positions[b as usize];
        let pc = self.positions[c as usize];
        let face = cross(sub(pb, pa), sub(pc, pa));
        if crate::math::length(face) <= 1e-12 {
            return;
        }
        let average = normalize(add(
            add(self.normals[a as usize], self.normals[b as usize]),
            self.normals[c as usize],
        ));
        if dot(face, average) < 0.0 {
            self.indices.extend([a, c, b]);
        } else {
            self.indices.extend([a, b, c]);
        }
    }

    pub(crate) fn build(self) -> Mesh {
        Mesh::new(
            self.positions,
            self.normals,
            Vec::new(),
            self.uvs,
            self.indices,
        )
    }
}

pub(crate) fn weld(mesh: Mesh, quantum: f32) -> Mesh {
    let quantum = quantum.max(1e-7);
    let mut key_of: Vec<(i32, i32, i32)> = Vec::with_capacity(mesh.positions.len());
    let mut map: std::collections::BTreeMap<(i32, i32, i32), u32> =
        std::collections::BTreeMap::new();
    let mut remap = vec![0u32; mesh.positions.len()];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for (index, position) in mesh.positions.iter().enumerate() {
        let key = (
            (position[0] / quantum).round() as i32,
            (position[1] / quantum).round() as i32,
            (position[2] / quantum).round() as i32,
        );
        key_of.push(key);
        if let Some(kept) = map.get(&key) {
            remap[index] = *kept;
        } else {
            let kept = positions.len() as u32;
            map.insert(key, kept);
            remap[index] = kept;
            positions.push(*position);
            normals.push(mesh.normals[index]);
            uvs.push(mesh.uvs[index]);
        }
    }
    let _ = key_of;
    let indices = mesh
        .indices
        .chunks_exact(3)
        .filter_map(|tri| {
            let a = remap[tri[0] as usize];
            let b = remap[tri[1] as usize];
            let c = remap[tri[2] as usize];
            (a != b && b != c && c != a).then_some([a, b, c])
        })
        .flatten()
        .collect::<Vec<_>>();
    Mesh::new(positions, normals, Vec::new(), uvs, indices)
}

fn tangents_from(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let mut along_u = vec![[0.0; 3]; positions.len()];
    let mut along_v = vec![[0.0; 3]; positions.len()];
    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let (p0, p1, p2) = (positions[i0], positions[i1], positions[i2]);
        let (w0, w1, w2) = (uvs[i0], uvs[i1], uvs[i2]);
        let edge1 = sub(p1, p0);
        let edge2 = sub(p2, p0);
        let du1 = w1[0] - w0[0];
        let du2 = w2[0] - w0[0];
        let dv1 = w1[1] - w0[1];
        let dv2 = w2[1] - w0[1];
        let denom = du1 * dv2 - du2 * dv1;
        let factor = if denom.abs() < 1e-12 {
            0.0
        } else {
            1.0 / denom
        };
        let sdir = scale(
            [
                dv2 * edge1[0] - dv1 * edge2[0],
                dv2 * edge1[1] - dv1 * edge2[1],
                dv2 * edge1[2] - dv1 * edge2[2],
            ],
            factor,
        );
        let tdir = scale(
            [
                du1 * edge2[0] - du2 * edge1[0],
                du1 * edge2[1] - du2 * edge1[1],
                du1 * edge2[2] - du2 * edge1[2],
            ],
            factor,
        );
        for index in [i0, i1, i2] {
            along_u[index] = add(along_u[index], sdir);
            along_v[index] = add(along_v[index], tdir);
        }
    }
    along_u
        .iter()
        .zip(along_v.iter())
        .zip(normals.iter())
        .map(|((sdir, tdir), normal)| {
            let normal = normalize(*normal);
            let mut tangent = sub(*sdir, scale(normal, dot(normal, *sdir)));
            if crate::math::length(tangent) <= 1e-8 {
                let axis = if normal[0].abs() < 0.9 {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                tangent = sub(axis, scale(normal, dot(normal, axis)));
            }
            let tangent = normalize(tangent);
            let handed = if dot(cross(normal, tangent), *tdir) < 0.0 {
                -1.0
            } else {
                1.0
            };
            [tangent[0], tangent[1], tangent[2], handed]
        })
        .collect()
}
