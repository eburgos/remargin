//! End-to-end CLI tests for `--after-heading`.
//!
//! Covers the singular `comment` subcommand, the `batch` subcommand,
//! and the clap-level mutual-exclusion guard. Uses `assert_cmd` against
//! the real binary so the wiring through main.rs is exercised end to
//! end (clap → resolver → writer → on-disk markdown).

#[cfg(test)]
#[path = "cli_after_heading/tests.rs"]
mod tests;
