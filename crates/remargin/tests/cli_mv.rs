//! `remargin mv` integration tests.
//!
//! Exercises the CLI subcommand against real-filesystem temp dirs.
//! The unit-test layer (`crates/remargin-core/src/operations/mv/tests.rs`)
//! covers the algorithm; this surface check confirms the CLI args
//! plumb through to it correctly, the JSON shape is documented, and
//! the plan projection round-trips cleanly.

#[cfg(test)]
#[path = "cli_mv/tests.rs"]
mod tests;
