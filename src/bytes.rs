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

/// Render a byte count using `B` / `KiB` / `MiB` with one decimal place
/// for K/M units. Used for displaying actual streaming/loaded sizes that
/// are rarely round numbers.
///
/// Examples: `format_bytes(0)` -> `"0 B"`, `format_bytes(1536)` -> `"1.5 KiB"`,
/// `format_bytes(4_500_000)` -> `"4.3 MiB"`.
pub fn format_bytes(n: u64) -> String {
    if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

/// Render a byte count using integer `B` / `KiB` / `MiB` units. Used for
/// power-of-1024 round numbers like configured streaming ceilings.
///
/// Examples: `format_bytes_int(4 * MIB)` -> `"4 MiB"`,
/// `format_bytes_int(2 * KIB)` -> `"2 KiB"`, `format_bytes_int(123)` -> `"123 B"`.
pub fn format_bytes_int(n: u64) -> String {
    if n >= MIB {
        format!("{} MiB", n / MIB)
    } else if n >= KIB {
        format!("{} KiB", n / KIB)
    } else {
        format!("{n} B")
    }
}

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

    #[test]
    fn format_bytes_fractional() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(KIB - 1), format!("{} B", KIB - 1));
        assert_eq!(format_bytes(KIB), "1.0 KiB");
        assert_eq!(format_bytes(KIB + 512), "1.5 KiB");
        assert_eq!(format_bytes(MIB), "1.0 MiB");
        assert_eq!(format_bytes(MIB + MIB / 2), "1.5 MiB");
    }

    #[test]
    fn format_bytes_int_round() {
        assert_eq!(format_bytes_int(0), "0 B");
        assert_eq!(format_bytes_int(KIB - 1), format!("{} B", KIB - 1));
        assert_eq!(format_bytes_int(KIB), "1 KiB");
        assert_eq!(format_bytes_int(2 * KIB), "2 KiB");
        assert_eq!(format_bytes_int(MIB), "1 MiB");
        assert_eq!(format_bytes_int(4 * MIB), "4 MiB");
    }
}
