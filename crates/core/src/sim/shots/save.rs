use serde::de::Error;
use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::pool::{Shot, Shots, Slot};

const MAX_SAVED: usize = 1 << 20;

#[derive(Serialize, Deserialize)]
struct Live<const N: usize> {
    index: u32,
    shot: Shot<N>,
    #[serde(with = "super::arr")]
    prev: [i32; N],
}

#[derive(Deserialize)]
struct Saved<const N: usize> {
    capacity: u32,
    generations: Vec<u32>,
    fresh: u32,
    free: Vec<u32>,
    live: Vec<Live<N>>,
}

struct LiveSlots<'a, const N: usize>(&'a Shots<N>);

impl<const N: usize> Serialize for LiveSlots<'_, N> {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        let shots = self.0;
        let mut seq = out.serialize_seq(Some(shots.len))?;
        for (index, slot) in shots.slots[..shots.high].iter().enumerate() {
            if slot.alive {
                seq.serialize_element(&Live {
                    index: index as u32,
                    shot: slot.shot,
                    prev: slot.prev,
                })?;
            }
        }
        seq.end()
    }
}

struct Generations<'a, const N: usize>(&'a [Slot<N>]);

impl<const N: usize> Serialize for Generations<'_, N> {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        out.collect_seq(self.0.iter().map(|slot| slot.generation))
    }
}

impl<const N: usize> Serialize for Shots<N> {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        let capacity = self.slots.len() as u32;
        let touched = self
            .slots
            .iter()
            .rposition(|slot| slot.generation != 0)
            .map_or(0, |at| at + 1);
        let fresh = self
            .free
            .iter()
            .zip((0..capacity).rev())
            .take_while(|(index, expected)| **index == *expected)
            .count();
        let mut saved = out.serialize_struct("Shots", 5)?;
        saved.serialize_field("capacity", &capacity)?;
        saved.serialize_field("generations", &Generations(&self.slots[..touched]))?;
        saved.serialize_field("fresh", &(fresh as u32))?;
        saved.serialize_field("free", &self.free[fresh..])?;
        saved.serialize_field("live", &LiveSlots(self))?;
        saved.end()
    }
}

impl<'de, const N: usize> Deserialize<'de> for Shots<N> {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let saved = Saved::<N>::deserialize(input)?;
        rebuild(saved).map_err(D::Error::custom)
    }
}

fn rebuild<const N: usize>(saved: Saved<N>) -> Result<Shots<N>, &'static str> {
    let capacity = saved.capacity as usize;
    if capacity > MAX_SAVED {
        return Err("saved shots claim more than 1048576 slots, the limit");
    }
    if saved.generations.len() > capacity || saved.fresh > saved.capacity {
        return Err("saved shots outgrow their capacity");
    }
    if saved.fresh as usize + saved.free.len() + saved.live.len() != capacity {
        return Err("saved shots miss a slot");
    }
    let mut seen = vec![false; capacity];
    let mut mark = |index: u32| -> Result<(), &'static str> {
        match seen.get_mut(index as usize) {
            Some(slot) if !*slot => {
                *slot = true;
                Ok(())
            }
            Some(_) => Err("a saved shot slot is listed twice"),
            None => Err("a saved shot slot is past the capacity"),
        }
    };
    let mut free = Vec::with_capacity(capacity);
    free.extend((saved.capacity - saved.fresh..saved.capacity).rev());
    free.extend_from_slice(&saved.free);
    for &index in &free {
        mark(index)?;
    }
    let mut slots = vec![Slot::IDLE; capacity];
    for (slot, &generation) in slots.iter_mut().zip(&saved.generations) {
        slot.generation = generation;
    }
    let mut high = 0;
    for live in &saved.live {
        mark(live.index)?;
        let at = live.index as usize;
        if at < high {
            return Err("saved live shots are out of order");
        }
        let slot = &mut slots[at];
        slot.shot = live.shot;
        slot.prev = live.prev;
        slot.alive = true;
        high = at + 1;
    }
    if free.len() + saved.live.len() != capacity {
        return Err("saved shots miss a slot");
    }
    Ok(Shots::from_parts(slots, free, saved.live.len(), high))
}
