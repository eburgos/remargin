//! CLI + lint E2E coverage for recipient registry validation (spec task 59).
//!
//! Scenario 25: `remargin comment --to <unknown>` in a registered-mode realm
//!   exits non-zero with a message naming the bad recipient.
//! Scenario 26: `remargin lint --json` on a doc with an unknown recipient in
//!   a registered-mode realm produces `ok:false` and a recipient finding.

#[cfg(test)]
#[path = "cli_recipient_gate/tests.rs"]
mod tests;
