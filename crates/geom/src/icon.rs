use crate::field::{Surface, smooth_min};
use crate::math::{add, length, normalize};
use crate::mesh::Mesh;
use crate::shapes::{Shape, round_cone};

pub const VOXEL: f32 = 0.01;
pub const RELAX: u32 = 8;
pub const BLEND_VOXELS: f32 = 2.0;
pub const CROSS_LIMIT: f32 = 900.0;
pub const ERROR: f32 = 0.001;

#[derive(Clone, Debug, PartialEq)]
pub enum PartShape {
    Ball {
        at: [f32; 2],
        z: f32,
        r: f32,
    },
    Drop {
        at: [f32; 2],
        z: f32,
        r: f32,
        squash: f32,
        stretch: f32,
    },
    Capsule {
        from: [f32; 2],
        to: [f32; 2],
        z: f32,
        r: f32,
    },
    Tube {
        points: Vec<[f32; 2]>,
        z: f32,
        r: f32,
    },
    Torus {
        at: [f32; 2],
        z: f32,
        major: f32,
        minor: f32,
    },
    Box {
        at: [f32; 2],
        z: f32,
        half: [f32; 3],
        round: f32,
        angle: f32,
    },
}

impl PartShape {
    pub fn ball() -> Self {
        PartShape::Ball {
            at: [0.0, 0.0],
            z: 0.0,
            r: 0.3,
        }
    }

    pub fn drop() -> Self {
        PartShape::Drop {
            at: [0.0, 0.0],
            z: 0.0,
            r: 1.0,
            squash: 0.45,
            stretch: 1.0,
        }
    }

    pub fn capsule(from: [f32; 2], to: [f32; 2]) -> Self {
        PartShape::Capsule {
            from,
            to,
            z: 0.0,
            r: 0.08,
        }
    }

    pub fn tube(points: Vec<[f32; 2]>) -> Self {
        PartShape::Tube {
            points,
            z: 0.0,
            r: 0.08,
        }
    }

    pub fn torus() -> Self {
        PartShape::Torus {
            at: [0.0, 0.0],
            z: 0.0,
            major: 0.6,
            minor: 0.1,
        }
    }

    pub fn cube() -> Self {
        PartShape::Box {
            at: [0.0, 0.0],
            z: 0.0,
            half: [0.3, 0.3, 0.1],
            round: 0.05,
            angle: 0.0,
        }
    }

    pub fn objects(&self) -> usize {
        match self {
            PartShape::Capsule { .. } | PartShape::Tube { .. } => 3,
            _ => 1,
        }
    }

    pub fn distance(&self, p: [f32; 3]) -> f32 {
        match self {
            PartShape::Ball { at, z, r } => length([p[0] - at[0], p[1] - at[1], p[2] - z]) - r,
            PartShape::Drop {
                at,
                z,
                r,
                squash,
                stretch,
            } => {
                let radii = [*r, r * stretch, r * squash];
                let q = [p[0] - at[0], p[1] - at[1], p[2] - z];
                let k = length([q[0] / radii[0], q[1] / radii[1], q[2] / radii[2]]);
                (k - 1.0) * radii[0].min(radii[1]).min(radii[2])
            }
            PartShape::Capsule { from, to, z, r } => {
                let a = [from[0], from[1], *z];
                let b = [to[0], to[1], *z];
                round_cone(p, a, b, *r, *r)
            }
            PartShape::Tube { points, z, r } => {
                if points.len() == 1 {
                    return length([p[0] - points[0][0], p[1] - points[0][1], p[2] - z]) - r;
                }
                points
                    .windows(2)
                    .map(|w| segment_distance(p, [w[0][0], w[0][1], *z], [w[1][0], w[1][1], *z]))
                    .fold(f32::INFINITY, f32::min)
                    - r
            }
            PartShape::Torus {
                at,
                z,
                major,
                minor,
            } => {
                let q = [p[0] - at[0], p[1] - at[1], p[2] - z];
                let ring = (q[0] * q[0] + q[1] * q[1]).sqrt() - major;
                (ring * ring + q[2] * q[2]).sqrt() - minor
            }
            PartShape::Box {
                at,
                z,
                half,
                round,
                angle,
            } => {
                let (s, c) = angle.to_radians().sin_cos();
                let d = [p[0] - at[0], p[1] - at[1], p[2] - z];
                let local = [c * d[0] + s * d[1], -s * d[0] + c * d[1], d[2]];
                let radius = box_round(*half, *round);
                let q = [
                    local[0].abs() - half[0] + radius,
                    local[1].abs() - half[1] + radius,
                    local[2].abs() - half[2] + radius,
                ];
                length([q[0].max(0.0), q[1].max(0.0), q[2].max(0.0)])
                    + q[0].max(q[1]).max(q[2]).min(0.0)
                    - radius
            }
        }
    }

    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let around = |at: [f32; 2], z: f32, e: [f32; 3]| {
            (
                [at[0] - e[0], at[1] - e[1], z - e[2]],
                [at[0] + e[0], at[1] + e[1], z + e[2]],
            )
        };
        match self {
            PartShape::Ball { at, z, r } => around(*at, *z, [*r; 3]),
            PartShape::Drop {
                at,
                z,
                r,
                squash,
                stretch,
            } => around(*at, *z, [*r, r * stretch, r * squash]),
            PartShape::Capsule { from, to, z, r } => line_bounds(&[*from, *to], *z, *r),
            PartShape::Tube { points, z, r } => line_bounds(points, *z, *r),
            PartShape::Torus {
                at,
                z,
                major,
                minor,
            } => around(*at, *z, [major + minor, major + minor, *minor]),
            PartShape::Box {
                at, z, half, angle, ..
            } => {
                let (s, c) = angle.to_radians().sin_cos();
                let ex = c.abs() * half[0] + s.abs() * half[1];
                let ey = s.abs() * half[0] + c.abs() * half[1];
                around(*at, *z, [ex, ey, half[2]])
            }
        }
    }

    pub fn mesh(&self, error: f32) -> Mesh {
        match self {
            PartShape::Ball { at, z, r } => {
                let base = Shape::Ellipsoid {
                    radius: *r,
                    squash: 1.0,
                }
                .mesh(error);
                place(base, [at[0], at[1], *z], 0.0, 1.0)
            }
            PartShape::Drop {
                at,
                z,
                r,
                squash,
                stretch,
            } => {
                let base = Shape::Ellipsoid {
                    radius: *r,
                    squash: *squash,
                }
                .mesh(error);
                place(base, [at[0], at[1], *z], 0.0, *stretch)
            }
            PartShape::Capsule { from, to, z, r } => Shape::Capsule {
                a: [from[0], from[1], *z],
                b: [to[0], to[1], *z],
                radius: *r,
            }
            .mesh(error),
            PartShape::Tube { points, z, r } => {
                let pieces = if points.len() == 1 {
                    vec![
                        PartShape::Ball {
                            at: points[0],
                            z: *z,
                            r: *r,
                        }
                        .mesh(error),
                    ]
                } else {
                    points
                        .windows(2)
                        .map(|w| {
                            Shape::Capsule {
                                a: [w[0][0], w[0][1], *z],
                                b: [w[1][0], w[1][1], *z],
                                radius: *r,
                            }
                            .mesh(error)
                        })
                        .collect()
                };
                join(&pieces)
            }
            PartShape::Torus {
                at,
                z,
                major,
                minor,
            } => place(
                Shape::Torus {
                    major: *major,
                    minor: *minor,
                }
                .mesh(error),
                [at[0], at[1], *z],
                0.0,
                1.0,
            ),
            PartShape::Box {
                at,
                z,
                half,
                round,
                angle,
            } => place(
                Shape::RoundBox {
                    half: *half,
                    radius: box_round(*half, *round),
                }
                .mesh(error),
                [at[0], at[1], *z],
                *angle,
                1.0,
            ),
        }
    }
}

fn box_round(half: [f32; 3], round: f32) -> f32 {
    round.min(half[0].min(half[1]).min(half[2]) * 0.9).max(0.0)
}

fn line_bounds(points: &[[f32; 2]], z: f32, r: f32) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY, f32::INFINITY, z - r];
    let mut hi = [f32::NEG_INFINITY, f32::NEG_INFINITY, z + r];
    for p in points {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i] - r);
            hi[i] = hi[i].max(p[i] + r);
        }
    }
    (lo, hi)
}

fn segment_distance(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let t = if l2 <= 1e-12 {
        0.0
    } else {
        ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / l2).clamp(0.0, 1.0)
    };
    length([ap[0] - ab[0] * t, ap[1] - ab[1] * t, ap[2] - ab[2] * t])
}

fn place(mesh: Mesh, at: [f32; 3], angle: f32, stretch: f32) -> Mesh {
    let (s, c) = angle.to_radians().sin_cos();
    let positions = mesh
        .positions
        .iter()
        .map(|p| {
            let y = p[1] * stretch;
            add([c * p[0] - s * y, s * p[0] + c * y, p[2]], at)
        })
        .collect();
    let normals = mesh
        .normals
        .iter()
        .map(|n| {
            let y = n[1] / stretch;
            normalize([c * n[0] - s * y, s * n[0] + c * y, n[2]])
        })
        .collect();
    Mesh::new(positions, normals, Vec::new(), mesh.uvs, mesh.indices)
}

pub(crate) fn join(meshes: &[Mesh]) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tangents = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for mesh in meshes {
        let base = positions.len() as u32;
        positions.extend_from_slice(&mesh.positions);
        normals.extend_from_slice(&mesh.normals);
        tangents.extend_from_slice(&mesh.tangents);
        uvs.extend_from_slice(&mesh.uvs);
        indices.extend(mesh.indices.iter().map(|i| i + base));
    }
    Mesh::new(positions, normals, tangents, uvs, indices)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub shape: PartShape,
    pub material: Option<String>,
}

impl Part {
    pub fn new(shape: PartShape) -> Self {
        Self {
            shape,
            material: None,
        }
    }

    pub fn with_material(mut self, material: &str) -> Self {
        self.material = Some(material.to_string());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub parts: Vec<Part>,
    pub fuse: bool,
    pub voxel: f32,
    pub blend: Option<f32>,
    pub relax: u32,
    pub error: f32,
}

impl Icon {
    pub fn new(parts: Vec<Part>) -> Self {
        Self {
            parts,
            fuse: true,
            voxel: VOXEL,
            blend: None,
            relax: RELAX,
            error: ERROR,
        }
    }

    pub fn build(&self) -> Result<Vec<Piece>, String> {
        for (i, part) in self.parts.iter().enumerate() {
            check(i, &part.shape)?;
        }
        if !(self.voxel > 0.0 && self.voxel.is_finite()) {
            return Err(format!("icon: voxel must be above 0, got {}", self.voxel));
        }
        let mut groups: Vec<(Option<String>, Vec<usize>)> = Vec::new();
        for (i, part) in self.parts.iter().enumerate() {
            match groups.iter_mut().find(|(name, _)| *name == part.material) {
                Some((_, members)) => members.push(i),
                None => groups.push((part.material.clone(), vec![i])),
            }
        }
        let mut pieces = Vec::new();
        for (material, members) in groups {
            let objects: usize = members.iter().map(|&i| self.parts[i].shape.objects()).sum();
            if self.fuse && objects > 1 {
                let shapes = members
                    .iter()
                    .map(|&i| &self.parts[i].shape)
                    .collect::<Vec<_>>();
                pieces.push(Piece {
                    material,
                    parts: members,
                    mesh: self.fused(&shapes),
                });
            } else {
                for i in members {
                    pieces.push(Piece {
                        material: material.clone(),
                        parts: vec![i],
                        mesh: self.parts[i].shape.mesh(self.error),
                    });
                }
            }
        }
        Ok(pieces)
    }

    pub fn voxel_for(&self, shapes: &[&PartShape]) -> f32 {
        let (lo, hi) = union_bounds(shapes);
        let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]);
        self.voxel.max(span / CROSS_LIMIT)
    }

    fn fused(&self, shapes: &[&PartShape]) -> Mesh {
        let voxel = self.voxel_for(shapes);
        let blend = self.blend.unwrap_or(BLEND_VOXELS * voxel).max(0.0);
        let (lo, hi) = union_bounds(shapes);
        let field = |p: [f32; 3]| {
            shapes
                .iter()
                .map(|s| s.distance(p))
                .reduce(|a, b| smooth_min(a, b, blend))
                .unwrap_or(f32::INFINITY)
        };
        Surface::new(voxel, self.relax).mesh(&field, sub3(lo, blend), add3(hi, blend))
    }
}

fn add3(p: [f32; 3], d: f32) -> [f32; 3] {
    [p[0] + d, p[1] + d, p[2] + d]
}

fn sub3(p: [f32; 3], d: f32) -> [f32; 3] {
    [p[0] - d, p[1] - d, p[2] - d]
}

fn union_bounds(shapes: &[&PartShape]) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for shape in shapes {
        let (a, b) = shape.bounds();
        for i in 0..3 {
            lo[i] = lo[i].min(a[i]);
            hi[i] = hi[i].max(b[i]);
        }
    }
    (lo, hi)
}

fn check(index: usize, shape: &PartShape) -> Result<(), String> {
    let positive = |name: &str, v: f32| {
        if v > 0.0 && v.is_finite() {
            Ok(())
        } else {
            Err(format!(
                "icon part {index}: {name} must be above 0, got {v}"
            ))
        }
    };
    match shape {
        PartShape::Ball { r, .. } => positive("r", *r),
        PartShape::Drop {
            r, squash, stretch, ..
        } => {
            positive("r", *r)?;
            positive("squash", *squash)?;
            positive("stretch", *stretch)
        }
        PartShape::Capsule { r, .. } => positive("r", *r),
        PartShape::Tube { points, r, .. } => {
            if points.is_empty() {
                return Err(format!("icon part {index}: a tube needs points"));
            }
            positive("r", *r)
        }
        PartShape::Torus { major, minor, .. } => {
            positive("major", *major)?;
            positive("minor", *minor)
        }
        PartShape::Box { half, .. } => {
            for v in half {
                positive("half", *v)?;
            }
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
pub struct Piece {
    pub material: Option<String>,
    pub parts: Vec<usize>,
    pub mesh: Mesh,
}
