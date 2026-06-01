//! Per-subcommand `--config` happy-path tests.
//!
//! The previous resolver silently dropped `--config` on every
//! identity-aware subcommand. These tests lock in the fix by setting
//! up two realms: a `walker` realm whose `.remargin.yaml` declares
//! `walker-agent` and a `flag` realm whose `.remargin.yaml` declares
//! `flag-agent`. Each test runs the subcommand from inside `walker`
//! with `--config` pointing at `flag`'s yaml. Any subcommand that
//! regresses to the old overlay model will attribute the operation
//! to `walker-agent` and fail the `flag-agent` assertion.
//!
//! Tests that inspect author attribution prove `--config` end-to-end.
//! Tests that merely exercise a subcommand (write / rm / purge)
//! prove `--config` at least reaches the resolver without being
//! silently dropped.

#[cfg(test)]
#[path = "cli_config_honored/tests.rs"]
mod tests;
