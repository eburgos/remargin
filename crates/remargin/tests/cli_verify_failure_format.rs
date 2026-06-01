//! Subset-gate refusal rendering at the CLI surface. Under the subset
//! gate, mutating ops that don't introduce new anomalies (e.g. `ack`
//! against a file with a pre-existing bad checksum) must succeed.

#[cfg(test)]
#[path = "cli_verify_failure_format/tests.rs"]
mod tests;
