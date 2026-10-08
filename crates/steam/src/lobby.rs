use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::hash::draw;
use crate::{Call, Error, SteamApi, lobby_key, lobby_value};

pub const LOBBY_KEY_MAX: usize = 255;
pub const LOBBY_VALUE_MAX: usize = 8192;
pub const LOBBY_MEMBERS_MAX: u32 = 250;
pub const LOBBY_RESULTS: u32 = 50;
pub const LOBBY_CODE_KEY: &str = "code";
pub const LOBBY_CODE_LEN: usize = 6;
const CODE_LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lobby(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyKind {
    Private,
    FriendsOnly,
    Public,
    Invisible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Entered,
    Left,
    Disconnected,
    Kicked,
    Banned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distance {
    Close,
    Default,
    Far,
    Worldwide,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LobbyFilter {
    pub strings: Vec<(String, String, Compare)>,
    pub numbers: Vec<(String, i32, Compare)>,
    pub near: Vec<(String, i32)>,
    pub open_slots: Option<u8>,
    pub distance: Option<Distance>,
    pub count: Option<u32>,
}

impl Compare {
    pub fn holds(self, order: Ordering) -> bool {
        match self {
            Compare::Equal => order == Ordering::Equal,
            Compare::NotEqual => order != Ordering::Equal,
            Compare::Less => order == Ordering::Less,
            Compare::LessOrEqual => order != Ordering::Greater,
            Compare::Greater => order == Ordering::Greater,
            Compare::GreaterOrEqual => order != Ordering::Less,
        }
    }
}

impl LobbyFilter {
    pub fn string(mut self, key: &str, value: &str, compare: Compare) -> Self {
        self.strings
            .push((key.to_string(), value.to_string(), compare));
        self
    }

    pub fn number(mut self, key: &str, value: i32, compare: Compare) -> Self {
        self.numbers.push((key.to_string(), value, compare));
        self
    }

    pub fn near(mut self, key: &str, value: i32) -> Self {
        self.near.push((key.to_string(), value));
        self
    }

    pub fn open_slots(mut self, slots: u8) -> Self {
        self.open_slots = Some(slots);
        self
    }

    pub fn distance(mut self, distance: Distance) -> Self {
        self.distance = Some(distance);
        self
    }

    pub fn count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        for (key, value, _) in &self.strings {
            lobby_key(key)?;
            lobby_value(value)?;
        }
        for (key, _, _) in &self.numbers {
            lobby_key(key)?;
        }
        for (key, _) in &self.near {
            lobby_key(key)?;
        }
        Ok(())
    }

    pub(crate) fn matches(&self, data: &BTreeMap<String, String>, open: u32) -> bool {
        if let Some(slots) = self.open_slots
            && open < u32::from(slots)
        {
            return false;
        }
        let strings = self.strings.iter().all(|(key, value, compare)| {
            data.get(key)
                .is_some_and(|have| compare.holds(have.as_str().cmp(value.as_str())))
        });
        let numbers = self.numbers.iter().all(|(key, value, compare)| {
            data.get(key)
                .and_then(|have| have.parse::<i32>().ok())
                .is_some_and(|have| compare.holds(have.cmp(value)))
        });
        strings && numbers
    }

    pub(crate) fn nearness(&self, data: &BTreeMap<String, String>) -> Vec<u64> {
        self.near
            .iter()
            .map(|(key, value)| {
                data.get(key)
                    .and_then(|have| have.parse::<i32>().ok())
                    .map(|have| i64::from(have).abs_diff(i64::from(*value)))
                    .unwrap_or(u64::MAX)
            })
            .collect()
    }

    pub(crate) fn limit(&self) -> usize {
        self.count.unwrap_or(LOBBY_RESULTS) as usize
    }
}

pub fn lobby_code(seed: u64) -> String {
    let mut state = seed;
    (0..LOBBY_CODE_LEN)
        .map(|_| {
            let pick = draw(&mut state) % CODE_LETTERS.len() as u64;
            CODE_LETTERS[pick as usize] as char
        })
        .collect()
}

pub fn normal_code(code: &str) -> String {
    code.chars()
        .filter(|letter| !letter.is_whitespace() && *letter != '-')
        .map(|letter| letter.to_ascii_uppercase())
        .collect()
}

pub fn set_lobby_code(steam: &mut impl SteamApi, lobby: Lobby, code: &str) -> Result<(), Error> {
    let code = normal_code(code);
    if code.is_empty() {
        return Err(Error::Rejected);
    }
    steam.set_lobby_data(lobby, LOBBY_CODE_KEY, &code)
}

pub fn find_lobby_by_code(steam: &mut impl SteamApi, code: &str) -> Result<Call, Error> {
    let code = normal_code(code);
    if code.is_empty() {
        return Err(Error::Rejected);
    }
    steam.find_lobbies(
        &LobbyFilter::default()
            .string(LOBBY_CODE_KEY, &code, Compare::Equal)
            .distance(Distance::Worldwide)
            .count(1),
    )
}
