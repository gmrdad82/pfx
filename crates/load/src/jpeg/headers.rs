use std::cmp::max;

use super::components::{Component, SampleRatios};
use super::huffman::HuffmanTable;
use super::{Decoder, MAX_COMPONENTS, Space};

#[rustfmt::skip]
pub(super) const UN_ZIGZAG: [usize; 64 + 16] = [
     0,  1,  8, 16,  9,  2,  3, 10,
    17, 24, 32, 25, 18, 11,  4,  5,
    12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13,  6,  7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46,
    53, 60, 61, 54, 47, 55, 62, 63,
    63, 63, 63, 63, 63, 63, 63, 63,
    63, 63, 63, 63, 63, 63, 63, 63,
];

pub(super) fn parse_huffman(decoder: &mut Decoder) -> Result<(), String> {
    let mut dht_length = i32::from(
        decoder
            .stream
            .get_u16_be_err()?
            .checked_sub(2)
            .ok_or("an invalid Huffman table length")?,
    );
    while dht_length > 16 {
        let ht_info = decoder.stream.read_u8_err()?;
        let dc_or_ac = (ht_info >> 4) & 0xF;
        let index = usize::from(ht_info & 0xF);
        let mut num_symbols = [0u8; 17];
        if index >= MAX_COMPONENTS {
            return Err(format!(
                "an invalid DHT index {index}, expected between 0 and 3"
            ));
        }
        if dc_or_ac > 1 {
            return Err(format!("an invalid DHT class {dc_or_ac}, expected 0 or 1"));
        }
        decoder.stream.read_exact(&mut num_symbols[1..17])?;
        dht_length -= 1 + 16;
        let symbols_sum: i32 = num_symbols.iter().map(|&f| i32::from(f)).sum();
        if symbols_sum > 256 {
            return Err("a Huffman table with too many symbols".into());
        }
        if symbols_sum > dht_length {
            return Err(format!(
                "a Huffman table of {symbols_sum} symbols in a segment of {dht_length} bytes"
            ));
        }
        dht_length -= symbols_sum;
        let mut symbols = [0u8; 256];
        decoder
            .stream
            .read_exact(&mut symbols[0..symbols_sum as usize])?;
        let is_dc = dc_or_ac == 0;
        let table = Some(HuffmanTable::new(&num_symbols, symbols, is_dc)?);
        if is_dc {
            decoder.dc_huffman_tables[index] = table;
        } else {
            decoder.ac_huffman_tables[index] = table;
        }
    }
    if dht_length > 0 {
        return Err("a bogus Huffman table definition".into());
    }
    Ok(())
}

pub(super) fn parse_dqt(decoder: &mut Decoder) -> Result<(), String> {
    let mut qt_length = decoder
        .stream
        .get_u16_be_err()?
        .checked_sub(2)
        .ok_or("an invalid DQT length")?;
    while qt_length > 0 {
        let qt_info = decoder.stream.read_u8_err()?;
        let precision = usize::from(qt_info >> 4);
        let table_position = usize::from(qt_info & 0x0f);
        let precision_value = 64 * (precision + 1);
        if (precision_value + 1) as u16 > qt_length {
            return Err(format!(
                "{qt_length} bytes left are too few for a quantization table of {}",
                precision_value + 1
            ));
        }
        let table = match precision {
            0 => {
                let mut values = [0u8; 64];
                decoder.stream.read_exact(&mut values)?;
                qt_length -= precision_value as u16 + 1;
                un_zig_zag(&values.map(u16::from))
            }
            1 => {
                let mut values = [0u16; 64];
                for value in &mut values {
                    *value = decoder.stream.get_u16_be_err()?;
                }
                qt_length -= precision_value as u16 + 1;
                un_zig_zag(&values)
            }
            _ => {
                return Err(format!(
                    "a quantization table precision of {precision}, expected 0 or 1"
                ));
            }
        };
        if table_position >= MAX_COMPONENTS {
            return Err(format!(
                "quantization table position {table_position} is out of range"
            ));
        }
        decoder.qt_tables[table_position] = Some(table);
    }
    Ok(())
}

pub(super) fn parse_start_of_frame(decoder: &mut Decoder) -> Result<(), String> {
    if decoder.seen_sof {
        return Err("two start-of-frame markers".into());
    }
    let length = decoder.stream.get_u16_be_err()?;
    let precision = decoder.stream.read_u8_err()?;
    if precision != 8 {
        return Err(format!(
            "{precision}-bit JPEGs are not supported, only 8-bit"
        ));
    }
    decoder.height = decoder.stream.get_u16_be_err()?;
    decoder.width = decoder.stream.get_u16_be_err()?;
    if decoder.width == 0 || decoder.height == 0 {
        return Err("the image is empty".into());
    }
    let num_components = decoder.stream.read_u8_err()?;
    if num_components == 0 {
        return Err("the frame has no components".into());
    }
    let expected = 8 + 3 * u16::from(num_components);
    if length != expected {
        return Err(format!(
            "the frame header is {length} bytes, expected {expected}"
        ));
    }
    if num_components == 1 {
        decoder.input = Space::Luma;
    }
    if num_components == 4 && decoder.input == Space::YCbCr {
        decoder.input = Space::Cmyk;
    }
    let mut components = Vec::with_capacity(usize::from(num_components));
    let mut temp = [0u8; 3];
    for pos in 0..num_components {
        decoder.stream.read_exact(&mut temp)?;
        components.push(Component::from(temp, pos)?);
    }
    decoder.seen_sof = true;
    decoder.components = components;
    let mut h_max = 1;
    let mut v_max = 1;
    for comp in &decoder.components {
        h_max = max(h_max, comp.horizontal_sample);
        v_max = max(v_max, comp.vertical_sample);
    }
    decoder.sample_ratio = match (h_max, v_max) {
        (1, 1) => SampleRatios::None,
        (1, 2) => SampleRatios::V,
        (2, 1) => SampleRatios::H,
        (2, 2) => SampleRatios::HV,
        (hs, vs) => SampleRatios::Generic(hs, vs),
    };
    Ok(())
}

pub(super) fn parse_sos(decoder: &mut Decoder) -> Result<(), String> {
    let ls = usize::from(decoder.stream.get_u16_be_err()?);
    let ns = decoder.stream.read_u8_err()?;
    let mut seen = [-1i32; MAX_COMPONENTS + 1];
    decoder.num_scans = ns;
    let smallest_size = 6 + 2 * usize::from(ns);
    if ls != smallest_size {
        return Err(format!("a bad SOS length {ls}"));
    }
    if !(1..5).contains(&ns) {
        return Err(format!(
            "{ns} components in a scan, expected between 1 and 4"
        ));
    }
    if decoder.components.is_empty() {
        return Err("a scan before the frame header".into());
    }
    decoder.scan_subsampled = false;
    for i in 0..usize::from(ns) {
        let id = decoder.stream.read_u8_err()?;
        if seen.contains(&i32::from(id)) {
            return Err(format!("component {id} appears twice in a scan"));
        }
        seen[i] = i32::from(id);
        let tables = decoder.stream.read_u8_err()?;
        let j = decoder
            .components
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| format!("a scan names component {id}, which the frame lacks"))?;
        let component = &mut decoder.components[j];
        component.dc_huff_table = usize::from((tables >> 4) & 0xF);
        component.ac_huff_table = usize::from(tables & 0xF);
        decoder.z_order[i] = j;
        if component.vertical_sample != 1 || component.horizontal_sample != 1 {
            decoder.scan_subsampled = true;
        }
    }
    decoder.spec_start = decoder.stream.read_u8_err()?;
    decoder.spec_end = decoder.stream.read_u8_err()?;
    let bit_approx = decoder.stream.read_u8_err()?;
    decoder.succ_high = bit_approx >> 4;
    if decoder.spec_end > 63 {
        return Err(format!(
            "an invalid Se {}, expected 0 to 63",
            decoder.spec_end
        ));
    }
    if decoder.spec_start > 63 {
        return Err(format!(
            "an invalid Ss {}, expected 0 to 63",
            decoder.spec_start
        ));
    }
    if decoder.succ_high > 13 {
        return Err(format!(
            "an invalid Ah {}, expected 0 to 13",
            decoder.succ_high
        ));
    }
    decoder.succ_low = bit_approx & 0xF;
    if decoder.succ_low > 13 {
        return Err(format!(
            "an invalid Al {}, expected 0 to 13",
            decoder.succ_low
        ));
    }
    Ok(())
}

pub(super) fn parse_app14(decoder: &mut Decoder) -> Result<(), String> {
    let mut length = usize::from(decoder.stream.get_u16_be());
    if length < 2 {
        return Err("an APP14 segment too short".into());
    }
    if decoder.stream.peek(5) == Some(b"Adobe") {
        if length < 14 {
            return Err("an Adobe APP14 segment too short".into());
        }
        decoder.stream.skip(11);
        let transform = decoder.stream.read_u8();
        decoder.input = match transform {
            0 => Space::Cmyk,
            1 => Space::YCbCr,
            2 => Space::Ycck,
            _ => return Err(format!("an unknown Adobe colour transform {transform}")),
        };
        length = length.saturating_sub(14);
    } else {
        length = length.saturating_sub(2);
    }
    decoder.stream.skip(length);
    Ok(())
}

fn un_zig_zag(a: &[u16; 64]) -> [i32; 64] {
    let mut output = [0i32; 64];
    for (i, &value) in a.iter().enumerate() {
        output[UN_ZIGZAG[i]] = i32::from(value);
    }
    output
}
