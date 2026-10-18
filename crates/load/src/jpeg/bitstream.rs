use std::cmp::min;

use super::headers::UN_ZIGZAG;
use super::huffman::{HUFF_LOOKAHEAD, HuffmanTable};
use super::{Marker, Reader};

pub(super) struct BitStream {
    buffer: u64,
    aligned_buffer: u64,
    pub(super) bits_left: u8,
    pub(super) marker: Option<Marker>,
    successive_low_mask: i16,
    spec_start: u8,
    spec_end: u8,
    pub(super) eob_run: i32,
    pub(super) overread_by: usize,
    pub(super) seen_eoi: bool,
}

impl BitStream {
    pub(super) fn new() -> BitStream {
        BitStream::new_progressive(0, 0, 0)
    }

    pub(super) fn new_progressive(al: u8, spec_start: u8, spec_end: u8) -> BitStream {
        BitStream {
            buffer: 0,
            aligned_buffer: 0,
            bits_left: 0,
            marker: None,
            successive_low_mask: 1i16 << al,
            spec_start,
            spec_end,
            eob_run: 0,
            overread_by: 0,
            seen_eoi: false,
        }
    }

    #[inline(always)]
    fn refill_byte(&mut self, reader: &mut Reader) -> Result<bool, String> {
        let byte = u64::from(reader.read_u8());
        self.overread_by += usize::from(reader.eof());
        self.buffer = (self.buffer << 8) | byte;
        self.bits_left += 8;
        if byte == 0xFF {
            let mut next_byte = reader.read_u8();
            if next_byte != 0x00 {
                while next_byte == 0xFF {
                    next_byte = reader.read_u8();
                }
                if next_byte != 0x00 {
                    self.buffer >>= 8;
                    self.bits_left -= 8;
                    if self.bits_left != 0 {
                        self.aligned_buffer = self.buffer << (64 - self.bits_left);
                    }
                    let marker = Marker::from_u8(next_byte);
                    self.marker = Some(marker);
                    if let Marker::Unknown(_) = marker {
                        return Err("an unknown marker inside the coded data".into());
                    }
                    if next_byte == 0xD9 {
                        self.buffer <<= 8;
                        self.bits_left += 8;
                        self.aligned_buffer = self.buffer << (64 - self.bits_left);
                    }
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    #[inline(always)]
    pub(super) fn refill(&mut self, reader: &mut Reader) -> Result<bool, String> {
        if self.bits_left < 32 {
            if self.marker.is_some() || self.seen_eoi {
                self.buffer <<= 32;
                self.bits_left += 32;
                self.aligned_buffer = self.buffer << (64 - self.bits_left);
                return Ok(true);
            }
            if self.overread_by > 0 {
                if self.bits_left == 0 {
                    return Err("the coded data runs out".into());
                }
                return Ok(true);
            }
            if let Some(msb_buf) = reader.read4() {
                if !has_byte(msb_buf, 255) {
                    self.bits_left += 32;
                    self.buffer <<= 32;
                    self.buffer |= u64::from(msb_buf);
                    self.aligned_buffer = self.buffer << (64 - self.bits_left);
                    return Ok(true);
                }
                reader.rewind(4);
            }
            for _ in 0..4 {
                if !self.refill_byte(reader)? {
                    return Ok(false);
                }
            }
            self.aligned_buffer = self.buffer << (64 - self.bits_left);
        }
        Ok(true)
    }

    #[inline(always)]
    fn decode_huff(&mut self, mut symbol: i32, table: &HuffmanTable) -> Result<i32, String> {
        let mut code_length = symbol >> HUFF_LOOKAHEAD;
        symbol &= (1 << HUFF_LOOKAHEAD) - 1;
        if code_length > i32::from(HUFF_LOOKAHEAD) {
            symbol = self.peek_bits::<16>();
            while code_length < 17 {
                if symbol < table.maxcode[code_length as usize] {
                    break;
                }
                code_length += 1;
            }
            if code_length == 17 {
                return Err(format!("a bad Huffman code 0x{symbol:X}"));
            }
            symbol >>= 16 - code_length;
            symbol = i32::from(
                table.values[((symbol + table.offset[code_length as usize]) & 0xFF) as usize],
            );
        }
        if code_length > i32::from(self.bits_left) {
            return Err(format!(
                "a code of {code_length} bits with {} bits left",
                self.bits_left
            ));
        }
        self.drop_bits(code_length as u8);
        Ok(symbol)
    }

    #[inline(always)]
    fn decode_dc(
        &mut self,
        reader: &mut Reader,
        dc_table: &HuffmanTable,
        dc_prediction: &mut i32,
    ) -> Result<(), String> {
        if self.bits_left < 32 {
            self.refill(reader)?;
        }
        let peek = self.peek_bits::<HUFF_LOOKAHEAD>();
        let mut symbol = self.decode_huff(dc_table.lookup[peek as usize], dc_table)?;
        if symbol != 0 {
            let r = self.get_bits(symbol as u8);
            symbol = huff_extend(r, symbol);
        }
        *dc_prediction = dc_prediction.wrapping_add(symbol);
        Ok(())
    }

    #[inline(never)]
    pub(super) fn decode_mcu_block(
        &mut self,
        reader: &mut Reader,
        dc_table: &HuffmanTable,
        ac_table: &HuffmanTable,
        block: &mut [i16; 64],
        dc_prediction: &mut i32,
    ) -> Result<(), String> {
        let ac_lookup = &ac_table.ac_lookup;
        let mut pos: usize = 1;
        if self.bits_left < 1 && self.marker.is_some() {
            return Err("no bytes left before a marker".into());
        }
        self.decode_dc(reader, dc_table, dc_prediction)?;
        block[0] = *dc_prediction as i16;
        while pos < 64 {
            self.refill(reader)?;
            let peek = self.peek_bits::<HUFF_LOOKAHEAD>() as usize;
            let fast_ac = ac_lookup[peek];
            if fast_ac != 0 {
                pos += ((fast_ac >> 4) & 15) as usize;
                block[UN_ZIGZAG[min(pos, 63)] & 63] = fast_ac >> 8;
                self.drop_bits((fast_ac & 15) as u8);
                pos += 1;
            } else {
                let mut symbol = self.decode_huff(ac_table.lookup[peek], ac_table)?;
                let mut r = symbol >> 4;
                symbol &= 15;
                if symbol != 0 {
                    pos += r as usize;
                    r = self.get_bits(symbol as u8);
                    symbol = huff_extend(r, symbol);
                    block[UN_ZIGZAG[pos & 63] & 63] = symbol as i16;
                    pos += 1;
                } else if r != 15 {
                    return Ok(());
                } else {
                    pos += 16;
                }
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn peek_bits<const LOOKAHEAD: u8>(&self) -> i32 {
        (self.aligned_buffer >> (64 - LOOKAHEAD)) as i32
    }

    #[inline(always)]
    fn drop_bits(&mut self, n: u8) {
        self.bits_left = self.bits_left.saturating_sub(n);
        self.aligned_buffer <<= n;
    }

    #[inline(always)]
    fn get_bits(&mut self, n_bits: u8) -> i32 {
        let mask = (1_u64 << n_bits) - 1;
        self.aligned_buffer = self.aligned_buffer.rotate_left(u32::from(n_bits));
        let bits = (self.aligned_buffer & mask) as i32;
        self.bits_left = self.bits_left.wrapping_sub(n_bits);
        bits
    }

    fn get_bit(&mut self) -> u8 {
        let k = (self.aligned_buffer >> 63) as u8;
        self.drop_bits(1);
        k
    }

    pub(super) fn decode_prog_dc_first(
        &mut self,
        reader: &mut Reader,
        dc_table: &HuffmanTable,
        block: &mut i16,
        dc_prediction: &mut i32,
    ) -> Result<(), String> {
        self.decode_dc(reader, dc_table, dc_prediction)?;
        *block = (*dc_prediction as i16).wrapping_mul(self.successive_low_mask);
        Ok(())
    }

    pub(super) fn decode_prog_dc_refine(
        &mut self,
        reader: &mut Reader,
        block: &mut i16,
    ) -> Result<(), String> {
        if self.bits_left < 1 {
            self.refill(reader)?;
            if self.bits_left < 1 {
                return Err("a marker where a refinement bit was expected".into());
            }
        }
        if self.get_bit() == 1 {
            *block = block.wrapping_add(self.successive_low_mask);
        }
        Ok(())
    }

    pub(super) fn decode_mcu_ac_first(
        &mut self,
        reader: &mut Reader,
        ac_table: &HuffmanTable,
        block: &mut [i16; 64],
    ) -> Result<(), String> {
        let fast_ac = &ac_table.ac_lookup;
        let bit = self.successive_low_mask;
        let mut k = usize::from(self.spec_start);
        loop {
            self.refill(reader)?;
            let peek = self.peek_bits::<HUFF_LOOKAHEAD>() as usize;
            let fac = fast_ac[peek];
            if fac != 0 {
                k += ((fac >> 4) & 15) as usize;
                block[UN_ZIGZAG[min(k, 63)] & 63] = (fac >> 8).wrapping_mul(bit);
                self.drop_bits((fac & 15) as u8);
                k += 1;
            } else {
                let mut symbol = self.decode_huff(ac_table.lookup[peek], ac_table)?;
                let mut r = symbol >> 4;
                symbol &= 15;
                if symbol != 0 {
                    k += r as usize;
                    r = self.get_bits(symbol as u8);
                    symbol = huff_extend(r, symbol);
                    block[UN_ZIGZAG[k & 63] & 63] = (symbol as i16).wrapping_mul(bit);
                    k += 1;
                } else {
                    if r != 15 {
                        self.eob_run = 1 << r;
                        self.eob_run += self.get_bits(r as u8);
                        self.eob_run -= 1;
                        break;
                    }
                    k += 16;
                }
            }
            if k > usize::from(self.spec_end) {
                break;
            }
        }
        Ok(())
    }

    pub(super) fn decode_mcu_ac_refine(
        &mut self,
        reader: &mut Reader,
        table: &HuffmanTable,
        block: &mut [i16; 64],
    ) -> Result<(), String> {
        let bit = self.successive_low_mask;
        let mut k = self.spec_start;
        if self.eob_run == 0 {
            'no_eob: loop {
                self.refill(reader)?;
                let peek = self.peek_bits::<HUFF_LOOKAHEAD>();
                let mut symbol = self.decode_huff(table.lookup[peek as usize], table)?;
                let mut r = symbol >> 4;
                symbol &= 15;
                if symbol == 0 {
                    if r != 15 {
                        self.eob_run = 1 << r;
                        self.eob_run += self.get_bits(r as u8);
                        break 'no_eob;
                    }
                } else if self.get_bit() == 1 {
                    symbol = i32::from(bit);
                } else {
                    symbol = i32::from(-bit);
                }
                if k <= self.spec_end {
                    loop {
                        let coefficient = &mut block[UN_ZIGZAG[usize::from(k) & 63] & 63];
                        if *coefficient != 0 {
                            if self.bits_left < 1 {
                                self.refill(reader)?;
                                if self.bits_left < 1 && self.marker.is_some() {
                                    return Err(
                                        "a marker where a refinement bit was expected".into()
                                    );
                                }
                            }
                            if self.get_bit() == 1 && (*coefficient & bit) == 0 {
                                if *coefficient > 0 {
                                    *coefficient = coefficient.wrapping_add(bit);
                                } else {
                                    *coefficient = coefficient.wrapping_sub(bit);
                                }
                            }
                        } else {
                            r -= 1;
                            if r < 0 {
                                break;
                            }
                        }
                        if k == self.spec_end {
                            break;
                        }
                        k += 1;
                    }
                }
                if symbol != 0 {
                    let pos = UN_ZIGZAG[usize::from(k) & 63];
                    block[pos & 63] = symbol as i16;
                }
                k += 1;
                if k > self.spec_end {
                    break 'no_eob;
                }
            }
        }
        if self.eob_run > 0 {
            if block[1..].iter().any(|&c| c != 0) {
                self.refill(reader)?;
                while k <= self.spec_end {
                    let coefficient = &mut block[UN_ZIGZAG[usize::from(k) & 63] & 63];
                    if *coefficient != 0 && self.get_bit() == 1 && (*coefficient & bit) == 0 {
                        if *coefficient >= 0 {
                            *coefficient = coefficient.wrapping_add(bit);
                        } else {
                            *coefficient = coefficient.wrapping_sub(bit);
                        }
                    }
                    if self.bits_left < 1 {
                        self.refill(reader)?;
                    }
                    k += 1;
                }
            }
            self.eob_run -= 1;
        }
        Ok(())
    }

    pub(super) fn update_progressive_params(&mut self, al: u8, spec_start: u8, spec_end: u8) {
        self.successive_low_mask = 1i16 << al;
        self.spec_start = spec_start;
        self.spec_end = spec_end;
    }

    pub(super) fn reset(&mut self) {
        self.bits_left = 0;
        self.marker = None;
        self.buffer = 0;
        self.aligned_buffer = 0;
        self.eob_run = 0;
    }
}

#[inline(always)]
fn huff_extend(x: i32, s: i32) -> i32 {
    x + (((x - (1 << (s - 1))) >> 31) & ((-1 << s) + 1))
}

const fn has_zero(v: u32) -> bool {
    !((((v & 0x7F7F_7F7F) + 0x7F7F_7F7F) | v) | 0x7F7F_7F7F) != 0
}

const fn has_byte(b: u32, val: u8) -> bool {
    has_zero(b ^ ((!0_u32 / 255) * (val as u32)))
}
