use super::super::{Aabb, Aabb3};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span<const N: usize> {
    pub min: [i32; N],
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shot<const N: usize> {
    pub pos: [i32; N],
    pub vel: [i32; N],
    pub half: [i32; N],
    pub life: Option<u32>,
    pub tag: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShotId {
    pub index: u32,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum End {
    Expired,
    Left,
}

#[derive(Clone, Copy, Debug)]
struct Slot<const N: usize> {
    shot: Shot<N>,
    prev: [i32; N],
    generation: u32,
    alive: bool,
}

#[derive(Clone, Debug)]
pub struct Shots<const N: usize> {
    slots: Vec<Slot<N>>,
    free: Vec<u32>,
    len: usize,
    ended: Vec<(ShotId, End)>,
}

impl<const N: usize> Shots<N> {
    pub fn new(capacity: usize) -> Self {
        let capacity = u32::try_from(capacity).expect("at most u32::MAX shots");
        let idle = Slot {
            shot: Shot {
                pos: [0; N],
                vel: [0; N],
                half: [0; N],
                life: None,
                tag: 0,
            },
            prev: [0; N],
            generation: 0,
            alive: false,
        };
        Self {
            slots: vec![idle; capacity as usize],
            free: (0..capacity).rev().collect(),
            len: 0,
            ended: Vec::new(),
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
        self.slots
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
        for index in 0..self.slots.len() {
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
        for (index, slot) in self.slots.iter().enumerate() {
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

    pub fn clear(&mut self) {
        for index in (0..self.slots.len()).rev() {
            if self.slots[index].alive {
                self.release(index as u32);
            }
        }
        self.free.clear();
        self.free.extend((0..self.slots.len() as u32).rev());
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
    }
}
