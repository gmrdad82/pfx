use crate::User;

pub const DETAILS_MAX: usize = 64;
pub const RUN_MARK: i32 = 0x5255_4e31;
pub const RUN_FIELDS: usize = 3;
const RANK_MAX: u32 = i32::MAX as u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Board(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ugc(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    Numeric,
    Seconds,
    Milliseconds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upload {
    KeepBest,
    Force,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    Global { first: u32, last: u32 },
    AroundUser { before: u32, after: u32 },
    Friends,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardInfo {
    pub board: Board,
    pub name: String,
    pub sort: Sort,
    pub display: Display,
    pub entries: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub user: User,
    pub rank: u32,
    pub score: i32,
    pub details: Vec<i32>,
    pub run: Option<Ugc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Uploaded {
    pub score: i32,
    pub changed: bool,
    pub rank: u32,
    pub previous_rank: u32,
}

impl Sort {
    pub fn better(self, score: i32, than: i32) -> bool {
        match self {
            Sort::Ascending => score < than,
            Sort::Descending => score > than,
        }
    }
}

impl Range {
    pub(crate) fn valid(self) -> bool {
        match self {
            Range::Global { first, last } => first >= 1 && last >= first && last <= RANK_MAX,
            Range::AroundUser { before, after } => before <= RANK_MAX && after <= RANK_MAX,
            Range::Friends => true,
        }
    }
}

pub fn run_details(run: Ugc, details: &[i32]) -> Vec<i32> {
    let mut out = Vec::with_capacity(RUN_FIELDS + details.len());
    out.push(RUN_MARK);
    out.push(run.0 as u32 as i32);
    out.push((run.0 >> 32) as u32 as i32);
    out.extend_from_slice(details);
    out
}

pub fn split_run(details: &[i32], keep: usize) -> (Option<Ugc>, Vec<i32>) {
    match details {
        [RUN_MARK, low, high, rest @ ..] => {
            let run = u64::from(*low as u32) | (u64::from(*high as u32) << 32);
            (Some(Ugc(run)), rest.iter().take(keep).copied().collect())
        }
        _ => (None, details.iter().take(keep).copied().collect()),
    }
}
