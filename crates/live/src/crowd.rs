use crate::cull::{Frustum, Run, UNBOUNDED, world_sphere};
use crate::frame::{Batch, Matrix, MeshHandle};

pub const CHUNK: usize = 1024;

#[derive(Clone, Copy, Debug)]
pub struct Crowd<'a> {
    pub mesh: MeshHandle,
    pub material: u32,
    pub id: u32,
    pub models: &'a [Matrix],
    pub casts_shadow: bool,
    pub shadow_only: bool,
    pub two_sided: bool,
    pub alpha_cutoff: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CrowdRef {
    pub crowd: u32,
    pub chunk: u32,
    pub material: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub crowd: u32,
    pub range: std::ops::Range<u32>,
    pub sphere: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Staged {
    pub mesh: MeshHandle,
    pub material: u32,
    pub id: u32,
    pub casts_shadow: bool,
    pub shadow_only: bool,
    pub two_sided: bool,
    pub alpha_cutoff: f32,
    pub models: Vec<Matrix>,
    pub start: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Crowds {
    pub staged: Vec<Staged>,
    pub chunks: Vec<Chunk>,
    pub base: u32,
    pub count: u32,
    pub dirty: bool,
    pub epoch: u64,
}

fn morton(x: u32, z: u32) -> u64 {
    let spread = |v: u32| {
        let mut v = u64::from(v);
        v = (v | (v << 16)) & 0x0000_ffff_0000_ffff;
        v = (v | (v << 8)) & 0x00ff_00ff_00ff_00ff;
        v = (v | (v << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
        v = (v | (v << 2)) & 0x3333_3333_3333_3333;
        (v | (v << 1)) & 0x5555_5555_5555_5555
    };
    spread(x) | (spread(z) << 1)
}

pub fn ordered(models: &[Matrix]) -> Vec<Matrix> {
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for model in models {
        let at = [model[3][0], model[3][2]];
        for axis in 0..2 {
            if at[axis].is_finite() {
                low[axis] = low[axis].min(at[axis]);
                high[axis] = high[axis].max(at[axis]);
            }
        }
    }
    let cell = |value: f32, axis: usize| -> u32 {
        let span = high[axis] - low[axis];
        if !(span > 0.0 && value.is_finite()) {
            return 0;
        }
        (((value - low[axis]) / span) * 65535.0).clamp(0.0, 65535.0) as u32
    };
    let mut keyed: Vec<(u64, usize)> = models
        .iter()
        .enumerate()
        .map(|(index, model)| (morton(cell(model[3][0], 0), cell(model[3][2], 1)), index))
        .collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, index)| models[index]).collect()
}

pub fn enclosing(spheres: impl Iterator<Item = [f32; 4]> + Clone) -> [f32; 4] {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for sphere in spheres.clone() {
        if !sphere[3].is_finite() {
            return UNBOUNDED;
        }
        for axis in 0..3 {
            low[axis] = low[axis].min(f64::from(sphere[axis] - sphere[3]));
            high[axis] = high[axis].max(f64::from(sphere[axis] + sphere[3]));
        }
    }
    if !low[0].is_finite() {
        return UNBOUNDED;
    }
    let centre: [f64; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
    let radius = spheres
        .map(|sphere| {
            let d: f64 = (0..3)
                .map(|axis| (f64::from(sphere[axis]) - centre[axis]).powi(2))
                .sum::<f64>()
                .sqrt();
            d + f64::from(sphere[3])
        })
        .fold(0.0f64, f64::max);
    let sphere = [
        centre[0] as f32,
        centre[1] as f32,
        centre[2] as f32,
        (radius * 1.0001 + 1e-6) as f32,
    ];
    if sphere.iter().all(|value| value.is_finite()) {
        sphere
    } else {
        UNBOUNDED
    }
}

impl Crowds {
    pub fn set(&mut self, crowds: &[Crowd<'_>], local: impl Fn(MeshHandle) -> [f32; 4]) {
        self.staged.clear();
        self.chunks.clear();
        let mut start = 0u32;
        for crowd in crowds {
            if crowd.models.is_empty() {
                continue;
            }
            let models = ordered(crowd.models);
            let sphere = local(crowd.mesh);
            for (at, piece) in models.chunks(CHUNK).enumerate() {
                let from = start + (at * CHUNK) as u32;
                self.chunks.push(Chunk {
                    crowd: self.staged.len() as u32,
                    range: from..from + piece.len() as u32,
                    sphere: enclosing(piece.iter().map(|model| world_sphere(sphere, *model))),
                });
            }
            let count = models.len() as u32;
            self.staged.push(Staged {
                mesh: crowd.mesh,
                material: crowd.material,
                id: crowd.id,
                casts_shadow: crowd.casts_shadow,
                shadow_only: crowd.shadow_only,
                two_sided: crowd.two_sided,
                alpha_cutoff: crowd.alpha_cutoff,
                models,
                start,
            });
            start += count;
        }
        self.count = start;
        self.dirty = true;
        self.epoch += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.staged.is_empty()
    }

    pub fn copies(&self) -> usize {
        self.count as usize
    }

    pub fn reserve(&mut self, dynamic: usize) -> usize {
        if self.staged.is_empty() {
            return dynamic;
        }
        if dynamic as u32 > self.base {
            self.base = (dynamic as u32).next_power_of_two();
            self.dirty = true;
        }
        (self.base + self.count) as usize
    }

    pub fn batches(
        &self,
        out: &mut Vec<Batch>,
        swaying: impl Fn(MeshHandle) -> bool,
        cut: impl Fn(u32) -> bool,
    ) {
        for (index, chunk) in self.chunks.iter().enumerate() {
            let staged = &self.staged[chunk.crowd as usize];
            out.push(Batch {
                mesh: staged.mesh,
                two_sided: staged.two_sided,
                moving: swaying(staged.mesh),
                clipped: false,
                cut: cut(staged.material),
                shadow_only: staged.shadow_only,
                casts_shadow: staged.casts_shadow,
                range: self.base + chunk.range.start..self.base + chunk.range.end,
                crowd: Some(CrowdRef {
                    crowd: chunk.crowd,
                    chunk: index as u32,
                    material: staged.material,
                }),
            });
        }
    }

    pub fn runs<'a>(
        &self,
        batches: impl Iterator<Item = (usize, &'a Batch)>,
        frustum: Option<&Frustum>,
        out: &mut Vec<Run>,
    ) {
        for (index, batch) in batches {
            let Some(crowd) = batch.crowd else {
                continue;
            };
            let seen = match (frustum, self.chunks.get(crowd.chunk as usize)) {
                (Some(frustum), Some(chunk)) => frustum.sees(chunk.sphere),
                _ => true,
            };
            if seen {
                out.push(Run {
                    batch: index,
                    range: batch.range.clone(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, z: f32) -> Matrix {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [x, 0.0, z, 1.0],
        ]
    }

    fn crowd(models: &[Matrix]) -> Crowd<'_> {
        Crowd {
            mesh: MeshHandle(3),
            material: 2,
            id: 7,
            models,
            casts_shadow: true,
            shadow_only: false,
            two_sided: false,
            alpha_cutoff: 0.5,
        }
    }

    fn grid(side: usize) -> Vec<Matrix> {
        (0..side * side)
            .map(|i| at((i % side) as f32, (i / side) as f32))
            .collect()
    }

    #[test]
    fn copies_are_ordered_by_place_the_same_way_every_time_and_none_is_lost() {
        let models = grid(40);
        let mut reversed = models.clone();
        reversed.reverse();
        let a = ordered(&models);
        assert_eq!(a, ordered(&models));
        assert_eq!(a, ordered(&reversed));
        assert_eq!(a.len(), models.len());
        let mut seen: Vec<[f32; 2]> = a.iter().map(|m| [m[3][0], m[3][2]]).collect();
        seen.sort_by(|p, q| p.partial_cmp(q).unwrap());
        let mut all: Vec<[f32; 2]> = models.iter().map(|m| [m[3][0], m[3][2]]).collect();
        all.sort_by(|p, q| p.partial_cmp(q).unwrap());
        assert_eq!(seen, all);
    }

    #[test]
    fn every_chunk_sphere_holds_each_of_its_copies() {
        let models = grid(50);
        let mut crowds = Crowds::default();
        crowds.set(&[crowd(&models)], |_| [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(crowds.count, 2500);
        assert_eq!(crowds.chunks.len(), 2500usize.div_ceil(CHUNK));
        let staged = &crowds.staged[0];
        for chunk in &crowds.chunks {
            let s = chunk.sphere;
            for model in &staged.models[chunk.range.start as usize..chunk.range.end as usize] {
                let d = ((model[3][0] - s[0]).powi(2)
                    + (model[3][1] - s[1]).powi(2)
                    + (model[3][2] - s[2]).powi(2))
                .sqrt();
                assert!(d + 0.5 <= s[3], "{d} {s:?}");
            }
        }
    }

    #[test]
    fn the_batches_and_runs_grow_with_chunks_never_with_copies() {
        let few = grid(10);
        let many = grid(30);
        let mut a = Crowds::default();
        a.set(&[crowd(&few)], |_| [0.0, 0.0, 0.0, 0.5]);
        let mut b = Crowds::default();
        b.set(&[crowd(&many)], |_| [0.0, 0.0, 0.0, 0.5]);
        assert_eq!((a.reserve(5), b.reserve(5)), (108, 908));
        let (mut ba, mut bb) = (Vec::new(), Vec::new());
        a.batches(&mut ba, |_| false, |_| false);
        b.batches(&mut bb, |_| false, |_| false);
        assert_eq!(ba.len(), 1);
        assert_eq!(bb.len(), 1);
        assert_eq!(bb[0].range, 8..908);
        let mut runs = Vec::new();
        b.runs(bb.iter().enumerate(), None, &mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].range, 8..908);
    }

    #[test]
    fn more_dynamic_instances_than_the_base_move_the_copies_and_mark_them_for_writing() {
        let models = grid(4);
        let mut crowds = Crowds::default();
        crowds.set(&[crowd(&models)], |_| [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(crowds.reserve(3), 4 + 16);
        crowds.dirty = false;
        assert_eq!(crowds.reserve(4), 4 + 16);
        assert!(!crowds.dirty);
        assert_eq!(crowds.reserve(9), 16 + 16);
        assert!(crowds.dirty);
    }
}
