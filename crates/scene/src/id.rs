use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sha256::sha256;

pub const ID_LENGTH: usize = 10;
pub const ID_BITS: u32 = 50;

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
const MASK: u64 = (1 << ID_BITS) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(u64);

impl Id {
    pub fn from_bits(bits: u64) -> Self {
        Self(bits & MASK)
    }

    pub fn derive(key: &[u8]) -> Self {
        let digest = sha256(key);
        let mut first = [0; 8];
        first.copy_from_slice(&digest[..8]);
        Self(u64::from_be_bytes(first) >> (64 - ID_BITS))
    }

    pub fn bits(self) -> u64 {
        self.0
    }

    pub fn parse(text: &str) -> Result<Self, IdError> {
        if text.chars().count() != ID_LENGTH {
            return Err(IdError::Length(text.chars().count()));
        }
        let mut bits = 0u64;
        for character in text.chars() {
            let Some(value) = ALPHABET
                .iter()
                .position(|&letter| char::from(letter) == character)
            else {
                return Err(IdError::Character(character));
            };
            bits = (bits << 5) | value as u64;
        }
        Ok(Self(bits))
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = [0u8; ID_LENGTH];
        for (place, slot) in text.iter_mut().enumerate() {
            let shift = 5 * (ID_LENGTH - 1 - place);
            *slot = ALPHABET[((self.0 >> shift) & 31) as usize];
        }
        f.write_str(std::str::from_utf8(&text).unwrap_or_default())
    }
}

impl FromStr for Id {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdError {
    Length(usize),
    Character(char),
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length(count) => write!(
                f,
                "an id is {ID_LENGTH} characters, not {count}: lowercase Crockford base32"
            ),
            Self::Character(character) => write!(
                f,
                "an id takes 0-9 and a-z without i, l, o and u, not {character:?}"
            ),
        }
    }
}

impl std::error::Error for IdError {}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_ten_crockford_characters_and_round_trips() {
        let id = Id::parse("7k2m9q4xzr").unwrap();
        assert_eq!(id.to_string(), "7k2m9q4xzr");
        assert_eq!(Id::from_bits(id.bits()), id);
        assert_eq!(Id::from_bits(0).to_string(), "0000000000");
        assert_eq!(Id::from_bits(u64::MAX).to_string(), "zzzzzzzzzz");
        assert_eq!(Id::from_bits(u64::MAX).bits(), (1 << 50) - 1);
        assert_eq!(Id::from_bits(31).to_string(), "000000000z");
    }

    #[test]
    fn a_bad_id_is_refused() {
        assert_eq!(Id::parse("7k2m9q4xz"), Err(IdError::Length(9)));
        assert_eq!(Id::parse("7k2m9q4xzrr"), Err(IdError::Length(11)));
        assert_eq!(Id::parse("7k2m9q4xzi"), Err(IdError::Character('i')));
        assert_eq!(Id::parse("7K2M9Q4XZR"), Err(IdError::Character('K')));
        assert!(Id::parse("7k2m9q4xzu").is_err());
        assert!(Id::parse("7k2m9q4xzo").is_err());
        assert!(Id::parse("7k2m9q4xzl").is_err());
    }

    #[test]
    fn a_derived_id_is_the_first_fifty_bits_of_the_sha256() {
        let digest = sha256(b"abc");
        let expected = u64::from_be_bytes(digest[..8].try_into().unwrap()) >> 14;
        assert_eq!(Id::derive(b"abc").bits(), expected);
        assert_eq!(Id::derive(b"abc").to_string(), "q9w1dfwf07");
        assert_eq!(Id::derive(b"abc"), Id::derive(b"abc"));
        assert_ne!(Id::derive(b"abc"), Id::derive(b"abd"));
    }
}
