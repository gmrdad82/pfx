use pfx_core::sim::mix64;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

pub(crate) fn draw(state: &mut u64) -> u64 {
    *state = state.wrapping_add(GAMMA);
    mix64(*state)
}
