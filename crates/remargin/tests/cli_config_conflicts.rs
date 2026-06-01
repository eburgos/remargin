//! `--config` must clap-conflict with `--identity`,
//! `--type`, and `--key` on every identity-aware subcommand. Mixing a
//! whole-identity declaration with partial-identity flags produces the
//! "inherited-part-from-walk, replaced-part-from-flag" class of silent
//! misattribution. Clap rejects the combination at parse time
//! rather than letting the three-branch resolver see it.
//!
//! the identity group is per-subcommand (not global), so
//! the flags go AFTER the subcommand name. This file iterates over
//! every subcommand that flattens `IdentityArgs` and locks the conflict
//! in — regressing any of them would silently drop `--config` for that
//! subcommand (the exact bug that motivated this test file).
//!
//! The subcommand table below is intentionally exhaustive. If you add a
//! new subcommand with `IdentityArgs`, add it here too.

#[cfg(test)]
#[path = "cli_config_conflicts/tests.rs"]
mod tests;
