//! `remargin doctor --verbose` CLI integration tests.
//!
//! Verifies that `--verbose` appends a per-check summary (hook-installed
//! verdict + inspected user/project settings paths) in both the clean and
//! findings cases, and that non-verbose output and `--json` are unchanged.

#[cfg(test)]
#[path = "cli_doctor_verbose/tests.rs"]
mod tests;
