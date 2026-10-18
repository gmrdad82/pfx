use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyShape {
    Number,
    Pair,
    Triple,
}

impl KeyShape {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Number => "a number",
            Self::Pair => "two numbers",
            Self::Triple => "three numbers",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyValue {
    Number(f32),
    Pair([f32; 2]),
    Triple([f32; 3]),
}

impl KeyValue {
    pub fn shape(&self) -> KeyShape {
        match self {
            Self::Number(_) => KeyShape::Number,
            Self::Pair(_) => KeyShape::Pair,
            Self::Triple(_) => KeyShape::Triple,
        }
    }

    pub fn parts(&self) -> &[f32] {
        match self {
            Self::Number(value) => std::slice::from_ref(value),
            Self::Pair(value) => value,
            Self::Triple(value) => value,
        }
    }

    pub fn number(&self) -> Option<f32> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn pair(&self) -> Option<[f32; 2]> {
        match self {
            Self::Pair(value) => Some(*value),
            _ => None,
        }
    }

    pub fn triple(&self) -> Option<[f32; 3]> {
        match self {
            Self::Triple(value) => Some(*value),
            _ => None,
        }
    }

    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => Self::Number(mix(*a, *b)),
            (Self::Pair(a), Self::Pair(b)) => Self::Pair([mix(a[0], b[0]), mix(a[1], b[1])]),
            (Self::Triple(a), Self::Triple(b)) => {
                Self::Triple([mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])])
            }
            _ => *self,
        }
    }
}

impl Serialize for KeyValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Number(value) => value.serialize(serializer),
            Self::Pair(value) => value.serialize(serializer),
            Self::Triple(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for KeyValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Pick;

        const EXPECTED: &str = "a number, two numbers or three numbers";

        impl<'de> Visitor<'de> for Pick {
            type Value = KeyValue;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(EXPECTED)
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<KeyValue, E> {
                Ok(KeyValue::Number(value as f32))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<KeyValue, E> {
                Ok(KeyValue::Number(value as f32))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<KeyValue, E> {
                Ok(KeyValue::Number(value as f32))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<KeyValue, A::Error> {
                let mut parts = Vec::new();
                while let Some(part) = seq.next_element::<f32>()? {
                    parts.push(part);
                    if parts.len() > 3 {
                        return Err(de::Error::invalid_length(parts.len(), &EXPECTED));
                    }
                }
                match parts[..] {
                    [a, b] => Ok(KeyValue::Pair([a, b])),
                    [a, b, c] => Ok(KeyValue::Triple([a, b, c])),
                    _ => Err(de::Error::invalid_length(parts.len(), &EXPECTED)),
                }
            }
        }

        deserializer.deserialize_any(Pick)
    }
}

pub type Key = (f32, KeyValue);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Keys(pub BTreeMap<String, Vec<Key>>);

impl Keys {
    pub fn get(&self, name: &str) -> Option<&[Key]> {
        self.0.get(name).map(Vec::as_slice)
    }

    pub fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    pub fn sample(&self, name: &str, seconds: f32) -> Option<KeyValue> {
        sample(self.get(name)?, seconds)
    }
}

pub fn sample(track: &[Key], seconds: f32) -> Option<KeyValue> {
    let (first, last) = (track.first()?, track.last()?);
    if seconds <= first.0 {
        return Some(first.1);
    }
    if seconds >= last.0 {
        return Some(last.1);
    }
    let next = track.partition_point(|(t, _)| *t <= seconds);
    let (t0, a) = track[next - 1];
    let (t1, b) = track[next];
    let span = t1 - t0;
    if span <= 0.0 {
        return Some(b);
    }
    Some(a.lerp(&b, (seconds - t0) / span))
}

pub(crate) fn numbers(pairs: &[[f32; 2]]) -> Vec<Key> {
    pairs
        .iter()
        .map(|[t, value]| (*t, KeyValue::Number(*value)))
        .collect()
}
