//! Regression guard for CLI/MCP adapter bloat.
//!
//! Every mutating surface in this workspace is implemented twice: once
//! as a clap `cmd_*` helper in the CLI binary, and once as a `handle_*`
//! tool handler in the in-process MCP server. After the audit
//! (and its ), both
//! adapter layers are meant to stay genuinely thin: argument extraction
//! plus response formatting, with any non-trivial logic living once in
//! core.
//!
//! This test walks the two adapter files at compile time, counts the
//! physical lines of every `cmd_*` / `handle_*` free-standing function,
//! and asserts each is under a target cap. The cap is deliberately
//! loose (keeps the noise level low) — the value of the guard is that a
//! *new* handler cannot creep in at 90 lines without either (a) shrinking
//! below the cap by pushing logic to core, or (b) being explicitly
//! allowlisted below with a rationale. Allowlist entries force a
//! conscious decision; the test surfaces every offender so ad-hoc
//! bloat cannot slip in silently.
//!
//! To refresh after a legitimate migration: if a function drops below
//! the cap, remove its allowlist entry. If a new function intentionally
//! exceeds the cap, add it here with a one-line reason.
#[cfg(test)]
#[path = "adapter_loc_cap/tests.rs"]
mod tests;
