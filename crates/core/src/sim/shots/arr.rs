use serde::de::{Error, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserializer, Serializer};

pub fn serialize<S: Serializer, const N: usize>(
    value: &[i32; N],
    out: S,
) -> Result<S::Ok, S::Error> {
    let mut tuple = out.serialize_tuple(N)?;
    for item in value {
        tuple.serialize_element(item)?;
    }
    tuple.end()
}

pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
    input: D,
) -> Result<[i32; N], D::Error> {
    struct Axes<const N: usize>;

    impl<'de, const N: usize> Visitor<'de> for Axes<N> {
        type Value = [i32; N];

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "{N} integers")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<[i32; N], A::Error> {
            let mut out = [0; N];
            for (at, slot) in out.iter_mut().enumerate() {
                *slot = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::invalid_length(at, &self))?;
            }
            Ok(out)
        }
    }

    input.deserialize_tuple(N, Axes::<N>)
}
