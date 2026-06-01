//! Cross-mode realm hazard: caller's CWD-resolved mode dominates over the
//! doc's realm mode. A caller standing in an open-mode dir who batch-writes
//! into a strict-mode realm produces unsigned comments inside that realm,
//! which subsequently fail `remargin verify` from inside the realm.
//!
//! Reproduces a real scenario from manual testing: an agent's CWD was
//! outside the realm under test (different `~/.remargin.yaml` declared
//! `mode: open`), the doc lived in a strict-mode realm, and `remargin batch`
//! silently wrote 23 unsigned comments. The realm's `verify` invariant
//! broke until `remargin sign --all-mine` was run.

#[cfg(test)]
#[path = "cli_batch_strict_realm/tests.rs"]
mod tests;
