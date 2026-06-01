//! End-to-end permissions integration tests.
//!
//! Covers the cross-cutting scenarios from the plan that
//! the per-feature integration files (`cli_restrict.rs`,
//! `cli_unprotect.rs`, `cli_permissions.rs`) do not exercise on
//! their own:
//!
//! - E3: CLI / MCP parity for restrict.
//! - E5: multi-path (restrict A + B, unprotect A leaves B).
//! - E9: per-op no-cache (manual `.remargin.yaml` edit picked up on
//!   the very next op without a restart).
//! - E13: back-compat with realms that have no `permissions:` block.
//! - E14: dot-folder default-deny under `trusted_roots`.
//! - E15: `allow_dot_folders` override.
//! - E16: `also_deny_bash` propagates into Claude settings.
//! - E17: `--cli-allowed` omits the `Bash(remargin *)` deny rule.

#[cfg(test)]
#[path = "permissions_e2e/tests.rs"]
mod tests;
