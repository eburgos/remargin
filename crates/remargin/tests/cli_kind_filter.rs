//! End-to-end CLI tests for the `remargin_kind` surface.
//!
//! Exercises the CLI binary to prove:
//!
//! 1. `remargin comment --kind X --kind Y` writes the tags into the
//!    YAML wire format and they round-trip.
//! 2. `remargin comments --kind X` filters the single-file listing.
//! 3. `remargin query --kind X --kind Y` applies the same filter with
//!    OR semantics across a vault.
//! 4. `remargin edit --kind Z` replaces the stored list; omitting
//!    `--kind` preserves it on content-only edits.
//!
//! Tests spin up a real `remargin` binary in a tempdir so they cover
//! the same wiring path users hit.

#[cfg(test)]
#[path = "cli_kind_filter/tests.rs"]
mod tests;
