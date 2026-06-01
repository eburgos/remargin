//! `remargin identity create` integration tests.
//!
//! The subcommand prints an identity YAML block to stdout. These
//! tests cover the happy path, the `--key` branch, the `--json` shape,
//! and the round-trip invariant: the emitted YAML must load cleanly as
//! a `.remargin.yaml` (no `mode:` leak, no broken shape).

#[cfg(test)]
#[path = "cli_identity_create/tests.rs"]
mod tests;
