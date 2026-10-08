use std::fmt;

use crate::lockstep::checksum;
use crate::{Board, Call, Display, Entry, Error, Range, Sort, SteamApi, Upload, User};

const DAY: u64 = 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: u64,
    month: u8,
    day: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    Server,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Today {
    pub date: Date,
    pub clock: Clock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    FirstOnly,
    Best,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Daily {
    pub salt: String,
    pub prefix: String,
    pub repeat: Repeat,
    pub sort: Sort,
    pub display: Display,
}

impl Date {
    pub fn new(year: u64, month: u8, day: u8) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    pub fn from_unix(seconds: u64) -> Self {
        let z = seconds / DAY + 719_468;
        let era = z / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + u64::from(month <= 2);
        Self {
            year,
            month: month as u8,
            day: day as u8,
        }
    }

    pub fn year(self) -> u64 {
        self.year
    }

    pub fn month(self) -> u8 {
        self.month
    }

    pub fn day(self) -> u8 {
        self.day
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn days_in(year: u64, month: u8) -> u8 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

pub fn daily_seed(salt: &str, date: Date) -> u64 {
    let mut bytes = Vec::with_capacity(salt.len() + 11);
    bytes.extend_from_slice(salt.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(date.to_string().as_bytes());
    checksum(&bytes)
}

impl Today {
    pub fn pick(server: Option<u64>, system: u64) -> Self {
        match server {
            Some(seconds) => Self {
                date: Date::from_unix(seconds),
                clock: Clock::Server,
            },
            None => Self {
                date: Date::from_unix(system),
                clock: Clock::System,
            },
        }
    }
}

impl Daily {
    pub fn new(salt: &str, prefix: &str, repeat: Repeat, sort: Sort, display: Display) -> Self {
        Self {
            salt: salt.to_string(),
            prefix: prefix.to_string(),
            repeat,
            sort,
            display,
        }
    }

    pub fn today(&self, steam: &impl SteamApi, system: u64) -> Today {
        Today::pick(steam.server_time().ok().flatten(), system)
    }

    pub fn seed(&self, date: Date) -> u64 {
        daily_seed(&self.salt, date)
    }

    pub fn board_name(&self, date: Date) -> String {
        format!("{}-{date}", self.prefix)
    }

    pub fn open(&self, steam: &mut impl SteamApi, date: Date) -> Result<Call, Error> {
        steam.find_or_create_board(&self.board_name(date), self.sort, self.display)
    }

    pub fn marker(&self) -> i32 {
        match self.sort {
            Sort::Descending => 0,
            Sort::Ascending => i32::MAX,
        }
    }

    pub fn start(&self, steam: &mut impl SteamApi, board: Board) -> Result<Call, Error> {
        steam.upload_score(board, self.marker(), &[], Upload::KeepBest)
    }

    pub fn check(&self, steam: &mut impl SteamApi, board: Board) -> Result<Call, Error> {
        steam.download_scores(
            board,
            Range::AroundUser {
                before: 0,
                after: 0,
            },
            0,
        )
    }

    pub fn used(entries: &[Entry], me: User) -> bool {
        entries.iter().any(|entry| entry.user == me)
    }

    pub fn upload(&self, date: Date, last_counted: Option<Date>) -> Option<Upload> {
        match self.repeat {
            Repeat::FirstOnly if last_counted == Some(date) => None,
            Repeat::FirstOnly | Repeat::Best => Some(Upload::KeepBest),
        }
    }

    pub fn submit(
        &self,
        steam: &mut impl SteamApi,
        board: Board,
        date: Date,
        last_counted: Option<Date>,
        score: i32,
        details: &[i32],
    ) -> Result<Option<Call>, Error> {
        match self.upload(date, last_counted) {
            Some(upload) => steam.upload_score(board, score, details, upload).map(Some),
            None => Ok(None),
        }
    }
}
