//! `remargin permissions show / check` CLI + MCP integration tests.
//!
//! Covers:
//!
//! - Scenario 20: `permissions show` and `permissions check`
//!   surface the parent-walked `.remargin.yaml` permissions correctly
//!   (text + JSON, restricted exit 0).
//! - Scenario 21: when no rules cover a path, `check` exits 1 and
//!   `show` lists the empty surface.
//! - Scenario 22: MCP `permissions_show` and `permissions_check`
//!   parity with CLI `--json` output.
//!
//! T26 (`restrict`) and T27 (`unprotect`) are not yet wired, so these
//! tests stage a `.remargin.yaml` directly. When the CLI restrict/
//! unprotect commands land, they replace the hand-written fixture with
//! a CLI invocation but the assertions stay the same.

#[cfg(test)]
#[path = "cli_permissions/tests.rs"]
mod tests;
