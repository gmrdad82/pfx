use std::fmt;

use super::rng::{Rng, STATE_BYTES, mix64};

pub const ONE: i32 = 1 << 16;
pub const UNCAPPED: u32 = u32::MAX;
pub const NO_SPOT: u32 = u32::MAX;
pub const MAX_ORDERS_PER_TICK: usize = 16;
pub const DIRECTOR_VERSION: u32 = 2;

pub const TIMING: u64 = 1;
pub const PICKS: u64 = 2;
pub const POSITIONS: u64 = 3;
pub const FORMS: u64 = 4;
pub const SPREADS: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Linear,
    Stepped(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ramp {
    pub from: i32,
    pub to: i32,
    pub shape: Shape,
}

impl Ramp {
    pub const fn flat(value: i32) -> Self {
        Self {
            from: value,
            to: value,
            shape: Shape::Linear,
        }
    }

    pub const fn linear(from: i32, to: i32) -> Self {
        Self {
            from,
            to,
            shape: Shape::Linear,
        }
    }

    pub const fn stepped(from: i32, to: i32, steps: u32) -> Self {
        Self {
            from,
            to,
            shape: Shape::Stepped(steps),
        }
    }

    #[must_use]
    pub fn at(&self, elapsed: u64, duration: u64) -> i32 {
        let duration = duration.max(1);
        let elapsed = elapsed.min(duration);
        let span = self.to as i128 - self.from as i128;
        let value = match self.shape {
            Shape::Linear => self.from as i128 + span * elapsed as i128 / duration as i128,
            Shape::Stepped(steps) => {
                let steps = steps.max(2) as u128;
                let level = (elapsed as u128 * steps / duration as u128).min(steps - 1);
                self.from as i128 + span * level as i128 / (steps as i128 - 1)
            }
        };
        value as i32
    }

    fn top(&self) -> i32 {
        self.from.max(self.to)
    }

    fn lowest(&self) -> i32 {
        self.from.min(self.to)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lull {
    pub min: u32,
    pub max: u32,
}

impl Lull {
    pub const fn fixed(ticks: u32) -> Self {
        Self {
            min: ticks,
            max: ticks,
        }
    }

    pub const fn between(min: u32, max: u32) -> Self {
        Self { min, max }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnLull {
    pub spawns: u32,
    pub lull: Lull,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub weight: u32,
    pub tags: u32,
    pub cost: u32,
    pub from: u64,
    pub line: Option<Line>,
}

impl Entry {
    pub const fn new(id: u32, weight: u32) -> Self {
        Self {
            id,
            weight,
            tags: 0,
            cost: 1,
            from: 0,
            line: None,
        }
    }

    pub const fn tagged(mut self, tags: u32) -> Self {
        self.tags = tags;
        self
    }

    pub const fn costing(mut self, cost: u32) -> Self {
        self.cost = cost;
        self
    }

    pub const fn from_tick(mut self, tick: u64) -> Self {
        self.from = tick;
        self
    }

    pub const fn in_line(mut self, line: Line) -> Self {
        self.line = Some(line);
        self
    }

    const fn least_members(&self) -> u32 {
        match self.line {
            Some(line) => line.min,
            None => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Centre,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub min: u32,
    pub max: u32,
    pub direction: [i32; 3],
    pub gap: i32,
    pub spread: [i32; 3],
    pub anchor: Anchor,
}

impl Line {
    pub const fn new(min: u32, max: u32, direction: [i32; 3], gap: i32) -> Self {
        Self {
            min,
            max,
            direction,
            gap,
            spread: [0; 3],
            anchor: Anchor::Start,
        }
    }

    pub const fn spread(mut self, spread: [i32; 3]) -> Self {
        self.spread = spread;
        self
    }

    pub const fn anchored(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    fn step(&self, axis: usize) -> i64 {
        self.direction[axis] as i64 * self.gap as i64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub entries: Vec<Entry>,
    pub cap: u32,
}

impl Table {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            cap: UNCAPPED,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub start: u64,
    pub end: u64,
    pub table: u32,
    pub tag_filter: u32,
    pub rate: Ramp,
    pub group: Ramp,
    pub speed: Ramp,
    pub after_spawns: Option<SpawnLull>,
    pub on_clear: Option<Lull>,
}

impl Phase {
    pub const fn new(start: u64, end: u64, table: u32) -> Self {
        Self {
            start,
            end,
            table,
            tag_filter: 0,
            rate: Ramp::flat(0),
            group: Ramp::flat(ONE),
            speed: Ramp::flat(ONE),
            after_spawns: None,
            on_clear: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub tick_rate: u32,
    pub cap: u32,
    pub allow_overlap: bool,
    pub tables: Vec<Table>,
    pub phases: Vec<Phase>,
    pub lulls: Vec<Window>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            tick_rate: 60,
            cap: UNCAPPED,
            allow_overlap: false,
            tables: Vec::new(),
            phases: Vec::new(),
            lulls: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectorError {
    TickRate,
    NoTables,
    ZeroWeight { table: u32 },
    DuplicateEntry { id: u32 },
    BadPhase { phase: u32 },
    UnknownTable { phase: u32, table: u32 },
    Overlap { first: u32, second: u32 },
    ZeroCap { phase: u32 },
    Unspawnable { phase: u32 },
    BadRamp { phase: u32 },
    BadLull { phase: u32 },
    BadLine { id: u32 },
    BadWindow { window: u32 },
    Length { found: usize, expected: usize },
    Version { found: u32 },
    Mismatch,
    Corrupt,
}

impl fmt::Display for DirectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TickRate => write!(f, "the schedule's tick rate is zero"),
            Self::NoTables => write!(f, "the schedule has phases but no tables"),
            Self::ZeroWeight { table } => {
                write!(f, "table {table} has no entry with a weight above zero")
            }
            Self::DuplicateEntry { id } => {
                write!(f, "entry id {id} appears more than once across the tables")
            }
            Self::BadPhase { phase } => write!(f, "phase {phase} ends at or before its start"),
            Self::UnknownTable { phase, table } => {
                write!(
                    f,
                    "phase {phase} draws from table {table}, which does not exist"
                )
            }
            Self::Overlap { first, second } => write!(
                f,
                "phases {first} and {second} overlap and the schedule does not allow it"
            ),
            Self::ZeroCap { phase } => write!(
                f,
                "phase {phase} schedules spawns but its table or the overall cap is zero"
            ),
            Self::Unspawnable { phase } => write!(
                f,
                "phase {phase} schedules spawns but no entry of its table passes its tag filter and fits the cap"
            ),
            Self::BadRamp { phase } => write!(
                f,
                "phase {phase} has a ramp with a negative value or fewer than two steps"
            ),
            Self::BadLull { phase } => write!(
                f,
                "phase {phase} has a lull whose minimum exceeds its maximum"
            ),
            Self::BadLine { id } => write!(
                f,
                "entry {id} has a line with no members, a minimum above its maximum or a negative spread"
            ),
            Self::BadWindow { window } => {
                write!(f, "quiet window {window} ends at or before its start")
            }
            Self::Length { found, expected } => {
                write!(
                    f,
                    "saved director state is {found} bytes, expected {expected}"
                )
            }
            Self::Version { found } => write!(
                f,
                "saved director state is version {found}, this build reads {DIRECTOR_VERSION}"
            ),
            Self::Mismatch => write!(f, "saved director state belongs to a different schedule"),
            Self::Corrupt => write!(f, "saved director state holds an invalid generator state"),
        }
    }
}

impl std::error::Error for DirectorError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl Region {
    pub const fn new(min: [i32; 3], max: [i32; 3]) -> Self {
        Self { min, max }
    }

    pub const fn flat(min: [i32; 2], max: [i32; 2]) -> Self {
        Self {
            min: [min[0], min[1], 0],
            max: [max[0], max[1], 0],
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Spots<'a> {
    pub points: &'a [[i32; 3]],
    pub regions: &'a [Region],
}

impl<'a> Spots<'a> {
    pub const EMPTY: Spots<'static> = Spots {
        points: &[],
        regions: &[],
    };

    pub const fn points(points: &'a [[i32; 3]]) -> Self {
        Self {
            points,
            regions: &[],
        }
    }

    pub const fn regions(regions: &'a [Region]) -> Self {
        Self {
            points: &[],
            regions,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnOrder {
    pub entry: u32,
    pub count: u32,
    pub speed: i32,
    pub position: [i32; 3],
    pub spot: u32,
    pub group: u32,
    pub member: u32,
    pub members: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PhaseState {
    charge: u64,
    spawned: u32,
    wave_open: bool,
    lull_until: u64,
}

const PHASE_BYTES: usize = 8 + 4 + 1 + 8;

#[derive(Clone, Debug)]
pub struct Director {
    schedule: Schedule,
    fingerprint: u64,
    ids: Vec<(u32, u32, u32)>,
    bases: Vec<u32>,
    now: u64,
    states: Vec<PhaseState>,
    alive: Vec<u32>,
    table_load: Vec<u64>,
    load: u64,
    timing: Rng,
    picks: Rng,
    positions: Rng,
    forms: Rng,
    spreads: Rng,
    next_group: u32,
}

impl Schedule {
    pub fn check(&self) -> Result<(), DirectorError> {
        if self.tick_rate == 0 {
            return Err(DirectorError::TickRate);
        }
        if !self.phases.is_empty() && self.tables.is_empty() {
            return Err(DirectorError::NoTables);
        }
        let mut ids: Vec<u32> = Vec::new();
        for (t, table) in self.tables.iter().enumerate() {
            let total: u64 = table.entries.iter().map(|e| e.weight as u64).sum();
            if total == 0 {
                return Err(DirectorError::ZeroWeight { table: t as u32 });
            }
            for e in &table.entries {
                if let Some(line) = e.line
                    && (line.min == 0 || line.min > line.max || line.spread.iter().any(|&v| v < 0))
                {
                    return Err(DirectorError::BadLine { id: e.id });
                }
            }
            ids.extend(table.entries.iter().map(|e| e.id));
        }
        ids.sort_unstable();
        if let Some(pair) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(DirectorError::DuplicateEntry { id: pair[0] });
        }
        for (window, w) in self.lulls.iter().enumerate() {
            if w.end <= w.start {
                return Err(DirectorError::BadWindow {
                    window: window as u32,
                });
            }
        }
        for (p, phase) in self.phases.iter().enumerate() {
            self.check_phase(p as u32, phase)?;
        }
        if !self.allow_overlap {
            for (a, first) in self.phases.iter().enumerate() {
                for (b, second) in self.phases.iter().enumerate().skip(a + 1) {
                    if first.start < second.end && second.start < first.end {
                        return Err(DirectorError::Overlap {
                            first: a as u32,
                            second: b as u32,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn check_phase(&self, p: u32, phase: &Phase) -> Result<(), DirectorError> {
        if phase.end <= phase.start {
            return Err(DirectorError::BadPhase { phase: p });
        }
        let Some(table) = self.tables.get(phase.table as usize) else {
            return Err(DirectorError::UnknownTable {
                phase: p,
                table: phase.table,
            });
        };
        for ramp in [&phase.rate, &phase.group, &phase.speed] {
            let steps_ok = match ramp.shape {
                Shape::Linear => true,
                Shape::Stepped(steps) => steps >= 2,
            };
            if ramp.lowest() < 0 || !steps_ok {
                return Err(DirectorError::BadRamp { phase: p });
            }
        }
        let lulls = phase
            .after_spawns
            .map(|s| s.lull)
            .into_iter()
            .chain(phase.on_clear);
        for lull in lulls {
            if lull.min > lull.max {
                return Err(DirectorError::BadLull { phase: p });
            }
        }
        if phase.rate.top() == 0 {
            return Ok(());
        }
        if table.cap == 0 || self.cap == 0 {
            return Err(DirectorError::ZeroCap { phase: p });
        }
        let room = table.cap.min(self.cap);
        let fits = table.entries.iter().any(|e| {
            e.weight > 0
                && (phase.tag_filter == 0 || e.tags & phase.tag_filter != 0)
                && e.from < phase.end
                && e.cost as u64 * e.least_members() as u64 <= room as u64
        });
        if !fits {
            return Err(DirectorError::Unspawnable { phase: p });
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        let mut hash = mix64(DIRECTOR_VERSION as u64);
        let mut put = |value: u64| hash = mix64(hash ^ value).wrapping_add(value);
        put(self.tick_rate as u64);
        put(self.cap as u64);
        put(self.allow_overlap as u64);
        put(self.tables.len() as u64);
        for table in &self.tables {
            put(table.cap as u64);
            put(table.entries.len() as u64);
            for e in &table.entries {
                put(e.id as u64);
                put(e.weight as u64);
                put(e.tags as u64);
                put(e.cost as u64);
                put(e.from);
                match e.line {
                    Some(l) => {
                        put(1 + l.min as u64);
                        put(l.max as u64);
                        for axis in 0..3 {
                            put(l.direction[axis] as u32 as u64);
                            put(l.spread[axis] as u32 as u64);
                        }
                        put(l.gap as u32 as u64);
                        put(l.anchor as u64);
                    }
                    None => put(0),
                }
            }
        }
        put(self.phases.len() as u64);
        for phase in &self.phases {
            put(phase.start);
            put(phase.end);
            put(phase.table as u64);
            put(phase.tag_filter as u64);
            for ramp in [&phase.rate, &phase.group, &phase.speed] {
                put(ramp.from as u32 as u64);
                put(ramp.to as u32 as u64);
                put(match ramp.shape {
                    Shape::Linear => 0,
                    Shape::Stepped(n) => 1 + n as u64,
                });
            }
            match phase.after_spawns {
                Some(s) => {
                    put(1 + s.spawns as u64);
                    put(s.lull.min as u64);
                    put(s.lull.max as u64);
                }
                None => put(0),
            }
            match phase.on_clear {
                Some(l) => {
                    put(1 + l.min as u64);
                    put(l.max as u64);
                }
                None => put(0),
            }
        }
        put(self.lulls.len() as u64);
        for w in &self.lulls {
            put(w.start);
            put(w.end);
        }
        hash
    }
}

impl Director {
    pub fn new(schedule: Schedule, seed: u64) -> Result<Self, DirectorError> {
        schedule.check()?;
        let root = Rng::new(seed);
        let mut ids = Vec::new();
        let mut bases = Vec::with_capacity(schedule.tables.len());
        let mut flat = 0u32;
        for (t, table) in schedule.tables.iter().enumerate() {
            bases.push(flat);
            for (i, e) in table.entries.iter().enumerate() {
                ids.push((e.id, flat + i as u32, t as u32));
            }
            flat += table.entries.len() as u32;
        }
        ids.sort_unstable();
        Ok(Self {
            fingerprint: schedule.fingerprint(),
            ids,
            bases,
            now: 0,
            states: vec![PhaseState::default(); schedule.phases.len()],
            alive: vec![0; flat as usize],
            table_load: vec![0; schedule.tables.len()],
            load: 0,
            timing: root.fork(TIMING),
            picks: root.fork(PICKS),
            positions: root.fork(POSITIONS),
            forms: root.fork(FORMS),
            spreads: root.fork(SPREADS),
            next_group: 0,
            schedule,
        })
    }

    #[must_use]
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    #[must_use]
    pub fn now(&self) -> u64 {
        self.now
    }

    #[must_use]
    pub fn load(&self) -> u64 {
        self.load
    }

    #[must_use]
    pub fn table_load(&self, table: u32) -> u64 {
        self.table_load.get(table as usize).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn alive(&self, id: u32) -> u32 {
        self.slot(id)
            .map_or(0, |(flat, _)| self.alive[flat as usize])
    }

    #[must_use]
    pub fn is_quiet(&self) -> bool {
        self.in_window(self.now)
    }

    fn slot(&self, id: u32) -> Option<(u32, u32)> {
        self.ids
            .binary_search_by_key(&id, |s| s.0)
            .ok()
            .map(|i| (self.ids[i].1, self.ids[i].2))
    }

    fn in_window(&self, now: u64) -> bool {
        self.schedule
            .lulls
            .iter()
            .any(|w| w.start <= now && now < w.end)
    }

    pub fn died(&mut self, id: u32, count: u32) -> u32 {
        let Some((flat, table)) = self.slot(id) else {
            return 0;
        };
        let removed = count.min(self.alive[flat as usize]);
        self.alive[flat as usize] -= removed;
        let cost = self.schedule.tables[table as usize].entries
            [(flat - self.bases[table as usize]) as usize]
            .cost as u64;
        let freed = removed as u64 * cost;
        self.table_load[table as usize] -= freed;
        self.load -= freed;
        removed
    }

    fn draw_lull(&mut self, lull: Lull) -> u64 {
        if lull.max <= lull.min {
            lull.min as u64
        } else {
            self.timing.range_u64(lull.min as u64, lull.max as u64 + 1)
        }
    }

    pub fn tick(&mut self, spots: &Spots, out: &mut Vec<SpawnOrder>) {
        out.clear();
        let now = self.now;
        self.now += 1;
        let quiet = self.in_window(now);
        for p in 0..self.schedule.phases.len() {
            let phase = &self.schedule.phases[p];
            if now < phase.start || now >= phase.end {
                self.states[p] = PhaseState::default();
                continue;
            }
            if quiet {
                continue;
            }
            let (start, end, table) = (phase.start, phase.end, phase.table as usize);
            let (tag_filter, on_clear, after) =
                (phase.tag_filter, phase.on_clear, phase.after_spawns);
            let (rate, group, speed) = (phase.rate, phase.group, phase.speed);
            if let Some(lull) = on_clear
                && self.load == 0
                && self.states[p].wave_open
            {
                self.states[p].wave_open = false;
                self.states[p].spawned = 0;
                let ticks = self.draw_lull(lull);
                self.states[p].lull_until = now + ticks;
            }
            if now < self.states[p].lull_until {
                continue;
            }
            let duration = end - start;
            let elapsed = now - start;
            let rate_now = rate.at(elapsed, duration).max(0) as u64;
            let threshold = 60 * self.schedule.tick_rate as u64 * ONE as u64;
            self.states[p].charge += rate_now;
            let mut fired = 0;
            while self.states[p].charge >= threshold && fired < MAX_ORDERS_PER_TICK {
                let group_now = (group.at(elapsed, duration) >> 16).max(1) as u32;
                let speed_now = speed.at(elapsed, duration).max(0);
                let Some(members) =
                    self.build(now, table, tag_filter, group_now, speed_now, spots, out)
                else {
                    break;
                };
                self.states[p].charge -= threshold;
                self.states[p].wave_open = true;
                self.states[p].spawned = self.states[p].spawned.saturating_add(members);
                fired += 1;
                if let Some(after) = after
                    && after.spawns > 0
                    && self.states[p].spawned >= after.spawns
                {
                    self.states[p].spawned = 0;
                    let ticks = self.draw_lull(after.lull);
                    self.states[p].lull_until = now + 1 + ticks;
                    break;
                }
            }
            self.states[p].charge = self.states[p].charge.min(threshold);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        &mut self,
        now: u64,
        table: usize,
        tag_filter: u32,
        group: u32,
        speed: i32,
        spots: &Spots,
        out: &mut Vec<SpawnOrder>,
    ) -> Option<u32> {
        let tbl = &self.schedule.tables[table];
        let room_table = (tbl.cap as u64).saturating_sub(self.table_load[table]);
        let room_all = (self.schedule.cap as u64).saturating_sub(self.load);
        let room = room_table.min(room_all);
        let eligible = |e: &Entry| {
            e.weight > 0
                && now >= e.from
                && (tag_filter == 0 || e.tags & tag_filter != 0)
                && e.cost as u64 * e.least_members() as u64 <= room
        };
        let total: u64 = tbl
            .entries
            .iter()
            .filter(|e| eligible(e))
            .map(|e| e.weight as u64)
            .sum();
        if total == 0 {
            return None;
        }
        let mut target = self.picks.below(total);
        let mut chosen = None;
        for (i, e) in tbl.entries.iter().enumerate() {
            if !eligible(e) {
                continue;
            }
            if target < e.weight as u64 {
                chosen = Some((i, *e));
                break;
            }
            target -= e.weight as u64;
        }
        let (index, entry) = chosen?;
        let fit = if entry.cost == 0 {
            u32::MAX
        } else {
            (room / entry.cost as u64).min(u32::MAX as u64) as u32
        };
        let (position, spot) = self.pick_position(spots);
        let id = self.next_group;
        self.next_group = self.next_group.wrapping_add(1);
        let members = match entry.line {
            None => group.min(fit),
            Some(line) => {
                let wanted = if line.max > line.min {
                    self.forms.range_u64(line.min as u64, line.max as u64 + 1) as u32
                } else {
                    line.min
                };
                wanted.min(fit)
            }
        };
        let cost = members as u64 * entry.cost as u64;
        self.alive[self.bases[table] as usize + index] += members;
        self.table_load[table] += cost;
        self.load += cost;
        match entry.line {
            None => out.push(SpawnOrder {
                entry: entry.id,
                count: members,
                speed,
                position,
                spot,
                group: id,
                member: 0,
                members,
            }),
            Some(line) => {
                for member in 0..members {
                    let at = self.place(&line, position, member, members);
                    out.push(SpawnOrder {
                        entry: entry.id,
                        count: 1,
                        speed,
                        position: at,
                        spot,
                        group: id,
                        member,
                        members,
                    });
                }
            }
        }
        Some(members)
    }

    fn place(&mut self, line: &Line, base: [i32; 3], member: u32, members: u32) -> [i32; 3] {
        let mut at = base;
        for (axis, slot) in at.iter_mut().enumerate() {
            let step = line.step(axis);
            let along = match line.anchor {
                Anchor::Start => step * member as i64,
                Anchor::End => step * (member as i64 - (members as i64 - 1)),
                Anchor::Centre => step * (2 * member as i64 - (members as i64 - 1)) / 2,
            };
            let jitter = if line.spread[axis] > 0 {
                self.spreads
                    .range_i64(-(line.spread[axis] as i64), line.spread[axis] as i64 + 1)
            } else {
                0
            };
            *slot =
                (base[axis] as i64 + along + jitter).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        }
        at
    }

    fn pick_position(&mut self, spots: &Spots) -> ([i32; 3], u32) {
        let total = spots.points.len() + spots.regions.len();
        if total == 0 {
            return ([0; 3], NO_SPOT);
        }
        let pick = self.positions.index(total);
        if pick < spots.points.len() {
            return (spots.points[pick], pick as u32);
        }
        let region = spots.regions[pick - spots.points.len()];
        let mut at = [0; 3];
        for (axis, slot) in at.iter_mut().enumerate() {
            *slot = if region.max[axis] > region.min[axis] {
                self.positions.range_i32(region.min[axis], region.max[axis])
            } else {
                region.min[axis]
            };
        }
        (at, pick as u32)
    }

    fn state_len(&self) -> usize {
        4 + 8 + 8 + 4 + self.states.len() * PHASE_BYTES + self.alive.len() * 4 + 5 * STATE_BYTES
    }

    #[must_use]
    pub fn save(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.state_len());
        out.extend_from_slice(&DIRECTOR_VERSION.to_le_bytes());
        out.extend_from_slice(&self.fingerprint.to_le_bytes());
        out.extend_from_slice(&self.now.to_le_bytes());
        out.extend_from_slice(&self.next_group.to_le_bytes());
        for s in &self.states {
            out.extend_from_slice(&s.charge.to_le_bytes());
            out.extend_from_slice(&s.spawned.to_le_bytes());
            out.push(s.wave_open as u8);
            out.extend_from_slice(&s.lull_until.to_le_bytes());
        }
        for a in &self.alive {
            out.extend_from_slice(&a.to_le_bytes());
        }
        out.extend_from_slice(&self.timing.save());
        out.extend_from_slice(&self.picks.save());
        out.extend_from_slice(&self.positions.save());
        out.extend_from_slice(&self.forms.save());
        out.extend_from_slice(&self.spreads.save());
        out
    }

    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), DirectorError> {
        let expected = self.state_len();
        if bytes.len() != expected {
            return Err(DirectorError::Length {
                found: bytes.len(),
                expected,
            });
        }
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let version = u32_at(0);
        if version != DIRECTOR_VERSION {
            return Err(DirectorError::Version { found: version });
        }
        if u64_at(4) != self.fingerprint {
            return Err(DirectorError::Mismatch);
        }
        let mut at = 24;
        let mut states = Vec::with_capacity(self.states.len());
        for _ in 0..self.states.len() {
            states.push(PhaseState {
                charge: u64_at(at),
                spawned: u32_at(at + 8),
                wave_open: bytes[at + 12] != 0,
                lull_until: u64_at(at + 13),
            });
            at += PHASE_BYTES;
        }
        let mut alive = Vec::with_capacity(self.alive.len());
        for _ in 0..self.alive.len() {
            alive.push(u32_at(at));
            at += 4;
        }
        let rng = |at: usize| {
            Rng::restore(&bytes[at..at + STATE_BYTES]).map_err(|_| DirectorError::Corrupt)
        };
        let timing = rng(at)?;
        let picks = rng(at + STATE_BYTES)?;
        let positions = rng(at + 2 * STATE_BYTES)?;
        let forms = rng(at + 3 * STATE_BYTES)?;
        let spreads = rng(at + 4 * STATE_BYTES)?;
        self.now = u64_at(12);
        self.next_group = u32_at(20);
        self.states = states;
        self.alive = alive;
        self.timing = timing;
        self.picks = picks;
        self.positions = positions;
        self.forms = forms;
        self.spreads = spreads;
        self.table_load.iter_mut().for_each(|l| *l = 0);
        self.load = 0;
        for (t, table) in self.schedule.tables.iter().enumerate() {
            let base = self.bases[t] as usize;
            for (i, e) in table.entries.iter().enumerate() {
                let cost = self.alive[base + i] as u64 * e.cost as u64;
                self.table_load[t] += cost;
                self.load += cost;
            }
        }
        Ok(())
    }
}
