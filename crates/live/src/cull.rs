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
    pub drawable: u32,
    pub camera: PassCounts,
    pub occlusion_tested: bool,
    pub occluded: Option<u32>,
    pub cascades: Vec<PassCounts>,
    pub light_faces: PassCounts,
}

impl CullStats {
    pub fn culled(&self) -> u32 {
        self.drawable.saturating_sub(self.camera.instances)
    }
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

pub type Extent = [[f32; 3]; 2];

pub fn local_box(positions: &[[f32; 3]]) -> Option<Extent> {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for position in positions {
        for axis in 0..3 {
            low[axis] = low[axis].min(position[axis]);
            high[axis] = high[axis].max(position[axis]);
        }
    }
    (!positions.is_empty() && positions.iter().flatten().all(|value| value.is_finite()))
        .then_some([low, high])
}

pub fn world_box(local: Extent, model: Matrix) -> Option<Extent> {
    if model[0][3] != 0.0 || model[1][3] != 0.0 || model[2][3] != 0.0 || model[3][3] != 1.0 {
        return None;
    }
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for corner in 0..8 {
        let point: [f64; 3] =
            std::array::from_fn(|axis| f64::from(local[(corner >> axis) & 1][axis]));
        for row in 0..3 {
            let value = (0..3)
                .map(|column| f64::from(model[column][row]) * point[column])
                .sum::<f64>()
                + f64::from(model[3][row]);
            low[row] = low[row].min(value);
            high[row] = high[row].max(value);
        }
    }
    let pad = |value: f64, toward: f64| (value + toward * (value.abs() * 1e-6 + 1e-5)) as f32;
    let extent = [
        low.map(|value| pad(value, -1.0)),
        high.map(|value| pad(value, 1.0)),
    ];
    extent
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(extent)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub batch: usize,
    pub start: u32,
    pub count: u32,
}

pub fn groups<K: Eq + std::hash::Hash>(
    runs: &[Run],
    key: impl Fn(usize) -> K,
    longest: u32,
) -> (Vec<Group>, Vec<u32>) {
    let mut found: std::collections::HashMap<K, usize> = std::collections::HashMap::new();
    let mut gathered: Vec<(usize, Vec<u32>)> = Vec::new();
    for run in runs {
        let at = *found.entry(key(run.batch)).or_insert_with(|| {
            gathered.push((run.batch, Vec::new()));
            gathered.len() - 1
        });
        gathered[at].1.extend(run.range.clone());
    }
    let mut out = Vec::new();
    let mut members = Vec::new();
    for (batch, list) in gathered {
        for piece in list.chunks(longest.max(1) as usize) {
            out.push(Group {
                batch,
                start: members.len() as u32,
                count: piece.len() as u32,
            });
            members.extend_from_slice(piece);
        }
    }
    (out, members)
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
    fn a_local_box_spans_the_positions_and_refuses_none_or_non_finite() {
        assert_eq!(
            local_box(&[[1.0, -2.0, 3.0], [-1.0, 4.0, 0.5], [0.0, 0.0, 0.0]]),
            Some([[-1.0, -2.0, 0.0], [1.0, 4.0, 3.0]])
        );
        assert_eq!(local_box(&[[2.0; 3]]), Some([[2.0; 3], [2.0; 3]]));
        assert_eq!(local_box(&[]), None);
        assert_eq!(local_box(&[[0.0; 3], [f32::NAN, 0.0, 0.0]]), None);
        assert_eq!(local_box(&[[0.0; 3], [0.0, f32::INFINITY, 0.0]]), None);
    }

    fn turned(degrees: f32, scale: f32, at: [f32; 3]) -> Matrix {
        let (sin, cos) = degrees.to_radians().sin_cos();
        [
            [cos * scale, 0.0, -sin * scale, 0.0],
            [0.0, scale, 0.0, 0.0],
            [sin * scale, 0.0, cos * scale, 0.0],
            [at[0], at[1], at[2], 1.0],
        ]
    }

    fn apply(model: Matrix, point: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|row| {
            (0..3)
                .map(|column| model[column][row] * point[column])
                .sum::<f32>()
                + model[3][row]
        })
    }

    #[test]
    fn a_world_box_holds_every_turned_corner_strictly_inside_its_padding() {
        let local = [[-0.5, 0.0, -1.0], [0.5, 2.0, 1.0]];
        for (degrees, scale, at) in [
            (0.0, 1.0, [0.0; 3]),
            (37.0, 2.5, [3.0, -1.0, 9.0]),
            (90.0, 0.25, [-400.0, 0.0, 1200.0]),
            (-135.0, 1.0, [1.0e5, 2.0, -1.0e5]),
        ] {
            let model = turned(degrees, scale, at);
            let [low, high] = world_box(local, model).unwrap();
            for corner in 0..8 {
                let point: [f32; 3] = std::array::from_fn(|axis| local[(corner >> axis) & 1][axis]);
                let world = apply(model, point);
                for axis in 0..3 {
                    assert!(
                        low[axis] < world[axis] && world[axis] < high[axis],
                        "{degrees}° corner {corner} axis {axis}: {} not in {}..{}",
                        world[axis],
                        low[axis],
                        high[axis]
                    );
                }
            }
            let centre = apply(model, [0.0, 1.0, 0.0]);
            let reach = 2.5 * scale;
            for axis in 0..3 {
                assert!(high[axis] - low[axis] <= 2.0 * reach + 1e-3 * (1.0 + centre[axis].abs()));
            }
        }
    }

    #[test]
    fn a_world_box_refuses_a_projective_model_and_a_non_finite_result() {
        let local = [[-1.0; 3], [1.0; 3]];
        let mut projective = identity();
        projective[2][3] = -1.0;
        assert_eq!(world_box(local, projective), None);
        let mut weighted = identity();
        weighted[3][3] = 2.0;
        assert_eq!(world_box(local, weighted), None);
        let mut huge = identity();
        huge[0][0] = f32::MAX;
        assert_eq!(world_box(local, huge), None);
        let mut lost = identity();
        lost[3][0] = f32::NAN;
        assert_eq!(world_box(local, lost), None);
    }

    #[test]
    fn groups_split_long_runs_take_a_zero_limit_as_one_and_keep_nothing_from_nothing() {
        let runs = [Run {
            batch: 0,
            range: 0..5,
        }];
        let (found, members) = groups(&runs, |_| 0, 0);
        assert_eq!(members, vec![0, 1, 2, 3, 4]);
        assert_eq!(found.len(), 5);
        assert!(
            found
                .iter()
                .enumerate()
                .all(|(at, group)| group.start == at as u32
                    && group.count == 1
                    && group.batch == 0)
        );
        let (found, members) = groups(&runs, |_| 0, 256);
        assert_eq!(
            found,
            vec![Group {
                batch: 0,
                start: 0,
                count: 5
            }]
        );
        assert_eq!(members.len(), 5);
        let (found, members) = groups(&[], |_: usize| 0, 256);
        assert!(found.is_empty() && members.is_empty());
    }

    #[test]
    fn groups_cover_every_kept_instance_once_and_never_mix_kinds() {
        let runs: Vec<Run> = (0..40)
            .map(|batch| Run {
                batch,
                range: (batch as u32 * 3)..(batch as u32 * 3 + 1 + batch as u32 % 3),
            })
            .collect();
        let kind = |batch: usize| batch % 4;
        let (found, members) = groups(&runs, kind, 5);
        let mut seen: Vec<u32> = members.clone();
        seen.sort_unstable();
        let mut expected: Vec<u32> = runs.iter().flat_map(|run| run.range.clone()).collect();
        expected.sort_unstable();
        assert_eq!(seen, expected);
        let mut next = 0;
        for group in &found {
            assert_eq!(group.start, next);
            assert!(group.count >= 1 && group.count <= 5);
            next += group.count;
            for member in &members[group.start as usize..(group.start + group.count) as usize] {
                let owner = runs.iter().find(|run| run.range.contains(member)).unwrap();
                assert_eq!(kind(owner.batch), kind(group.batch));
            }
        }
        assert_eq!(next as usize, members.len());
    }

    #[test]
    fn culled_counts_what_the_camera_dropped_and_never_goes_below_zero() {
        let stats = CullStats {
            drawable: 10,
            camera: PassCounts {
                draws: 2,
                instances: 7,
                triangles: 70,
            },
            ..CullStats::default()
        };
        assert_eq!(stats.culled(), 3);
        let odd = CullStats {
            drawable: 1,
            camera: PassCounts {
                instances: 4,
                ..PassCounts::default()
            },
            ..CullStats::default()
        };
        assert_eq!(odd.culled(), 0);
        assert_eq!(CullStats::default().culled(), 0);
        assert_eq!(CullStats::default().occluded, None);
    }

    #[test]
    fn groups_gather_runs_of_one_kind_in_first_seen_order_and_keep_instance_order() {
        let runs = [
            Run {
                batch: 0,
                range: 0..1,
            },
            Run {
                batch: 1,
                range: 1..4,
            },
            Run {
                batch: 2,
                range: 4..5,
            },
            Run {
                batch: 3,
                range: 5..7,
            },
        ];
        let kinds = [7, 9, 7, 9];
        let (found, members) = groups(&runs, |batch| kinds[batch], 3);
        assert_eq!(members, vec![0, 4, 1, 2, 3, 5, 6]);
        assert_eq!(
            found,
            vec![
                Group {
                    batch: 0,
                    start: 0,
                    count: 2
                },
                Group {
                    batch: 1,
                    start: 2,
                    count: 3
                },
                Group {
                    batch: 1,
                    start: 5,
                    count: 2
                },
            ]
        );
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
