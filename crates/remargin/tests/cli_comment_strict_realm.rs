//! Cross-mode realm hazard for the singular `remargin comment` path.
//!
//! Companion to `cli_batch_strict_realm.rs`. Same scenario,
//! same invariant: a caller standing in an open-mode dir who writes a
//! single comment into a strict-mode realm must not leave an unsigned
//! comment in that realm. Either the write escalates to strict (signs)
//! or it refuses with a cross-mode error.

#[cfg(test)]
#[path = "cli_comment_strict_realm/tests.rs"]
mod tests;
