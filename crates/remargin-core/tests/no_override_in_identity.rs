//! Structural guard: System B must not come back.
//!
//! System B was the earlier identity overlay model (`CliOverrides` +
//! `ResolvedConfig::resolve(cli: &CliOverrides)` + a `cli.*.or(base.*)`
//! merge + `with_identity_overrides` + `build_overrides`). The CLI's
//! `--config` flag silently dropped on 15 subcommands because that
//! model had nowhere to put a whole-config pointer. The three-branch
//! resolver (`config::identity::resolve_identity`, System A) was
//! supposed to replace it but the CLI adapter was not rewired until
//! later.
//!
//! This test grep-gates the whole `crates/` tree. It fails CI if any of
//! the deleted System B symbols are reintroduced OR if the word
//! `override` appears inside the identity-resolution files. The
//! allowlist is empty today and exists only as an escape hatch for
//! future unrelated uses; every entry must carry an explicit reason.

#[cfg(test)]
#[path = "no_override_in_identity/tests.rs"]
mod tests;
