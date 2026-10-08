use std::fmt;

pub const STREAM_VERSION: u32 = 1;
pub const STATE_BYTES: usize = 44;

const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
const FORK_TAG: u64 = 0xd1b5_4a32_d192_ed03;
const F64_UNIT: f64 = 1.0 / 9007199254740992.0;
const F32_UNIT: f32 = 1.0 / 16777216.0;

pub fn mix64(value: u64) -> u64 {
    let z = value.wrapping_add(GAMMA);
    let z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub fn splitmix64(state: &mut u64) -> u64 {
    let out = mix64(*state);
    *state = state.wrapping_add(GAMMA);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimError {
    Length { found: usize },
    Version { found: u32 },
    ZeroState,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { found } => {
                write!(
                    f,
                    "saved rng state is {found} bytes, expected {STATE_BYTES}"
                )
            }
            Self::Version { found } => write!(
                f,
                "saved rng state is stream version {found}, this build reads {STREAM_VERSION}"
            ),
            Self::ZeroState => write!(f, "saved rng state has no bits set in its four words"),
        }
    }
}

impl std::error::Error for SimError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RngState {
    pub root: u64,
    pub words: [u64; 4],
}

impl RngState {
    pub fn to_bytes(&self) -> [u8; STATE_BYTES] {
        let words = if self.words == [0; 4] {
            Rng::new(self.root).s
        } else {
            self.words
        };
        let mut out = [0u8; STATE_BYTES];
        out[..4].copy_from_slice(&STREAM_VERSION.to_le_bytes());
        out[4..12].copy_from_slice(&self.root.to_le_bytes());
        for (i, word) in words.iter().enumerate() {
            out[12 + i * 8..20 + i * 8].copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SimError> {
        if bytes.len() != STATE_BYTES {
            return Err(SimError::Length { found: bytes.len() });
        }
        let version = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        if version != STREAM_VERSION {
            return Err(SimError::Version { found: version });
        }
        let word_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let words = [word_at(12), word_at(20), word_at(28), word_at(36)];
        if words == [0; 4] {
            return Err(SimError::ZeroState);
        }
        Ok(Self {
            root: word_at(4),
            words,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    root: u64,
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut sm = seed;
        let s = [
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
            splitmix64(&mut sm),
        ];
        Self { root: seed, s }
    }

    pub fn fork(&self, label: u64) -> Self {
        Self::new(mix64(
            mix64(self.root ^ FORK_TAG).wrapping_add(mix64(label)),
        ))
    }

    pub fn state(&self) -> RngState {
        RngState {
            root: self.root,
            words: self.s,
        }
    }

    pub fn from_state(state: RngState) -> Self {
        if state.words == [0; 4] {
            return Self::new(state.root);
        }
        Self {
            root: state.root,
            s: state.words,
        }
    }

    pub fn save(&self) -> [u8; STATE_BYTES] {
        self.state().to_bytes()
    }

    pub fn restore(bytes: &[u8]) -> Result<Self, SimError> {
        RngState::from_bytes(bytes).map(Self::from_state)
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    pub fn next_bool(&mut self) -> bool {
        self.next_u64() >> 63 == 1
    }

    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * F64_UNIT
    }

    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * F32_UNIT
    }

    pub fn chance(&mut self, probability: f64) -> bool {
        self.next_f64() < probability
    }

    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below(0)");
        let mut m = (self.next_u64() as u128) * (n as u128);
        let mut low = m as u64;
        if low < n {
            let threshold = n.wrapping_neg() % n;
            while low < threshold {
                m = (self.next_u64() as u128) * (n as u128);
                low = m as u64;
            }
        }
        (m >> 64) as u64
    }

    pub fn below_u32(&mut self, n: u32) -> u32 {
        self.below(n as u64) as u32
    }

    pub fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    pub fn range_u64(&mut self, lo: u64, hi: u64) -> u64 {
        assert!(lo < hi, "range_u64 needs lo < hi");
        lo + self.below(hi - lo)
    }

    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        assert!(lo < hi, "range_i64 needs lo < hi");
        lo.wrapping_add(self.below(hi.wrapping_sub(lo) as u64) as i64)
    }

    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        self.range_i64(lo as i64, hi as i64) as i32
    }

    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    pub fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }

    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.index(items.len())])
        }
    }

    pub fn weighted_index(&mut self, weights: &[f64]) -> Option<usize> {
        let mut total = 0.0;
        for &w in weights {
            if w.is_nan() || w < 0.0 || !w.is_finite() {
                return None;
            }
            total += w;
        }
        if total.is_nan() || total <= 0.0 || !total.is_finite() {
            return None;
        }
        let target = self.next_f64() * total;
        let mut acc = 0.0;
        let mut last = None;
        for (i, &w) in weights.iter().enumerate() {
            if w > 0.0 {
                last = Some(i);
                acc += w;
                if target < acc {
                    return Some(i);
                }
            }
        }
        last
    }

    pub fn weighted_index_u32(&mut self, weights: &[u32]) -> Option<usize> {
        let total: u64 = weights.iter().map(|&w| w as u64).sum();
        if total == 0 {
            return None;
        }
        let mut target = self.below(total);
        for (i, &w) in weights.iter().enumerate() {
            if target < w as u64 {
                return Some(i);
            }
            target -= w as u64;
        }
        None
    }
}
