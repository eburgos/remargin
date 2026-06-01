//! `remargin lint` end-to-end coverage for the
//! permissions-aware op-name validation.
//!
//! - A typo in `permissions.deny_ops.ops` (`purg`) inside an ambient
//!   `.remargin.yaml` causes `remargin lint <doc>` to exit non-zero
//!   with an error message that names the offending typo and lists
//!   the valid op names.
//! - A clean `.remargin.yaml` does NOT make the doc fail to lint.
//! - `--json` mirrors the same payload through the documented schema.

#[cfg(test)]
#[path = "cli_lint_deny_ops/tests.rs"]
mod tests;
