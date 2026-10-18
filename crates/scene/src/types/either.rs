use std::fmt;

use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};

pub(crate) fn invalid<E: de::Error>(unexpected: de::Unexpected<'_>, expected: &str) -> E {
    E::invalid_type(unexpected, &expected)
}

macro_rules! either {
    (
        $name:ident, $expected:literal,
        $( bool => $bool_variant:ident, )?
        $( number => $number_variant:ident, )?
        $( text => $text_variant:ident($text_type:ty), )?
        $( seq => $seq_variant:ident($seq_type:ty), )?
        $( map => $map_variant:ident($map_type:ty), )?
    ) => {
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Pick;

                impl<'de> serde::de::Visitor<'de> for Pick {
                    type Value = $name;

                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str($expected)
                    }

                    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<$name, E> {
                        $( return Ok($name::$bool_variant(value)); )?
                        #[allow(unreachable_code)]
                        Err($crate::types::either::invalid(serde::de::Unexpected::Bool(value), $expected))
                    }

                    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<$name, E> {
                        $( return Ok($name::$number_variant(value as f32)); )?
                        #[allow(unreachable_code)]
                        Err($crate::types::either::invalid(serde::de::Unexpected::Signed(value), $expected))
                    }

                    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<$name, E> {
                        $( return Ok($name::$number_variant(value as f32)); )?
                        #[allow(unreachable_code)]
                        Err($crate::types::either::invalid(serde::de::Unexpected::Unsigned(value), $expected))
                    }

                    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<$name, E> {
                        $( return Ok($name::$number_variant(value as f32)); )?
                        #[allow(unreachable_code)]
                        Err($crate::types::either::invalid(serde::de::Unexpected::Float(value), $expected))
                    }

                    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<$name, E> {
                        $(
                            return <$text_type as serde::Deserialize>::deserialize(
                                serde::de::value::StrDeserializer::<E>::new(value),
                            )
                            .map($name::$text_variant);
                        )?
                        #[allow(unreachable_code)]
                        Err($crate::types::either::invalid(serde::de::Unexpected::Str(value), $expected))
                    }

                    #[allow(unused_mut)]
                    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<$name, A::Error> {
                        $(
                            let value = <$seq_type as serde::Deserialize>::deserialize(
                                serde::de::value::SeqAccessDeserializer::new(&mut seq),
                            )?;
                            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                                return Err(<A::Error as serde::de::Error>::custom(format!(
                                    "invalid length, expected {}",
                                    $expected
                                )));
                            }
                            return Ok($name::$seq_variant(value));
                        )?
                        #[allow(unreachable_code)]
                        {
                            let _ = seq;
                            Err($crate::types::either::invalid(serde::de::Unexpected::Seq, $expected))
                        }
                    }

                    fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<$name, A::Error> {
                        $(
                            return <$map_type as serde::Deserialize>::deserialize(
                                serde::de::value::MapAccessDeserializer::new(map),
                            )
                            .map($name::$map_variant);
                        )?
                        #[allow(unreachable_code)]
                        {
                            let _ = map;
                            Err($crate::types::either::invalid(serde::de::Unexpected::Map, $expected))
                        }
                    }
                }

                deserializer.deserialize_any(Pick)
            }
        }

        #[cfg(feature = "schema")]
        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            #[allow(unused_variables)]
            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                let mut any: Vec<schemars::Schema> = Vec::new();
                $( let _ = stringify!($bool_variant); any.push(schemars::json_schema!({ "type": "boolean" })); )?
                $( let _ = stringify!($number_variant); any.push(schemars::json_schema!({ "type": "number" })); )?
                $( let _ = stringify!($text_variant); any.push(generator.subschema_for::<$text_type>()); )?
                $( let _ = stringify!($seq_variant); any.push(generator.subschema_for::<$seq_type>()); )?
                $( let _ = stringify!($map_variant); any.push(generator.subschema_for::<$map_type>()); )?
                schemars::json_schema!({ "anyOf": any })
            }
        }
    };
}

pub(crate) use either;

#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Hex(String),
    Rgb([f32; 3]),
}

impl Color {
    pub fn rgb(&self) -> Option<[f32; 3]> {
        match self {
            Self::Rgb(rgb) => Some(*rgb),
            Self::Hex(text) => {
                let hex = text.strip_prefix('#')?;
                if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return None;
                }
                let byte = |at: usize| -> Option<f32> {
                    let value = u8::from_str_radix(&hex[at..at + 2], 16).ok()?;
                    Some(srgb_to_linear(f32::from(value) / 255.0))
                };
                Some([byte(0)?, byte(2)?, byte(4)?])
            }
        }
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::Rgb(rgb) => rgb.iter().all(|value| value.is_finite()),
            Self::Hex(_) => self.rgb().is_some(),
        }
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Hex(text) => serializer.serialize_str(text),
            Self::Rgb(rgb) => rgb.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Pick;

        impl<'de> Visitor<'de> for Pick {
            type Value = Color;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("\"#rrggbb\" or three numbers")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Color, E> {
                Ok(Color::Hex(value.to_string()))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Color, A::Error> {
                <[f32; 3]>::deserialize(SeqAccessDeserializer::new(seq)).map(Color::Rgb)
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Color, A::Error> {
                let _ = MapAccessDeserializer::new(map);
                Err(invalid(de::Unexpected::Map, "\"#rrggbb\" or three numbers"))
            }
        }

        deserializer.deserialize_any(Pick)
    }
}
