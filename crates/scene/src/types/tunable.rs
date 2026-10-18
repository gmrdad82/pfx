use std::fmt;

use serde::de::value::SeqAccessDeserializer;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use super::either::invalid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TunableKind {
    Float,
    Int,
    Bool,
    Vector,
}

impl TunableKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Vector => "vector",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TunableValue {
    Bool(bool),
    Int(i64),
    Float(f32),
    Vector([f32; 3]),
}

impl TunableValue {
    pub fn kind(self) -> TunableKind {
        match self {
            Self::Bool(_) => TunableKind::Bool,
            Self::Int(_) => TunableKind::Int,
            Self::Float(_) => TunableKind::Float,
            Self::Vector(_) => TunableKind::Vector,
        }
    }

    pub fn finite(self) -> bool {
        match self {
            Self::Bool(_) | Self::Int(_) => true,
            Self::Float(value) => value.is_finite(),
            Self::Vector(axes) => axes.iter().all(|value| value.is_finite()),
        }
    }
}

impl fmt::Display for TunableValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => write!(f, "{value}"),
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value:?}"),
            Self::Vector([x, y, z]) => write!(f, "[{x:?}, {y:?}, {z:?}]"),
        }
    }
}

const EXPECTED: &str = "true or false, a number or [x, y, z]";

impl<'de> Deserialize<'de> for TunableValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Pick;

        impl<'de> Visitor<'de> for Pick {
            type Value = TunableValue;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(EXPECTED)
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<TunableValue, E> {
                Ok(TunableValue::Bool(value))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<TunableValue, E> {
                Ok(TunableValue::Int(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<TunableValue, E> {
                i64::try_from(value)
                    .map(TunableValue::Int)
                    .map_err(|_| invalid(de::Unexpected::Unsigned(value), EXPECTED))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<TunableValue, E> {
                Ok(TunableValue::Float(value as f32))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<TunableValue, A::Error> {
                <[f32; 3]>::deserialize(SeqAccessDeserializer::new(seq)).map(TunableValue::Vector)
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<TunableValue, A::Error> {
                let _ = map;
                Err(invalid(de::Unexpected::Map, EXPECTED))
            }
        }

        deserializer.deserialize_any(Pick)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Tunable {
    #[serde(rename = "type")]
    pub kind: TunableKind,
    pub default: TunableValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<TunableValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<TunableValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl Tunable {
    pub(crate) fn settle(&mut self) {
        if self.kind != TunableKind::Float {
            return;
        }
        for value in [&mut self.default]
            .into_iter()
            .chain(self.min.as_mut())
            .chain(self.max.as_mut())
        {
            if let TunableValue::Int(whole) = *value {
                *value = TunableValue::Float(whole as f32);
            }
        }
    }
}
