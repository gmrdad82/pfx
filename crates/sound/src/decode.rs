pub const AHEAD_SECONDS: u32 = 2;

pub fn ahead_limit(rate: u32, channels: u16) -> usize {
    (rate as usize)
        .saturating_mul(channels as usize)
        .saturating_mul(AHEAD_SECONDS as usize)
}

#[cfg_attr(not(feature = "media"), allow(dead_code))]
pub(crate) fn ahead_is_full(buffered: usize, rate: u32, channels: u16) -> bool {
    buffered > ahead_limit(rate, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reader_waits_only_above_two_seconds() {
        assert_eq!(ahead_limit(48_000, 2), 192_000);
        assert_eq!(AHEAD_SECONDS, 2);
        assert!(!ahead_is_full(192_000, 48_000, 2));
        assert!(ahead_is_full(192_001, 48_000, 2));
    }
}
