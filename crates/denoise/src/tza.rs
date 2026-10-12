use std::collections::BTreeMap;

const MAGIC: u16 = 0x41D7;
const MAX_DIM: u32 = 1 << 30;

#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    pub dims: Vec<usize>,
    pub data: Vec<f32>,
}

pub type Tensors = BTreeMap<String, Tensor>;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .filter(|&end| end <= self.bytes.len())
            .ok_or(CORRUPT)?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

const CORRUPT: &str = "invalid or corrupted weights blob";

pub fn parse(bytes: &[u8]) -> Result<Tensors, String> {
    let mut head = Reader { bytes, at: 0 };
    if head.u16()? != MAGIC {
        return Err(CORRUPT.into());
    }
    let major = head.u8()?;
    let _minor = head.u8()?;
    if major != 2 {
        return Err("unsupported weights blob version".into());
    }
    let table = usize::try_from(head.u64()?).map_err(|_| CORRUPT)?;
    let mut input = Reader { bytes, at: table };
    let count = input.u32()?;
    let mut tensors = Tensors::new();
    for _ in 0..count {
        let name_len = usize::from(input.u16()?);
        let name = std::str::from_utf8(input.take(name_len)?)
            .map_err(|_| CORRUPT)?
            .to_string();
        let rank = usize::from(input.u8()?);
        let mut dims = Vec::with_capacity(rank);
        for _ in 0..rank {
            let dim = input.u32()?;
            if dim == 0 || dim > MAX_DIM {
                return Err("invalid tensor dimension".into());
            }
            dims.push(dim as usize);
        }
        let layout = input.take(rank)?;
        if layout != b"x" && layout != b"oihw" {
            return Err("invalid tensor layout".into());
        }
        let size = match input.u8()? {
            b'f' => 4,
            b'h' => 2,
            _ => return Err("invalid tensor data type".into()),
        };
        let elements = dims
            .iter()
            .try_fold(1usize, |total, &dim| total.checked_mul(dim))
            .ok_or(CORRUPT)?;
        let offset = usize::try_from(input.u64()?).map_err(|_| CORRUPT)?;
        let mut data_reader = Reader { bytes, at: offset };
        let raw = data_reader.take(elements.checked_mul(size).ok_or(CORRUPT)?)?;
        let data = if size == 4 {
            raw.chunks_exact(4)
                .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
                .collect()
        } else {
            raw.chunks_exact(2)
                .map(|value| half_to_f32(u16::from_le_bytes(value.try_into().unwrap())))
                .collect()
        };
        tensors.insert(name, Tensor { dims, data });
    }
    Ok(tensors)
}

pub fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x3ff);
    let value = match (exponent, mantissa) {
        (0, 0) => sign,
        (0, _) => {
            let shift = mantissa.leading_zeros() - 21;
            let normalized = (mantissa << shift) & 0x3ff;
            sign | ((113 - shift) << 23) | (normalized << 13)
        }
        (31, 0) => sign | 0x7f80_0000,
        (31, _) => sign | 0x7fc0_0000 | (mantissa << 13),
        _ => sign | ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_widen_exactly() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert_eq!(half_to_f32(0x7bff), 65504.0);
        assert_eq!(half_to_f32(0x0001), 2f32.powi(-24));
        assert_eq!(half_to_f32(0x03ff), 1023.0 * 2f32.powi(-24));
        assert_eq!(half_to_f32(0x3555), 0.333_251_95);
        assert!(half_to_f32(0x7c00).is_infinite());
    }

    #[test]
    fn the_shipped_weights_parse_with_their_layers() {
        let tensors = parse(crate::WEIGHTS).unwrap();
        assert_eq!(tensors["enc_conv0.weight"].dims, [32, 9, 3, 3]);
        assert_eq!(tensors["dec_conv0.bias"].dims, [3]);
        assert_eq!(tensors.len(), 32);
        assert!(parse(&crate::WEIGHTS[..100]).is_err());
    }
}
