use crate::frame::{Batch, Matrix};
use std::ops::Range;

pub const UNBOUNDED: [f32; 4] = [0.0, 0.0, 0.0, f32::INFINITY];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub batch: usize,
    pub range: Range<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassCounts {
    pub draws: u32,
    pub instances: u32,
    pub triangles: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CullStats {
    pub instances: u32,
    pub camera: PassCounts,
    pub occlusion_tested: bool,
    pub cascades: Vec<PassCounts>,
    pub light_faces: PassCounts,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    planes: [[f64; 4]; 6],
    count: usize,
}

fn row(matrix: Matrix, index: usize) -> [f64; 4] {
    std::array::from_fn(|column| f64::from(matrix[column][index]))
}

impl Frustum {
    pub fn of(view_projection: Matrix, near: bool, far: bool) -> Self {
        let [x, y, z, w] = [0, 1, 2, 3].map(|index| row(view_projection, index));
        let add = |a: [f64; 4], b: [f64; 4], sign: f64| -> [f64; 4] {
            std::array::from_fn(|k| a[k] + sign * b[k])
        };
        let mut candidates = vec![
            add(w, x, 1.0),
            add(w, x, -1.0),
            add(w, y, 1.0),
            add(w, y, -1.0),
        ];
        if near {
            candidates.push(z);
        }
        if far {
            candidates.push(add(w, z, -1.0));
        }
        let mut planes = [[0.0; 4]; 6];
        let mut count = 0;
        for plane in candidates {
            let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            if !length.is_finite() || length < 1e-12 || plane.iter().any(|v| !v.is_finite()) {
                continue;
            }
            planes[count] = plane.map(|value| value / length);
            count += 1;
        }
        Self { planes, count }
    }

    pub fn camera(view_projection: Matrix) -> Self {
        Self::of(view_projection, true, true)
    }

    pub fn caster(view_projection: Matrix) -> Self {
        Self::of(view_projection, false, false)
    }

    pub fn sees(&self, sphere: [f32; 4]) -> bool {
        let radius = f64::from(sphere[3]);
        if !radius.is_finite() || sphere[..3].iter().any(|value| !value.is_finite()) {
            return true;
        }
        let margin = radius * 1.0001 + 1e-4;
        self.planes[..self.count].iter().all(|plane| {
            plane[0] * f64::from(sphere[0])
                + plane[1] * f64::from(sphere[1])
                + plane[2] * f64::from(sphere[2])
                + plane[3]
                >= -margin
        })
    }
}

pub fn local_sphere(positions: &[[f32; 3]]) -> [f32; 4] {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for position in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
    }
    if positions.is_empty() || min.iter().chain(&max).any(|value| !value.is_finite()) {
        return UNBOUNDED;
    }
    let center: [f64; 3] =
        std::array::from_fn(|axis| (f64::from(min[axis]) + f64::from(max[axis])) * 0.5);
    let radius = positions
        .iter()
        .map(|position| {
            (0..3)
                .map(|axis| (f64::from(position[axis]) - center[axis]).powi(2))
                .sum::<f64>()
        })
        .fold(0.0, f64::max)
        .sqrt();
    [
        center[0] as f32,
        center[1] as f32,
        center[2] as f32,
        (radius * 1.0001 + 1e-6) as f32,
    ]
}

fn largest_stretch(model: Matrix) -> f64 {
    let a: [[f64; 3]; 3] =
        std::array::from_fn(|column| std::array::from_fn(|r| f64::from(model[column][r])));
    let g: [[f64; 3]; 3] =
        std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * a[j][k]).sum()));
    let off = g[0][1] * g[0][1] + g[0][2] * g[0][2] + g[1][2] * g[1][2];
    let largest = if off < 1e-30 {
        g[0][0].max(g[1][1]).max(g[2][2])
    } else {
        let q = (g[0][0] + g[1][1] + g[2][2]) / 3.0;
        let p =
            (((g[0][0] - q).powi(2) + (g[1][1] - q).powi(2) + (g[2][2] - q).powi(2) + 2.0 * off)
                / 6.0)
                .sqrt();
        let b: [[f64; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| (g[i][j] - if i == j { q } else { 0.0 }) / p)
        });
        let det = b[0][0] * (b[1][1] * b[2][2] - b[1][2] * b[2][1])
            - b[0][1] * (b[1][0] * b[2][2] - b[1][2] * b[2][0])
            + b[0][2] * (b[1][0] * b[2][1] - b[1][1] * b[2][0]);
        let phi = (det * 0.5).clamp(-1.0, 1.0).acos() / 3.0;
        q + 2.0 * p * phi.cos()
    };
    let frobenius = g[0][0] + g[1][1] + g[2][2];
    largest.max(0.0).min(frobenius).sqrt()
}

pub fn world_sphere(local: [f32; 4], model: Matrix) -> [f32; 4] {
    if !local[3].is_finite()
        || model[0][3] != 0.0
        || model[1][3] != 0.0
        || model[2][3] != 0.0
        || model[3][3] != 1.0
    {
        return UNBOUNDED;
    }
    let center: [f64; 3] = std::array::from_fn(|r| {
        (0..3)
            .map(|column| f64::from(model[column][r]) * f64::from(local[column]))
            .sum::<f64>()
            + f64::from(model[3][r])
    });
    let radius = f64::from(local[3]) * largest_stretch(model) * 1.0001 + 1e-6;
    let sphere = [
        center[0] as f32,
        center[1] as f32,
        center[2] as f32,
        radius as f32,
    ];
    if sphere.iter().all(|value| value.is_finite()) {
        sphere
    } else {
        UNBOUNDED
    }
}

pub const FADE_BAND: f32 = 0.1;

pub fn distance_fade(center: [f32; 3], eye: [f32; 3], max_distance: f32) -> f32 {
    if !max_distance.is_finite() {
        return 1.0;
    }
    let distance = (0..3)
        .map(|axis| (center[axis] - eye[axis]).powi(2))
        .sum::<f32>()
        .sqrt();
    let start = max_distance * (1.0 - FADE_BAND);
    if distance <= start {
        1.0
    } else if distance >= max_distance || !distance.is_finite() {
        0.0
    } else {
        let t = (max_distance - distance) / (max_distance - start);
        t * t * (3.0 - 2.0 * t)
    }
}

pub fn runs<'a>(
    batches: impl Iterator<Item = (usize, &'a Batch)>,
    sees: impl Fn(usize) -> bool,
    out: &mut Vec<Run>,
) {
    for (index, batch) in batches {
        let mut start = None;
        for instance in batch.range.clone() {
            match (sees(instance as usize), start) {
                (true, None) => start = Some(instance),
                (false, Some(from)) => {
                    out.push(Run {
                        batch: index,
                        range: from..instance,
                    });
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(from) = start {
            out.push(Run {
                batch: index,
                range: from..batch.range.end,
            });
        }
    }
}

pub fn all_runs<'a>(batches: impl Iterator<Item = (usize, &'a Batch)>) -> Vec<Run> {
    batches
        .map(|(index, batch)| Run {
            batch: index,
            range: batch.range.clone(),
        })
        .collect()
}

pub fn counts(runs: &[Run], triangles: impl Fn(usize) -> u64) -> PassCounts {
    runs.iter().fold(PassCounts::default(), |mut counts, run| {
        let instances = run.range.end - run.range.start;
        counts.draws += 1;
        counts.instances += instances;
        counts.triangles += u64::from(instances) * triangles(run.batch);
        counts
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{MeshHandle, multiply};

    fn identity() -> Matrix {
        let mut m = [[0.0; 4]; 4];
        for (i, column) in m.iter_mut().enumerate() {
            column[i] = 1.0;
        }
        m
    }

    fn perspective(aspect: f32, near: f32, far: f32) -> Matrix {
        let f = 1.0 / (0.5f32).tan();
        let range = far / (near - far);
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, range, -1.0],
            [0.0, 0.0, range * near, 0.0],
        ]
    }

    fn batch(range: Range<u32>) -> Batch {
        Batch {
            mesh: MeshHandle(0),
            two_sided: false,
            moving: false,
            clipped: false,
            cut: false,
            shadow_only: false,
            casts_shadow: true,
            range,
            crowd: None,
        }
    }

    #[test]
    fn a_camera_sees_what_is_in_front_and_not_what_is_behind_or_aside() {
        let frustum = Frustum::camera(multiply(perspective(1.0, 0.1, 100.0), identity()));
        assert!(frustum.sees([0.0, 0.0, -5.0, 1.0]));
        assert!(!frustum.sees([0.0, 0.0, 5.0, 1.0]));
        assert!(!frustum.sees([50.0, 0.0, -5.0, 1.0]));
        assert!(!frustum.sees([0.0, 0.0, -500.0, 1.0]));
        assert!(frustum.sees([0.0, 0.0, 0.5, 1.0]));
        assert!(frustum.sees([3.2, 0.0, -5.0, 1.0]));
        assert!(frustum.sees(UNBOUNDED));
    }

    #[test]
    fn a_caster_frustum_keeps_what_lies_toward_the_light() {
        let ortho = [
            [0.1, 0.0, 0.0, 0.0],
            [0.0, 0.1, 0.0, 0.0],
            [0.0, 0.0, 0.01, 0.0],
            [0.0, 0.0, 0.5, 1.0],
        ];
        let caster = Frustum::caster(ortho);
        assert!(caster.sees([0.0, 0.0, -1000.0, 1.0]));
        assert!(caster.sees([0.0, 0.0, 1000.0, 1.0]));
        assert!(!caster.sees([20.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_world_sphere_bounds_every_transformed_vertex() {
        let positions = [
            [-1.0, 0.0, 0.0],
            [1.0, 2.0, 0.5],
            [0.3, -0.4, -2.0],
            [0.0, 1.0, 1.0],
        ];
        let local = local_sphere(&positions);
        let mut model = identity();
        model[0] = [2.0, 0.5, 0.0, 0.0];
        model[1] = [1.0, 1.0, 0.3, 0.0];
        model[2] = [0.0, -0.7, 3.0, 0.0];
        model[3] = [10.0, -4.0, 2.0, 1.0];
        let sphere = world_sphere(local, model);
        for position in positions {
            let world: [f32; 3] = std::array::from_fn(|r| {
                (0..3).map(|c| model[c][r] * position[c]).sum::<f32>() + model[3][r]
            });
            let distance = (0..3)
                .map(|k| (world[k] - sphere[k]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!(distance <= sphere[3], "{distance} > {}", sphere[3]);
        }
        let mut rotated = identity();
        rotated[0] = [0.0, 1.0, 0.0, 0.0];
        rotated[1] = [-1.0, 0.0, 0.0, 0.0];
        let turned = world_sphere([0.0, 0.0, 0.0, 1.0], rotated);
        assert!((turned[3] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_far_object_fades_over_the_last_tenth_of_its_distance_and_then_goes() {
        let eye = [0.0; 3];
        assert_eq!(distance_fade([5.0, 0.0, 0.0], eye, f32::INFINITY), 1.0);
        assert_eq!(distance_fade([89.0, 0.0, 0.0], eye, 100.0), 1.0);
        assert_eq!(distance_fade([100.0, 0.0, 0.0], eye, 100.0), 0.0);
        let mid = distance_fade([95.0, 0.0, 0.0], eye, 100.0);
        assert!((mid - 0.5).abs() < 1e-4);
        let near = distance_fade([91.0, 0.0, 0.0], eye, 100.0);
        let far = distance_fade([99.0, 0.0, 0.0], eye, 100.0);
        assert!(near > mid && mid > far && far > 0.0);
    }

    #[test]
    fn runs_split_a_batch_where_instances_are_hidden_and_keep_their_order() {
        let batches = [batch(0..5), batch(5..7)];
        let mut out = Vec::new();
        runs(batches.iter().enumerate(), |i| i != 2 && i != 5, &mut out);
        assert_eq!(
            out,
            vec![
                Run {
                    batch: 0,
                    range: 0..2
                },
                Run {
                    batch: 0,
                    range: 3..5
                },
                Run {
                    batch: 1,
                    range: 6..7
                },
            ]
        );
        let counts = counts(&out, |_| 10);
        assert_eq!(
            counts,
            PassCounts {
                draws: 3,
                instances: 5,
                triangles: 50
            }
        );
    }
}
