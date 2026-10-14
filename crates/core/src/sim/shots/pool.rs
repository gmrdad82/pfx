use super::super::{Aabb, Aabb3, Hit};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Span<const N: usize> {
    #[cfg_attr(feature = "serde", serde(with = "super::arr"))]
    pub min: [i32; N],
    #[cfg_attr(feature = "serde", serde(with = "super::arr"))]
    pub max: [i32; N],
}

impl<const N: usize> Span<N> {
    pub const fn new(min: [i32; N], max: [i32; N]) -> Self {
        Self { min, max }
    }

    pub fn around(centre: [i32; N], half: [i32; N]) -> Self {
        Self {
            min: std::array::from_fn(|i| centre[i].saturating_sub(half[i])),
            max: std::array::from_fn(|i| centre[i].saturating_add(half[i])),
        }
    }

    pub fn swept(from: [i32; N], to: [i32; N], half: [i32; N]) -> Self {
        Self {
            min: std::array::from_fn(|i| from[i].min(to[i]).saturating_sub(half[i])),
            max: std::array::from_fn(|i| from[i].max(to[i]).saturating_add(half[i])),
        }
    }

    pub fn is_empty(&self) -> bool {
        (0..N).any(|i| self.max[i] <= self.min[i])
    }

    pub fn overlaps(&self, other: &Span<N>) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && (0..N).all(|i| self.min[i] < other.max[i] && other.min[i] < self.max[i])
    }
}

impl From<Span<2>> for Aabb {
    fn from(span: Span<2>) -> Self {
        Aabb::new(span.min, span.max)
    }
}

impl From<Span<3>> for Aabb3 {
    fn from(span: Span<3>) -> Self {
        Aabb3::new(span.min, span.max)
    }
}

impl From<Aabb> for Span<2> {
    fn from(aabb: Aabb) -> Self {
        Span::new(aabb.min, aabb.max)
    }
}

impl From<Aabb3> for Span<3> {
    fn from(aabb: Aabb3) -> Self {
        Span::new(aabb.min, aabb.max)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Shot<const N: usize> {
    #[cfg_attr(feature = "serde", serde(with = "super::arr"))]
    pub pos: [i32; N],
    #[cfg_attr(feature = "serde", serde(with = "super::arr"))]
    pub vel: [i32; N],
    #[cfg_attr(feature = "serde", serde(with = "super::arr"))]
    pub half: [i32; N],
    pub life: Option<u32>,
    pub pierce: u32,
    pub tag: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShotId {
    pub index: u32,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum End {
    Expired,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Strike<const N: usize> {
    pub shot: ShotId,
    pub target: u32,
    pub gone: bool,
    #[cfg_attr(feature = "serde", serde(with = "super::arr"))]
    pub pos: [i32; N],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Slot<const N: usize> {
    pub(super) shot: Shot<N>,
    pub(super) prev: [i32; N],
    pub(super) generation: u32,
    pub(super) alive: bool,
}

impl<const N: usize> Slot<N> {
    pub(super) const IDLE: Self = Self {
        shot: Shot {
            pos: [0; N],
            vel: [0; N],
            half: [0; N],
            life: None,
            pierce: 0,
            tag: 0,
        },
        prev: [0; N],
        generation: 0,
        alive: false,
    };

    fn same(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.alive == other.alive
            && (!self.alive || (self.shot == other.shot && self.prev == other.prev))
    }
}

#[derive(Clone, Debug)]
pub struct Shots<const N: usize> {
    pub(super) slots: Vec<Slot<N>>,
    pub(super) free: Vec<u32>,
    pub(super) len: usize,
    pub(super) high: usize,
    ended: Vec<(ShotId, End)>,
    order: Vec<(f64, u32)>,
    strikes: Vec<Strike<N>>,
}

impl<const N: usize> PartialEq for Shots<N> {
    fn eq(&self, other: &Self) -> bool {
        self.free == other.free
            && self.len == other.len
            && self.high == other.high
            && self.slots.len() == other.slots.len()
            && self.slots.iter().zip(&other.slots).all(|(a, b)| a.same(b))
    }
}

impl<const N: usize> Eq for Shots<N> {}

impl<const N: usize> Shots<N> {
    pub fn new(capacity: usize) -> Self {
        let capacity = u32::try_from(capacity).expect("at most u32::MAX shots");
        Self::from_parts(
            vec![Slot::IDLE; capacity as usize],
            (0..capacity).rev().collect(),
            0,
            0,
        )
    }

    pub(super) fn from_parts(slots: Vec<Slot<N>>, free: Vec<u32>, len: usize, high: usize) -> Self {
        Self {
            slots,
            free,
            len,
            high,
            ended: Vec::new(),
            order: Vec::new(),
            strikes: Vec::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.free.is_empty()
    }

    pub fn spawn(&mut self, shot: Shot<N>) -> Option<ShotId> {
        let index = self.free.pop()?;
        let slot = &mut self.slots[index as usize];
        slot.shot = shot;
        slot.prev = shot.pos;
        slot.alive = true;
        self.len += 1;
        self.high = self.high.max(index as usize + 1);
        Some(ShotId {
            index,
            generation: slot.generation,
        })
    }

    pub fn despawn(&mut self, id: ShotId) -> Option<Shot<N>> {
        let slot = self.slot(id)?;
        let shot = slot.shot;
        self.release(id.index);
        Some(shot)
    }

    pub fn contains(&self, id: ShotId) -> bool {
        self.slot(id).is_some()
    }

    pub fn get(&self, id: ShotId) -> Option<&Shot<N>> {
        self.slot(id).map(|slot| &slot.shot)
    }

    pub fn get_mut(&mut self, id: ShotId) -> Option<&mut Shot<N>> {
        let slot = self.slots.get_mut(id.index as usize)?;
        (slot.alive && slot.generation == id.generation).then_some(&mut slot.shot)
    }

    pub fn swept(&self, id: ShotId) -> Option<Span<N>> {
        self.slot(id)
            .map(|slot| Span::swept(slot.prev, slot.shot.pos, slot.shot.half))
    }

    pub fn iter(&self) -> impl Iterator<Item = (ShotId, &Shot<N>)> {
        self.slots[..self.high]
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.alive)
            .map(|(index, slot)| {
                (
                    ShotId {
                        index: index as u32,
                        generation: slot.generation,
                    },
                    &slot.shot,
                )
            })
    }

    pub fn step(&mut self, bounds: Span<N>) -> &[(ShotId, End)] {
        self.ended.clear();
        for index in 0..self.high {
            let slot = &mut self.slots[index];
            if !slot.alive {
                continue;
            }
            let id = ShotId {
                index: index as u32,
                generation: slot.generation,
            };
            if slot.shot.life == Some(0) {
                self.ended.push((id, End::Expired));
                self.release(id.index);
                continue;
            }
            slot.prev = slot.shot.pos;
            for axis in 0..N {
                slot.shot.pos[axis] = slot.shot.pos[axis].saturating_add(slot.shot.vel[axis]);
            }
            if let Some(life) = &mut slot.shot.life {
                *life -= 1;
            }
            if !Span::swept(slot.prev, slot.shot.pos, slot.shot.half).overlaps(&bounds) {
                self.ended.push((id, End::Left));
                self.release(id.index);
            }
        }
        &self.ended
    }

    pub fn ended(&self) -> &[(ShotId, End)] {
        &self.ended
    }

    pub fn boxes<B: From<Span<N>>>(&self, ids: &mut Vec<ShotId>, boxes: &mut Vec<B>) {
        ids.clear();
        boxes.clear();
        for (index, slot) in self.slots[..self.high].iter().enumerate() {
            if slot.alive {
                ids.push(ShotId {
                    index: index as u32,
                    generation: slot.generation,
                });
                boxes.push(B::from(Span::swept(
                    slot.prev,
                    slot.shot.pos,
                    slot.shot.half,
                )));
            }
        }
    }

    pub fn resolve<B: Copy + Into<Span<N>>>(
        &mut self,
        ids: &[ShotId],
        hits: &[Hit],
        targets: &[B],
        mut accept: impl FnMut(ShotId, u32) -> bool,
    ) -> &[Strike<N>] {
        self.strikes.clear();
        let mut at = 0;
        while at < hits.len() {
            let a = hits[at].a;
            let mut end = at;
            while end < hits.len() && hits[end].a == a {
                end += 1;
            }
            let group = &hits[at..end];
            at = end;
            let Some(&id) = ids.get(a as usize) else {
                continue;
            };
            let Some(slot) = self.slot(id).copied() else {
                continue;
            };
            self.order.clear();
            if let [hit] = group {
                if (hit.b as usize) < targets.len() {
                    self.order.push((0.0, hit.b));
                }
            } else {
                for hit in group {
                    if let Some(target) = targets.get(hit.b as usize) {
                        self.order.push((entry(&slot, &(*target).into()), hit.b));
                    }
                }
                self.order
                    .sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
            }
            let mut pierce = slot.shot.pierce;
            for index in 0..self.order.len() {
                let target = self.order[index].1;
                if !accept(id, target) {
                    continue;
                }
                let gone = pierce == 0;
                self.strikes.push(Strike {
                    shot: id,
                    target,
                    gone,
                    pos: slot.shot.pos,
                });
                if gone {
                    self.release(id.index);
                    break;
                }
                pierce -= 1;
            }
            if let Some(shot) = self.get_mut(id) {
                shot.pierce = pierce;
            }
        }
        &self.strikes
    }

    pub fn clear(&mut self) {
        for index in (0..self.high).rev() {
            if self.slots[index].alive {
                self.release(index as u32);
            }
        }
        self.free.clear();
        self.free.extend((0..self.slots.len() as u32).rev());
        self.high = 0;
        self.ended.clear();
    }

    fn slot(&self, id: ShotId) -> Option<&Slot<N>> {
        let slot = self.slots.get(id.index as usize)?;
        (slot.alive && slot.generation == id.generation).then_some(slot)
    }

    fn release(&mut self, index: u32) {
        let slot = &mut self.slots[index as usize];
        slot.alive = false;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(index);
        self.len -= 1;
        while self.high > 0 && !self.slots[self.high - 1].alive {
            self.high -= 1;
        }
    }
}

fn entry<const N: usize>(slot: &Slot<N>, target: &Span<N>) -> f64 {
    let mut first = 0.0_f64;
    for axis in 0..N {
        let from = i64::from(slot.prev[axis]);
        let half = i64::from(slot.shot.half[axis]);
        let moved = i64::from(slot.shot.pos[axis]) - from;
        let gap = if moved > 0 {
            i64::from(target.min[axis]) - (from + half)
        } else if moved < 0 {
            i64::from(target.max[axis]) - (from - half)
        } else {
            continue;
        };
        let at = gap as f64 / moved as f64;
        if at > first {
            first = at;
        }
    }
    if first > 1.0 { 1.0 } else { first }
}
