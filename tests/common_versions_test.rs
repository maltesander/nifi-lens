//! Drives the inner `#[cfg(test)]` tests inside `tests/common/versions.rs`.
//! `tests/common/` is not a test binary on its own; cargo only compiles its
//! contents when another test file declares `mod common;`.

#[path = "common/mod.rs"]
mod common;

#[test]
fn common_module_loads() {
    // Touches the const so the module is actually compiled and its inner
    // unit tests run as part of `cargo test`.
    assert!(!common::versions::FIXTURE_VERSIONS.is_empty());
}
