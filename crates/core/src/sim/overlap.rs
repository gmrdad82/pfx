#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Aabb {
    pub min: [i32; 2],
    pub max: [i32; 2],
}

impl Aabb {
    pub const fn new(min: [i32; 2], max: [i32; 2]) -> Self {
        Self { min, max }
    }

    pub const fn point(at: [i32; 2]) -> Self {
        Self {
            min: at,
            max: [at[0].saturating_add(1), at[1].saturating_add(1)],
        }
    }

    pub const fn around(centre: [i32; 2], half: [i32; 2]) -> Self {
        Self {
            min: [
                centre[0].saturating_sub(half[0]),
                centre[1].saturating_sub(half[1]),
            ],
            max: [
                centre[0].saturating_add(half[0]),
                centre[1].saturating_add(half[1]),
            ],
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.max[0] <= self.min[0] || self.max[1] <= self.min[1]
    }

    pub const fn overlaps(&self, other: &Aabb) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.min[0] < other.max[0]
            && other.min[0] < self.max[0]
            && self.min[1] < other.max[1]
            && other.min[1] < self.max[1]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hit {
    pub a: u32,
    pub b: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    start: i32,
    end: i32,
    low: i32,
    high: i32,
    index: u32,
    side: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Overlaps {
    entries: Vec<Entry>,
    active: [Vec<Entry>; 2],
    keys: Vec<u64>,
    hits: Vec<Hit>,
}

impl Overlaps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn find(&mut self, a: &[Aabb], b: &[Aabb]) -> &[Hit] {
        self.entries.clear();
        self.keys.clear();
        self.hits.clear();
        let axis = sweep_axis(a.iter().chain(b));
        let other = 1 - axis;
        for (side, boxes) in [a, b].into_iter().enumerate() {
            for (index, aabb) in boxes.iter().enumerate() {
                if aabb.is_empty() {
                    continue;
                }
                self.entries.push(Entry {
                    start: aabb.min[axis],
                    end: aabb.max[axis],
                    low: aabb.min[other],
                    high: aabb.max[other],
                    index: u32::try_from(index).expect("at most u32::MAX boxes a side"),
                    side: side as u8,
                });
            }
        }
        self.entries
            .sort_unstable_by_key(|entry| (entry.start, entry.side, entry.index));
        self.active[0].clear();
        self.active[1].clear();
        for entry in &self.entries {
            let side = usize::from(entry.side);
            let others = &mut self.active[1 - side];
            let mut at = 0;
            while at < others.len() {
                let seen = others[at];
                if seen.end <= entry.start {
                    others.swap_remove(at);
                    continue;
                }
                if seen.low < entry.high && entry.low < seen.high {
                    let (a, b) = if side == 0 {
                        (entry.index, seen.index)
                    } else {
                        (seen.index, entry.index)
                    };
                    self.keys.push((u64::from(a) << 32) | u64::from(b));
                }
                at += 1;
            }
            self.active[side].push(*entry);
        }
        self.keys.sort_unstable();
        self.hits.extend(self.keys.iter().map(|key| Hit {
            a: (key >> 32) as u32,
            b: *key as u32,
        }));
        &self.hits
    }

    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }
}

fn sweep_axis<'a>(boxes: impl Iterator<Item = &'a Aabb>) -> usize {
    let mut low = [i32::MAX; 2];
    let mut high = [i32::MIN; 2];
    for aabb in boxes.filter(|aabb| !aabb.is_empty()) {
        for axis in 0..2 {
            low[axis] = low[axis].min(aabb.min[axis]);
            high[axis] = high[axis].max(aabb.min[axis]);
        }
    }
    let spread = |axis: usize| i64::from(high[axis]) - i64::from(low[axis]);
    usize::from(spread(1) > spread(0))
}
