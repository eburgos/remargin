//! On-disk schema for `permissions:` in `.remargin.yaml`.

pub mod op_name;
pub mod resolve;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use self::op_name::OpName;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DenyOpsEntry {
    pub ops: Vec<OpName>,
    pub path: String,
    /// Identity filter; honored only in strict mode (open / registered
    /// realms can't trust declared identity).
    #[serde(default)]
    pub to: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Permissions {
    #[serde(default)]
    pub allow_dot_folders: Vec<String>,

    #[serde(default)]
    pub deny_ops: Vec<DenyOpsEntry>,

    /// `None` = falls back to cwd. `Some(vec![])` = locked realm, deny
    /// everything outside inherited parent roots. `Some(non-empty)` =
    /// exactly those paths reachable. `"*"` = entire declaring realm.
    #[serde(default)]
    pub trusted_roots: Option<Vec<TrustedRootEntry>>,
}

/// Bare-string or full-record on-disk form for a `trusted_roots` entry.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
#[non_exhaustive]
pub enum TrustedRootEntry {
    Full(TrustedRootEntryFull),
    Path(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TrustedRootEntryFull {
    #[serde(default)]
    pub also_deny_bash: Vec<String>,

    /// Suppress the projected `Bash(remargin *)` deny so the CLI stays
    /// usable inside this entry.
    #[serde(default)]
    pub cli_allowed: bool,

    pub path: String,
}

impl TrustedRootEntry {
    #[must_use]
    pub const fn also_deny_bash(&self) -> &[String] {
        match self {
            Self::Path(_) => &[],
            Self::Full(full) => full.also_deny_bash.as_slice(),
        }
    }

    #[must_use]
    pub const fn cli_allowed(&self) -> bool {
        match self {
            Self::Path(_) => false,
            Self::Full(full) => full.cli_allowed,
        }
    }

    #[must_use]
    pub const fn path(&self) -> &str {
        match self {
            Self::Full(full) => full.path.as_str(),
            Self::Path(s) => s.as_str(),
        }
    }
}
