use crate::frame::{Matrix, transform};

pub const TOTAL: u32 = 4096;
pub const Z_SLICES: u32 = 24;
pub const FIRST_SLICE_DEPTH: f32 = 5.0;
pub const LIGHT_WORDS: usize = 20;
const MARGIN: f32 = 1e-3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    pub centre: [f32; 3],
    pub radius: f32,
    pub cone: Option<Cone>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cone {
    pub direction: [f32; 3],
    pub outer: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Grid {
    pub dims: [u32; 3],
    pub factors: [f32; 4],
}

#[derive(Default)]
pub struct Assignment {
    pub grid: Grid,
    pub table: Vec<[u32; 2]>,
    pub indices: Vec<u32>,
    pairs: Vec<(u32, u32)>,
    boxes: Vec<[[f32; 3]; 2]>,
    boxes_key: Option<([u32; 3], [u32; 6], u32)>,
}

pub fn dims(aspect: f32) -> [u32; 3] {
    let per_layer = TOTAL as f32 / Z_SLICES as f32;
    let y = (per_layer / aspect.max(1e-3)).sqrt();
    let mut x = (y * aspect) as u32;
    let mut y = y as u32;
    if x == 0 {
        x = 1;
        y = per_layer as u32;
    }
    if y == 0 {
        x = per_layer as u32;
        y = 1;
    }
    [x, y, Z_SLICES]
}

fn perspective(projection: &Matrix) -> bool {
    projection[2][3] != 0.0
}

fn view_point(projection: &Matrix, ndc: [f32; 2], depth: f32) -> [f32; 3] {
    let z = -depth;
    let w = projection[2][3] * z + projection[3][3];
    let x = (ndc[0] * w - projection[2][0] * z - projection[3][0]) / projection[0][0];
    let y = (ndc[1] * w - projection[2][1] * z - projection[3][1]) / projection[1][1];
    [x, y, z]
}

impl Assignment {
    pub fn clusters(&self) -> usize {
        self.table.len()
    }

    pub fn build(&mut self, view: &Matrix, projection: &Matrix, lights: &[Sphere]) {
        self.table.clear();
        self.indices.clear();
        self.pairs.clear();
        if lights.is_empty() {
            self.grid = Grid::default();
            return;
        }
        let aspect = (projection[1][1] / projection[0][0]).abs();
        let dims = dims(aspect);
        let persp = perspective(projection);
        let spheres: Vec<Sphere> = lights
            .iter()
            .map(|light| {
                let c = transform(
                    *view,
                    [light.centre[0], light.centre[1], light.centre[2], 1.0],
                );
                Sphere {
                    centre: [c[0], c[1], c[2]],
                    radius: light.radius,
                    cone: light.cone.map(|cone| {
                        let d = transform(
                            *view,
                            [cone.direction[0], cone.direction[1], cone.direction[2], 0.0],
                        );
                        Cone {
                            direction: [d[0], d[1], d[2]],
                            ..cone
                        }
                    }),
                }
            })
            .collect();
        let near = if persp { FIRST_SLICE_DEPTH } else { 0.0 };
        let far = spheres
            .iter()
            .map(|sphere| -sphere.centre[2] + sphere.radius)
            .fold(0.0_f32, f32::max)
            .max(if persp { near * 2.0 } else { 1e-3 });
        let z = dims[2] as f32;
        let factors = if persp {
            let fz = (z - 1.0) / (far / near).ln();
            [fz, near.ln() * fz, 0.0, 0.0]
        } else {
            [z / far, 0.0, 1.0, 0.0]
        };
        self.grid = Grid { dims, factors };
        let bounds: Vec<f32> = (0..=dims[2])
            .map(|slice| self.slice_depth(slice, near, far))
            .collect();
        let key = (
            dims,
            [
                projection[0][0].to_bits(),
                projection[1][1].to_bits(),
                projection[2][0].to_bits(),
                projection[2][1].to_bits(),
                projection[3][0].to_bits(),
                projection[3][1].to_bits(),
            ],
            far.to_bits(),
        );
        if self.boxes_key != Some(key) {
            self.boxes = froxels(projection, dims, &bounds);
            self.boxes_key = Some(key);
        }
        for (index, sphere) in spheres.iter().enumerate() {
            let depth = -sphere.centre[2];
            if depth + sphere.radius <= 0.0 {
                continue;
            }
            let first = self.slice_of(depth - sphere.radius, near).min(dims[2] - 1);
            let last = self.slice_of(depth + sphere.radius, near).min(dims[2] - 1);
            let [x0, x1, y0, y1] = tiles(projection, sphere, dims, persp);
            for slice in first..=last {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let cluster = (y * dims[0] + x) * dims[2] + slice;
                        let aabb = self.boxes[cluster as usize];
                        if touches(sphere, aabb) {
                            self.pairs.push((cluster, index as u32));
                        }
                    }
                }
            }
        }
        let total = (dims[0] * dims[1] * dims[2]) as usize;
        self.table.resize(total, [0, 0]);
        for &(cluster, _) in &self.pairs {
            self.table[cluster as usize][1] += 1;
        }
        let mut offset = 0;
        for entry in &mut self.table {
            entry[0] = offset;
            offset += entry[1];
        }
        self.indices.resize(offset as usize, 0);
        let mut filled = vec![0u32; total];
        for &(cluster, light) in &self.pairs {
            let entry = self.table[cluster as usize];
            self.indices[(entry[0] + filled[cluster as usize]) as usize] = light;
            filled[cluster as usize] += 1;
        }
    }

    fn slice_of(&self, depth: f32, near: f32) -> u32 {
        let [fz, fw, ortho, _] = self.grid.factors;
        let slice = if ortho > 0.5 {
            depth * fz - fw
        } else if depth <= near {
            0.0
        } else {
            depth.ln() * fz - fw + 1.0
        };
        slice.max(0.0) as u32
    }

    fn slice_depth(&self, slice: u32, near: f32, far: f32) -> f32 {
        let z = self.grid.dims[2];
        if self.grid.factors[2] > 0.5 {
            return far * slice as f32 / z as f32;
        }
        if slice == 0 {
            return 0.0;
        }
        if slice >= z {
            return far;
        }
        near * (far / near).powf((slice - 1) as f32 / (z - 1) as f32)
    }
}

fn froxels(projection: &Matrix, dims: [u32; 3], bounds: &[f32]) -> Vec<[[f32; 3]; 2]> {
    let mut out = Vec::with_capacity((dims[0] * dims[1] * dims[2]) as usize);
    for y in 0..dims[1] {
        for x in 0..dims[0] {
            let nx = [
                x as f32 / dims[0] as f32 * 2.0 - 1.0,
                (x + 1) as f32 / dims[0] as f32 * 2.0 - 1.0,
            ];
            let ny = [
                y as f32 / dims[1] as f32 * 2.0 - 1.0,
                (y + 1) as f32 / dims[1] as f32 * 2.0 - 1.0,
            ];
            for slice in 0..dims[2] as usize {
                let mut lo = [f32::MAX; 3];
                let mut hi = [f32::MIN; 3];
                for depth in [bounds[slice], bounds[slice + 1]] {
                    for ndc_x in nx {
                        for ndc_y in ny {
                            let p = view_point(projection, [ndc_x, ndc_y], depth);
                            for k in 0..3 {
                                lo[k] = lo[k].min(p[k]);
                                hi[k] = hi[k].max(p[k]);
                            }
                        }
                    }
                }
                let pad = (0..3)
                    .map(|k| (hi[k] - lo[k]).abs())
                    .fold(0.0_f32, f32::max)
                    * MARGIN
                    + MARGIN;
                out.push([lo.map(|v| v - pad), hi.map(|v| v + pad)]);
            }
        }
    }
    out
}

fn tiles(projection: &Matrix, sphere: &Sphere, dims: [u32; 3], persp: bool) -> [u32; 4] {
    let full = [0, dims[0] - 1, 0, dims[1] - 1];
    let c = sphere.centre;
    let r = sphere.radius;
    if persp && -c[2] - r <= 1e-4 {
        return full;
    }
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for corner in 0..8 {
        let p = [
            c[0] + if corner & 1 == 0 { -r } else { r },
            c[1] + if corner & 2 == 0 { -r } else { r },
            c[2] + if corner & 4 == 0 { -r } else { r },
        ];
        let clip = transform(*projection, [p[0], p[1], p[2], 1.0]);
        if clip[3] <= 0.0 {
            return full;
        }
        for k in 0..2 {
            let ndc = clip[k] / clip[3];
            lo[k] = lo[k].min(ndc);
            hi[k] = hi[k].max(ndc);
        }
    }
    let cell = |ndc: f32, count: u32| -> u32 {
        (((ndc * 0.5 + 0.5) * count as f32).floor()).clamp(0.0, (count - 1) as f32) as u32
    };
    [
        cell(lo[0] - MARGIN, dims[0]),
        cell(hi[0] + MARGIN, dims[0]),
        cell(lo[1] - MARGIN, dims[1]),
        cell(hi[1] + MARGIN, dims[1]),
    ]
}

fn touches(sphere: &Sphere, aabb: [[f32; 3]; 2]) -> bool {
    let c = sphere.centre;
    let mut distance = 0.0;
    for k in 0..3 {
        let v = c[k].clamp(aabb[0][k], aabb[1][k]) - c[k];
        distance += v * v;
    }
    if distance > sphere.radius * sphere.radius {
        return false;
    }
    let Some(cone) = sphere.cone else {
        return true;
    };
    let centre = [0, 1, 2].map(|k| (aabb[0][k] + aabb[1][k]) * 0.5);
    let half = [0, 1, 2].map(|k| (aabb[1][k] - aabb[0][k]) * 0.5);
    let bound = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
    let v = [0, 1, 2].map(|k| centre[k] - c[k]);
    let length_squared = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    let along = v[0] * cone.direction[0] + v[1] * cone.direction[1] + v[2] * cone.direction[2];
    let (sin, cos) = cone.outer.sin_cos();
    let closest = cos * (length_squared - along * along).max(0.0).sqrt() - along * sin;
    let angle_cull = closest > bound;
    let front_cull = along > bound + sphere.radius;
    let back_cull = along < -bound;
    !(angle_cull || front_cull || back_cull)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::multiply;

    fn projection(aspect: f32) -> Matrix {
        let near = 0.1;
        let far = 200.0;
        let f = 1.0 / (40.0_f32.to_radians() * 0.5).tan();
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ]
    }

    fn view() -> Matrix {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.3, -1.2, -0.5, 1.0],
        ]
    }

    fn cluster_of(
        assignment: &Assignment,
        view: &Matrix,
        projection: &Matrix,
        world: [f32; 3],
    ) -> usize {
        let grid = assignment.grid;
        let clip = transform(
            multiply(*projection, *view),
            [world[0], world[1], world[2], 1.0],
        );
        let ndc = [clip[0] / clip[3], clip[1] / clip[3]];
        let x = (((ndc[0] * 0.5 + 0.5) * grid.dims[0] as f32).floor())
            .clamp(0.0, (grid.dims[0] - 1) as f32) as u32;
        let y = (((ndc[1] * 0.5 + 0.5) * grid.dims[1] as f32).floor())
            .clamp(0.0, (grid.dims[1] - 1) as f32) as u32;
        let depth = -transform(*view, [world[0], world[1], world[2], 1.0])[2];
        let slice = ((depth.ln() * grid.factors[0] - grid.factors[1] + 1.0).max(0.0) as u32)
            .min(grid.dims[2] - 1);
        ((y * grid.dims[0] + x) * grid.dims[2] + slice) as usize
    }

    #[test]
    fn portrait_4k_takes_bevys_fixed_z_grid() {
        assert_eq!(dims(2160.0 / 3840.0), [9, 17, 24]);
        assert_eq!(dims(16.0 / 9.0), [17, 9, 24]);
    }

    #[test]
    fn every_lit_point_finds_its_light_in_its_cluster() {
        let projection = projection(0.5625);
        let view = view();
        let mut seed = 9u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        let lights: Vec<Sphere> = (0..96)
            .map(|index| Sphere {
                centre: [next() * 30.0 - 15.0, next() * 6.0, -next() * 80.0],
                radius: 1.0 + next() * 8.0,
                cone: (index % 3 == 0).then(|| Cone {
                    direction: {
                        let d = [next() - 0.5, -1.0, next() - 0.5];
                        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                        d.map(|v| v / l)
                    },
                    outer: (10.0 + next() * 50.0).to_radians(),
                }),
            })
            .collect();
        let mut assignment = Assignment::default();
        assignment.build(&view, &projection, &lights);
        let mut checked = 0;
        for _ in 0..20000 {
            let world = [next() * 40.0 - 20.0, next() * 8.0 - 1.0, -next() * 90.0];
            let at = cluster_of(&assignment, &view, &projection, world);
            let clip = transform(
                multiply(projection, view),
                [world[0], world[1], world[2], 1.0],
            );
            if clip[3] <= 0.0 || clip[0].abs() > clip[3] || clip[1].abs() > clip[3] {
                continue;
            }
            let [start, count] = assignment.table[at];
            let listed = &assignment.indices[start as usize..(start + count) as usize];
            for (index, light) in lights.iter().enumerate() {
                let to = [0, 1, 2].map(|k| world[k] - light.centre[k]);
                let distance = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
                if distance >= light.radius {
                    continue;
                }
                if let Some(cone) = light.cone {
                    let along = (to[0] * cone.direction[0]
                        + to[1] * cone.direction[1]
                        + to[2] * cone.direction[2])
                        / distance.max(1e-6);
                    if along <= cone.outer.cos() {
                        continue;
                    }
                }
                assert!(
                    listed.contains(&(index as u32)),
                    "light {index} missing at {world:?}"
                );
                checked += 1;
            }
        }
        assert!(checked > 1000, "{checked}");
        let pairs: usize = assignment.table.iter().map(|entry| entry[1] as usize).sum();
        assert!(pairs < assignment.clusters() * lights.len() / 4, "{pairs}");
    }
}
