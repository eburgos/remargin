//! End-to-end activity integration tests.
//!
//! Exercises the full stack — sandbox-add timestamp refresh
//!, edit-stamps-`edited_at`,
//! `gather_activity`, CLI / MCP wiring —
//! against real-filesystem temp dirs.

#[cfg(test)]
#[path = "activity_e2e/tests.rs"]
mod tests;
