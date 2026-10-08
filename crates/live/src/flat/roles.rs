use std::num::NonZeroU64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenKey(NonZeroU64);

impl TokenKey {
    pub const fn new(name: &str) -> Self {
        let bytes = name.as_bytes();
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut index = 0;
        while index < bytes.len() {
            hash ^= bytes[index] as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            index += 1;
        }
        match NonZeroU64::new(hash) {
            Some(hash) => Self(hash),
            None => Self(NonZeroU64::MIN),
        }
    }
}

impl From<&str> for TokenKey {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<&String> for TokenKey {
    fn from(name: &String) -> Self {
        Self::new(name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Roles {
    pub fill: Option<TokenKey>,
    pub fill_end: Option<TokenKey>,
    pub stroke: Option<TokenKey>,
    pub pattern: Option<TokenKey>,
    pub glow: Option<TokenKey>,
}

impl Roles {
    pub fn is_none(&self) -> bool {
        *self == Self::default()
    }
}
