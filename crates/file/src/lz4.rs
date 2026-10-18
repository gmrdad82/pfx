const MFLIMIT: usize = 12;
const LAST_LITERALS: usize = 5;
const END_OFFSET: usize = LAST_LITERALS + 1;
const MIN_LENGTH: usize = MFLIMIT + 1;
const MAX_DISTANCE: usize = (1 << 16) - 1;
const MINMATCH: usize = 4;
const STEP_SHIFT: usize = 5;
const TABLE_SIZE: usize = 4 * 1024;
const TABLE_SHIFT: usize = 4;

trait Table {
    fn get(&self, hash: usize) -> usize;
    fn put(&mut self, hash: usize, value: usize);
    fn hash_at(input: &[u8], at: usize) -> usize;
}

struct Small(Box<[u16; TABLE_SIZE]>);
struct Large(Box<[u32; TABLE_SIZE]>);

fn batch(input: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([input[at], input[at + 1], input[at + 2], input[at + 3]])
}

fn batch64(input: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&input[at..at + 8]);
    u64::from_le_bytes(word)
}

impl Table for Small {
    fn get(&self, hash: usize) -> usize {
        self.0[hash >> TABLE_SHIFT] as usize
    }

    fn put(&mut self, hash: usize, value: usize) {
        self.0[hash >> TABLE_SHIFT] = value as u16;
    }

    fn hash_at(input: &[u8], at: usize) -> usize {
        (batch(input, at).wrapping_mul(2654435761) >> 16) as usize
    }
}

impl Table for Large {
    fn get(&self, hash: usize) -> usize {
        self.0[hash >> TABLE_SHIFT] as usize
    }

    fn put(&mut self, hash: usize, value: usize) {
        self.0[hash >> TABLE_SHIFT] = value as u32;
    }

    fn hash_at(input: &[u8], at: usize) -> usize {
        ((batch64(input, at) << 24).wrapping_mul(889523592379) >> 48) as usize
    }
}

fn token(literals: usize, matched: usize) -> u8 {
    let high = if literals < 0xF {
        (literals as u8) << 4
    } else {
        0xF0
    };
    high | if matched < 0xF { matched as u8 } else { 0xF }
}

fn write_integer(out: &mut Vec<u8>, mut n: usize) {
    while n >= 0xFF {
        n -= 0xFF;
        out.push(0xFF);
    }
    out.push(n as u8);
}

fn last_literals(out: &mut Vec<u8>, input: &[u8], start: usize) {
    let literals = input.len() - start;
    out.push(token(literals, 0));
    if literals >= 0xF {
        write_integer(out, literals - 0xF);
    }
    out.extend_from_slice(&input[start..]);
}

fn same_bytes(input: &[u8], cur: &mut usize, candidate: usize) -> usize {
    const WORD: usize = 8;
    let ahead = &input[*cur..input.len() - END_OFFSET];
    let behind = &input[candidate..];
    let mut num = 0;
    for (a, b) in ahead.chunks_exact(WORD).zip(behind.chunks_exact(WORD)) {
        let diff = batch64(a, 0) ^ batch64(b, 0);
        if diff != 0 {
            num += (diff.trailing_zeros() / 8) as usize;
            *cur += num;
            return num;
        }
        num += WORD;
    }
    num += ahead
        .iter()
        .zip(behind)
        .skip(num)
        .take_while(|(a, b)| a == b)
        .count();
    *cur += num;
    num
}

fn compress_with<T: Table>(input: &[u8], table: &mut T, out: &mut Vec<u8>) {
    if input.len() < MIN_LENGTH {
        last_literals(out, input, 0);
        return;
    }
    let end_check = input.len() - MFLIMIT;
    let mut literal_start = 0;
    table.put(T::hash_at(input, 0), 0);
    let mut cur = 1;
    loop {
        let mut misses = 1usize << STEP_SHIFT;
        let mut next = cur;
        let mut candidate;
        loop {
            let step = misses >> STEP_SHIFT;
            misses += 1;
            cur = next;
            next += step;
            if cur > end_check {
                last_literals(out, input, literal_start);
                return;
            }
            let hash = T::hash_at(input, cur);
            candidate = table.get(hash);
            table.put(hash, cur);
            if cur - candidate > MAX_DISTANCE {
                continue;
            }
            if batch(input, candidate) == batch(input, cur) {
                break;
            }
        }
        while candidate > 0 && cur > literal_start && input[cur - 1] == input[candidate - 1] {
            cur -= 1;
            candidate -= 1;
        }
        let literals = cur - literal_start;
        let offset = (cur - candidate) as u16;
        cur += MINMATCH;
        let matched = same_bytes(input, &mut cur, candidate + MINMATCH);
        table.put(T::hash_at(input, cur - 2), cur - 2);
        out.push(token(literals, matched));
        if literals >= 0xF {
            write_integer(out, literals - 0xF);
        }
        out.extend_from_slice(&input[literal_start..literal_start + literals]);
        out.extend_from_slice(&offset.to_le_bytes());
        if matched >= 0xF {
            write_integer(out, matched - 0xF);
        }
        literal_start = cur;
    }
}

pub fn compress(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + 4 + input.len() + input.len() / 10);
    if input.len() < u16::MAX as usize {
        compress_with(input, &mut Small(Box::new([0; TABLE_SIZE])), &mut out);
    } else {
        compress_with(input, &mut Large(Box::new([0; TABLE_SIZE])), &mut out);
    }
    out
}

fn read_integer(input: &[u8], at: &mut usize) -> Option<usize> {
    let mut n = 0usize;
    loop {
        let extra = *input.get(*at)?;
        *at += 1;
        n += extra as usize;
        if extra != 0xFF {
            return Some(n);
        }
    }
}

fn read_offset(input: &[u8], at: &mut usize) -> Option<usize> {
    let bytes = input.get(*at..*at + 2)?;
    *at += 2;
    let offset = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
    (offset != 0).then_some(offset)
}

fn fits(token: u8) -> bool {
    token & 0x0F != 0x0F && token & 0xF0 != 0xF0
}

fn overlapping(out: &mut [u8], pos: usize, offset: usize, len: usize) {
    if offset == 1 {
        let byte = out[pos - 1];
        out[pos..pos + len].fill(byte);
    } else {
        for i in pos..pos + len {
            out[i] = out[i - offset];
        }
    }
}

pub fn decompress(input: &[u8], raw: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; raw];
    let cap = out.len();
    let mut at = 0;
    let mut pos = 0;
    let safe_input = input.len().saturating_sub(16 + 2);
    let safe_output = cap.saturating_sub(16 + 18);
    loop {
        let token = *input.get(at)?;
        at += 1;
        if fits(token) && at <= safe_input && pos < safe_output {
            let literals = (token >> 4) as usize;
            out[pos..pos + 16].copy_from_slice(&input[at..at + 16]);
            pos += literals;
            at += literals;
            let offset = read_offset(input, &mut at)?;
            let matched = MINMATCH + (token & 0xF) as usize;
            let start = pos.checked_sub(offset)?;
            if offset >= matched {
                out.copy_within(start..start + 18, pos);
            } else {
                overlapping(&mut out, pos, offset, matched);
            }
            pos += matched;
            continue;
        }
        let mut literals = (token >> 4) as usize;
        if literals != 0 {
            if literals == 15 {
                literals += read_integer(input, &mut at)?;
            }
            if literals > input.len() - at || literals > cap - pos {
                return None;
            }
            out[pos..pos + literals].copy_from_slice(&input[at..at + literals]);
            at += literals;
            pos += literals;
        }
        if at >= input.len() {
            break;
        }
        let offset = read_offset(input, &mut at)?;
        let mut matched = MINMATCH + (token & 0xF) as usize;
        if matched == MINMATCH + 15 {
            matched += read_integer(input, &mut at)?;
        }
        if matched > cap - pos {
            return None;
        }
        let start = pos.checked_sub(offset)?;
        if matched > offset {
            overlapping(&mut out, pos, offset, matched);
        } else {
            out.copy_within(start..start + matched, pos);
        }
        pos += matched;
    }
    (pos == raw).then_some(out)
}
