//! `remargin plan claude unrestrict` integration tests.
//!
//! Mirrors the `cli_plan_claude_restrict.rs` patterns: real-filesystem
//! temp dirs, `assert_cmd` invocations, JSON output assertions. Covers
//! the testing-plan scenarios from the T43 spec: plan-then-act
//! parity, --json output, MCP / CLI parity, wildcard end-to-end,
//! drift detection, multi-path independence, and the no-write
//! invariant.

#[cfg(test)]
#[path = "cli_plan_claude_unrestrict/tests.rs"]
mod tests;
