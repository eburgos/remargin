//! `remargin claude restrict` integration tests.
//!
//! Exercises the CLI subcommand and the matching MCP tool against
//! real-filesystem temp dirs. Covers scenarios 14-20 of the
//! testing plan: end-to-end restrict + Layer 1
//! enforcement, settings-file and sidecar updates, gitignore
//! automation, wildcard, --json output, MCP parity.

#[cfg(test)]
#[path = "cli_claude_restrict/tests.rs"]
mod tests;
