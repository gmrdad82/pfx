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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Aabb3 {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl Aabb3 {
    pub const fn new(min: [i32; 3], max: [i32; 3]) -> Self {
        Self { min, max }
    }

    pub const fn point(at: [i32; 3]) -> Self {
        Self {
            min: at,
            max: [
                at[0].saturating_add(1),
                at[1].saturating_add(1),
                at[2].saturating_add(1),
            ],
        }
    }

    pub const fn around(centre: [i32; 3], half: [i32; 3]) -> Self {
        Self {
            min: [
                centre[0].saturating_sub(half[0]),
                centre[1].saturating_sub(half[1]),
                centre[2].saturating_sub(half[2]),
            ],
            max: [
                centre[0].saturating_add(half[0]),
                centre[1].saturating_add(half[1]),
                centre[2].saturating_add(half[2]),
            ],
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.max[0] <= self.min[0] || self.max[1] <= self.min[1] || self.max[2] <= self.min[2]
    }

    pub const fn overlaps(&self, other: &Aabb3) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.min[0] < other.max[0]
            && other.min[0] < self.max[0]
            && self.min[1] < other.max[1]
            && other.min[1] < self.max[1]
            && self.min[2] < other.max[2]
            && other.min[2] < self.max[2]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hit {
    pub a: u32,
    pub b: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Overlaps(Engine<2>);

impl Overlaps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn find(&mut self, a: &[Aabb], b: &[Aabb]) -> &[Hit] {
        self.0.load(
            a.iter().map(|x| (x.min, x.max)),
            b.iter().map(|x| (x.min, x.max)),
        );
        self.0.run()
    }

    pub fn hits(&self) -> &[Hit] {
        &self.0.hits
    }
}

#[derive(Clone, Debug, Default)]
pub struct Overlaps3(Engine<3>);

impl Overlaps3 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn find(&mut self, a: &[Aabb3], b: &[Aabb3]) -> &[Hit] {
        self.0.load(
            a.iter().map(|x| (x.min, x.max)),
            b.iter().map(|x| (x.min, x.max)),
        );
        self.0.run()
    }

    pub fn hits(&self) -> &[Hit] {
        &self.0.hits
    }
}

#[derive(Clone, Copy, Debug)]
struct Span<const N: usize> {
    min: [i32; N],
    max: [i32; N],
    index: u32,
}

impl<const N: usize> Span<N> {
    fn overlaps(&self, other: &Span<N>) -> bool {
        (0..N).all(|k| self.min[k] < other.max[k] && other.min[k] < self.max[k])
    }
}

#[derive(Clone, Copy, Debug)]
struct Grid<const N: usize> {
    origin: [i64; N],
    cell: [i64; N],
    dims: [usize; N],
}

impl<const N: usize> Grid<N> {
    fn at(&self, axis: usize, value: i32) -> usize {
        let at = (i64::from(value) - self.origin[axis]) / self.cell[axis];
        (at.max(0) as usize).min(self.dims[axis] - 1)
    }

    fn range(&self, span: &Span<N>) -> ([usize; N], [usize; N]) {
        (
            std::array::from_fn(|k| self.at(k, span.min[k])),
            std::array::from_fn(|k| self.at(k, span.max[k] - 1)),
        )
    }

    fn index(&self, at: [usize; N]) -> usize {
        (0..N).rev().fold(0, |sum, k| sum * self.dims[k] + at[k])
    }
}

fn cells_in<const N: usize>(low: [usize; N], high: [usize; N]) -> usize {
    (0..N).map(|k| high[k] - low[k] + 1).product()
}

fn each_cell<const N: usize>(low: [usize; N], high: [usize; N], mut visit: impl FnMut([usize; N])) {
    let mut at = low;
    loop {
        visit(at);
        let mut k = 0;
        loop {
            if k == N {
                return;
            }
            if at[k] < high[k] {
                at[k] += 1;
                break;
            }
            at[k] = low[k];
            k += 1;
        }
    }
}

fn fill<const N: usize>(
    spans: &mut Vec<Span<N>>,
    boxes: impl Iterator<Item = ([i32; N], [i32; N])>,
) {
    spans.clear();
    for (index, (min, max)) in boxes.enumerate() {
        if (0..N).any(|k| max[k] <= min[k]) {
            continue;
        }
        assert!(index < 1 << 31, "at most 2^31 boxes a side");
        spans.push(Span {
            min,
            max,
            index: index as u32,
        });
    }
}

#[derive(Clone, Debug)]
struct Engine<const N: usize> {
    boxes: [Vec<Span<N>>; 2],
    keys: Vec<u64>,
    active: [Vec<u32>; 2],
    pairs: Vec<u64>,
    starts: Vec<u32>,
    cursor: Vec<u32>,
    members: Vec<u32>,
    found: Vec<u32>,
    hits: Vec<Hit>,
}

impl<const N: usize> Default for Engine<N> {
    fn default() -> Self {
        Self {
            boxes: Default::default(),
            keys: Vec::new(),
            active: Default::default(),
            pairs: Vec::new(),
            starts: Vec::new(),
            cursor: Vec::new(),
            members: Vec::new(),
            found: Vec::new(),
            hits: Vec::new(),
        }
    }
}

impl<const N: usize> Engine<N> {
    fn load(
        &mut self,
        a: impl Iterator<Item = ([i32; N], [i32; N])>,
        b: impl Iterator<Item = ([i32; N], [i32; N])>,
    ) {
        fill(&mut self.boxes[0], a);
        fill(&mut self.boxes[1], b);
    }

    fn run(&mut self) -> &[Hit] {
        self.hits.clear();
        if self.boxes[0].is_empty() || self.boxes[1].is_empty() {
            return &self.hits;
        }
        match self.plan() {
            Some(grid) => self.through(grid),
            None => self.sweep(),
        }
        &self.hits
    }

    fn plan(&self) -> Option<Grid<N>> {
        let mut low = [i64::MAX; N];
        let mut high = [i64::MIN; N];
        let mut extent = [0i128; N];
        for span in self.boxes.iter().flatten() {
            for k in 0..N {
                low[k] = low[k].min(i64::from(span.min[k]));
                high[k] = high[k].max(i64::from(span.max[k]));
                extent[k] += i128::from(span.max[k]) - i128::from(span.min[k]);
            }
        }
        let count = self.boxes[0].len() + self.boxes[1].len();
        let budget = (4 * count).max(64);
        let mut cell: [i64; N] = std::array::from_fn(|k| (extent[k] / count as i128).max(1) as i64);
        let dims = |cell: &[i64; N]| -> [usize; N] {
            std::array::from_fn(|k| ((high[k] - low[k] + cell[k] - 1) / cell[k]).max(1) as usize)
        };
        let mut grid_dims = dims(&cell);
        while grid_dims.iter().map(|&d| d as u128).product::<u128>() > budget as u128 {
            let widest = (0..N).max_by_key(|&k| (grid_dims[k], std::cmp::Reverse(k)))?;
            cell[widest] = cell[widest].saturating_mul(2);
            grid_dims = dims(&cell);
        }
        let grid = Grid {
            origin: low,
            cell,
            dims: grid_dims,
        };
        let covered: usize = self
            .boxes
            .iter()
            .flatten()
            .map(|span| {
                let (from, to) = grid.range(span);
                cells_in(from, to)
            })
            .sum();
        (covered <= 8 * count + 1024).then_some(grid)
    }

    fn through(&mut self, grid: Grid<N>) {
        let cells: usize = grid.dims.iter().product();
        self.starts.clear();
        self.starts.resize(cells + 1, 0);
        for span in &self.boxes[1] {
            let (from, to) = grid.range(span);
            each_cell(from, to, |at| self.starts[grid.index(at) + 1] += 1);
        }
        for c in 0..cells {
            self.starts[c + 1] += self.starts[c];
        }
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.starts[..cells]);
        self.members.clear();
        self.members.resize(self.starts[cells] as usize, 0);
        for (position, span) in self.boxes[1].iter().enumerate() {
            let (from, to) = grid.range(span);
            each_cell(from, to, |at| {
                let c = grid.index(at);
                self.members[self.cursor[c] as usize] = position as u32;
                self.cursor[c] += 1;
            });
        }
        let targets = &self.boxes[1];
        for shot in &self.boxes[0] {
            self.found.clear();
            let (from, to) = grid.range(shot);
            each_cell(from, to, |at| {
                let c = grid.index(at);
                let edge: [i64; N] = std::array::from_fn(|k| {
                    if at[k] == from[k] {
                        i64::MIN
                    } else {
                        grid.origin[k] + at[k] as i64 * grid.cell[k]
                    }
                });
                for &position in &self.members[self.starts[c] as usize..self.starts[c + 1] as usize]
                {
                    let target = &targets[position as usize];
                    if shot.overlaps(target) && (0..N).all(|k| i64::from(target.min[k]) >= edge[k])
                    {
                        self.found.push(target.index);
                    }
                }
            });
            self.found.sort_unstable();
            self.hits
                .extend(self.found.iter().map(|&b| Hit { a: shot.index, b }));
        }
    }

    fn sweep(&mut self) {
        let axis = self.sweep_axis();
        let Self {
            boxes,
            keys,
            active,
            pairs,
            hits,
            ..
        } = self;
        keys.clear();
        for (side, spans) in boxes.iter().enumerate() {
            for (position, span) in spans.iter().enumerate() {
                let start = u64::from(span.min[axis] as u32 ^ 0x8000_0000);
                keys.push((start << 32) | ((side as u64) << 31) | position as u64);
            }
        }
        keys.sort_unstable();
        active[0].clear();
        active[1].clear();
        pairs.clear();
        for &key in keys.iter() {
            let side = ((key >> 31) & 1) as usize;
            let position = (key & 0x7fff_ffff) as u32;
            let entry = &boxes[side][position as usize];
            let spans = &boxes[1 - side];
            let others = &mut active[1 - side];
            let mut at = 0;
            while at < others.len() {
                let seen = &spans[others[at] as usize];
                if seen.max[axis] <= entry.min[axis] {
                    others.swap_remove(at);
                    continue;
                }
                if entry.overlaps(seen) {
                    let (a, b) = if side == 0 {
                        (entry.index, seen.index)
                    } else {
                        (seen.index, entry.index)
                    };
                    pairs.push((u64::from(a) << 32) | u64::from(b));
                }
                at += 1;
            }
            active[side].push(position);
        }
        pairs.sort_unstable();
        hits.extend(pairs.iter().map(|key| Hit {
            a: (key >> 32) as u32,
            b: *key as u32,
        }));
    }

    fn sweep_axis(&self) -> usize {
        let mut low = [i32::MAX; N];
        let mut high = [i32::MIN; N];
        for span in self.boxes.iter().flatten() {
            for k in 0..N {
                low[k] = low[k].min(span.min[k]);
                high[k] = high[k].max(span.min[k]);
            }
        }
        (0..N)
            .rev()
            .max_by_key(|&k| i64::from(high[k]) - i64::from(low[k]))
            .unwrap_or(0)
    }
}
