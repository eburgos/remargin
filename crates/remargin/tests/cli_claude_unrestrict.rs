//! `remargin claude unrestrict` integration tests.
//!
//! Exercises the CLI subcommand and the matching MCP tool against
//! real-filesystem temp dirs. Covers scenarios 12-16 of the
//! plan: end-to-end restrict + unrestrict round-trip,
//! Layer 1 enforcement transitions, wildcard cycle, --json output,
//! MCP parity.

#[cfg(test)]
#[path = "cli_claude_unrestrict/tests.rs"]
mod tests;
