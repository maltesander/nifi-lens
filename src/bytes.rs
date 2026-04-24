//! Named byte-size constants used across nifi-lens for memory,
//! repository, and streaming limits. All constants are `u64` and
//! use power-of-1024 semantics (KiB = 1024 bytes).

pub const KIB: u64 = 1024;
pub const MIB: u64 = 1024 * KIB;
pub const GIB: u64 = 1024 * MIB;

/// 512 MiB — used as a test-fixture heap-used baseline.
pub const HEAP_512_MIB: u64 = 512 * MIB;

/// 1 GiB — used as a test-fixture heap-max baseline.
pub const HEAP_1_GIB: u64 = GIB;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_power_of_1024() {
        assert_eq!(KIB, 1024);
        assert_eq!(MIB, 1024 * 1024);
        assert_eq!(GIB, 1024 * 1024 * 1024);
        assert_eq!(HEAP_512_MIB, 536_870_912);
        assert_eq!(HEAP_1_GIB, 1_073_741_824);
    }
}
