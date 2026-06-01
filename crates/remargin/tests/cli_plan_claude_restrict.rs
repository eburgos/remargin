//! `remargin plan claude restrict` integration tests.
//!
//! Mirrors the `cli_restrict.rs` patterns: real-filesystem temp dirs,
//! `assert_cmd` invocations, JSON output assertions. Covers
//! scenarios 16-22 of the testing plan: plan + apply parity,
//! the no-write invariant, the noop covenant, allow-vs-deny overlap
//! detection, anchor-surprise detection, MCP/CLI parity, and the
//! wildcard projection.

#[cfg(test)]
#[path = "cli_plan_claude_restrict/tests.rs"]
mod tests;
