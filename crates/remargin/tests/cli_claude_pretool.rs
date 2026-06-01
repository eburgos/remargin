//! `remargin claude pretool` integration tests.
//!
//! Exercises the CLI subcommand against a real-filesystem temp realm.
//! The Claude Code stdin/stdout/exit-code contract is the source of
//! truth — every test pipes an envelope into the binary and asserts on
//! stdout, stderr, and exit code.

#[cfg(test)]
#[path = "cli_claude_pretool/tests.rs"]
mod tests;
