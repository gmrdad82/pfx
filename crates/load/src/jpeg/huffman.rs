pub(super) const HUFF_LOOKAHEAD: u8 = 9;

#[derive(Clone)]
pub(super) struct HuffmanTable {
    pub(super) maxcode: [i32; 18],
    pub(super) offset: [i32; 18],
    pub(super) lookup: [i32; 1 << HUFF_LOOKAHEAD],
    pub(super) ac_lookup: [i16; 1 << HUFF_LOOKAHEAD],
    pub(super) values: [u8; 256],
}

impl HuffmanTable {
    pub(super) fn new(codes: &[u8; 17], values: [u8; 256], is_dc: bool) -> Result<Self, String> {
        let too_long_code = (i32::from(HUFF_LOOKAHEAD) + 1) << HUFF_LOOKAHEAD;
        let mut table = HuffmanTable {
            maxcode: [0; 18],
            offset: [0; 18],
            lookup: [too_long_code; 1 << HUFF_LOOKAHEAD],
            ac_lookup: [0; 1 << HUFF_LOOKAHEAD],
            values,
        };
        table.make_derived_table(is_dc, codes)?;
        Ok(table)
    }

    fn make_derived_table(&mut self, is_dc: bool, bits: &[u8; 17]) -> Result<(), String> {
        let mut huff_size = [0u8; 257];
        let mut huff_code = [0u32; 257];
        let mut p = 0;
        for (l, &count) in bits.iter().enumerate().skip(1) {
            for _ in 0..count {
                huff_size[p] = l as u8;
                p += 1;
            }
        }
        huff_size[p] = 0;
        let num_symbols = p;
        let mut code = 0u32;
        let mut si = i32::from(huff_size[0]);
        p = 0;
        while huff_size[p] != 0 {
            while i32::from(huff_size[p]) == si {
                huff_code[p] = code;
                code += 1;
                p += 1;
            }
            self.maxcode[si as usize] = (code << (16 - si)) as i32;
            if (code as i32) >= (1 << si) {
                return Err("a bad Huffman table".into());
            }
            code <<= 1;
            si += 1;
        }
        p = 0;
        for (l, &count) in bits.iter().enumerate() {
            if count == 0 {
                self.maxcode[l] = -1;
            } else {
                self.offset[l] = (p as i32) - (huff_code[p] as i32);
                p += usize::from(count);
            }
        }
        self.offset[17] = 0;
        self.maxcode[17] = 0x000F_FFFF;
        p = 0;
        for l in 1..=HUFF_LOOKAHEAD {
            for _ in 0..bits[usize::from(l)] {
                let first = (huff_code[p] as usize) << (HUFF_LOOKAHEAD - l);
                let entry = (i32::from(l) << HUFF_LOOKAHEAD) | i32::from(self.values[p]);
                self.lookup[first..first + (1 << (HUFF_LOOKAHEAD - l))].fill(entry);
                p += 1;
            }
        }
        if !is_dc {
            let mut fast = [255i16; 1 << HUFF_LOOKAHEAD];
            for i in 0..num_symbols {
                let s = huff_size[i];
                if s <= HUFF_LOOKAHEAD {
                    let c = (huff_code[i] << (HUFF_LOOKAHEAD - s)) as usize;
                    let m = 1usize << (HUFF_LOOKAHEAD - s);
                    for slot in &mut fast[c..c + m] {
                        *slot = i as i16;
                    }
                }
            }
            for (i, &fast_v) in fast.iter().enumerate() {
                if fast_v < 255 {
                    let rs = self.values[fast_v as usize];
                    let run = i16::from((rs >> 4) & 15);
                    let mag_bits = i16::from(rs & 15);
                    let len = i16::from(huff_size[fast_v as usize]);
                    if mag_bits != 0 && (len + mag_bits) <= i16::from(HUFF_LOOKAHEAD) {
                        let mut k = (((i as i16) << len) & ((1 << HUFF_LOOKAHEAD) - 1))
                            >> (i16::from(HUFF_LOOKAHEAD) - mag_bits);
                        let m = 1 << (mag_bits - 1);
                        if k < m {
                            k += (!0_i16 << mag_bits) + 1;
                        }
                        if (-128..=127).contains(&k) {
                            self.ac_lookup[i] = (k << 8) + (run << 4) + (len + mag_bits);
                        }
                    }
                }
            }
        }
        if is_dc && self.values[..num_symbols].iter().any(|&sym| sym > 15) {
            return Err("a bad Huffman table".into());
        }
        Ok(())
    }
}
