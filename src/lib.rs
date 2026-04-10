//! nifi-lens — a keyboard-driven TUI lens into Apache NiFi 2.x.
//!
//! The library crate holds every module except the `main` entry point.
//! Integration tests can `use nifi_lens::...` without spawning a binary.

pub mod error;

pub use error::NifiLensError;

/// Run nifi-lens. The binary calls this and maps the result to a process
/// exit code; integration tests can call into the library directly without
/// going through `main`.
pub fn run() -> std::process::ExitCode {
    // Phase 0 Task 1 leaves this as a placeholder. Subsequent tasks wire
    // CLI parsing, logging, config loading, and the TUI run loop.
    println!("nifilens {}", env!("CARGO_PKG_VERSION"));
    std::process::ExitCode::SUCCESS
}
