//! Regression tests for: non-JSON mode emits no
//! `elapsed` footer on any stream; the timing value only survives as
//! the `elapsed_ms` key inside the JSON payload.
//!
//! The scenarios use `remargin resolve-mode` because it is a read-only
//! command that needs no sandbox or filesystem fixture, which keeps the
//! test hermetic.

#[cfg(test)]
#[path = "cli_streams/tests.rs"]
mod tests;
