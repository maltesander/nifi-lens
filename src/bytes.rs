//! Named byte-size constants used across nifi-lens for memory,
//! repository, and streaming limits. All constants are `u64` and
//! use power-of-1024 semantics (KiB = 1024 bytes).

pub const KIB: u64 = 1024;
pub const MIB: u64 = 1024 * KIB;
pub const GIB: u64 = 1024 * MIB;

/// 512 MiB — test-fixture value for `heap_used_bytes` on synthetic
/// `NodeHealthRow` / snapshot inputs.
pub const FIXTURE_HEAP_USED: u64 = 512 * MIB;

/// 1 GiB — test-fixture value for `heap_max_bytes` on synthetic
/// `NodeHealthRow` / snapshot inputs.
pub const FIXTURE_HEAP_MAX: u64 = GIB;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_power_of_1024() {
        assert_eq!(KIB, 1024);
        assert_eq!(MIB, 1024 * 1024);
        assert_eq!(GIB, 1024 * 1024 * 1024);
        assert_eq!(FIXTURE_HEAP_USED, 536_870_912);
        assert_eq!(FIXTURE_HEAP_MAX, 1_073_741_824);
    }
}
