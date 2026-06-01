//! `remargin purge` integration tests, focused on the directory form.
//!
//! The unit-test layer
//! (`crates/remargin-core/src/operations/purge/tests.rs`) covers the
//! algorithm; this surface check confirms the CLI args plumb through
//! correctly, the JSON shape is documented, and the plan projection
//! round-trips cleanly.

#[cfg(test)]
#[path = "cli_purge/tests.rs"]
mod tests;
