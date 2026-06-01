//! Path expansion helpers.
//!
//! Single chokepoint for `~`, `$VAR`, `${VAR}`, and (on Windows)
//! `%VAR%`. Called at adapter boundaries (CLI value parser, MCP
//! dispatcher) so downstream code receives already-expanded paths.
//! Purely string-level — no canonicalization, no symlink resolution.

use std::path::PathBuf;

use thiserror::Error;

use os_shim::System;

/// Error returned by [`expand_path`] when input cannot be resolved.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExpandPathError {
    /// Syntax like `${UNCLOSED`, `${}`, or `%UNCLOSED` — the sigil started
    /// a variable reference but the reference was not terminated.
    #[error("invalid path syntax: {0}")]
    InvalidSyntax(String),

    /// An environment variable referenced by the path was not set. The
    /// wrapped string is the variable name (without sigils).
    #[error("environment variable `{0}` is not set")]
    UndefinedVariable(String),

    /// `~user/...` form is not supported — only `~` for the current user
    /// works. Users who want another user's home should write out the
    /// full path.
    #[error(
        "~{0} is not supported (only `~` for the current user works — write out the full path)"
    )]
    UnsupportedUserTilde(String),
}

/// Expand `~`, `$VAR`, `${VAR}`, and (on Windows) `%VAR%` in a path.
///
/// Returns the expanded path as a [`PathBuf`]. See the module-level docs
/// for full semantics.
///
/// Pass a [`System`] so mock filesystems can provide controlled env vars
/// in tests. The real filesystem's implementation reads from the process
/// environment.
///
/// # Errors
///
/// - [`ExpandPathError::UnsupportedUserTilde`] if the input starts with
///   `~user` (anything other than `~` alone or `~/`).
/// - [`ExpandPathError::UndefinedVariable`] if any referenced environment
///   variable is unset.
/// - [`ExpandPathError::InvalidSyntax`] for malformed variable references
///   like `${}` or an unclosed `${UNCLOSED`.
pub fn expand_path(system: &dyn System, input: &str) -> Result<PathBuf, ExpandPathError> {
    let raw = input;

    // Empty paths pass through unchanged.
    if raw.is_empty() {
        return Ok(PathBuf::new());
    }

    // Step 1: tilde at the absolute start.
    let after_tilde = expand_leading_tilde(system, raw)?;

    // Step 2: env-var substitution across the rest of the string.
    let expanded = expand_env_vars(system, &after_tilde)?;

    Ok(PathBuf::from(expanded))
}

/// Handle a leading `~` or `~/...`. Returns the input unchanged when the
/// tilde is not in the leading position.
fn expand_leading_tilde(system: &dyn System, raw: &str) -> Result<String, ExpandPathError> {
    if !raw.starts_with('~') {
        return Ok(raw.to_owned());
    }

    // `~` alone → $HOME.
    if raw.len() == 1 {
        let home = home_dir(system)?;
        return Ok(home);
    }

    // `~/...` or `~\\...` on Windows → $HOME + rest.
    let rest = &raw[1..];
    // Safe unwrap: `raw.len() > 1` above guarantees rest is non-empty.
    let Some(first) = rest.chars().next() else {
        let home = home_dir(system)?;
        return Ok(home);
    };

    if first == '/' || (cfg!(windows) && first == '\\') {
        let home = home_dir(system)?;
        return Ok(format!("{home}{rest}"));
    }

    // `~~`, `~user/...`, etc. — anything else after `~` is unsupported.
    // Strip up to the first separator (or end) so the error message names
    // the offending user token.
    let end_idx = rest
        .find(|c: char| c == '/' || (cfg!(windows) && c == '\\'))
        .unwrap_or(rest.len());
    let user = &rest[..end_idx];
    Err(ExpandPathError::UnsupportedUserTilde(user.to_owned()))
}

/// Resolve the platform home directory. POSIX uses `$HOME`; Windows prefers
/// `%USERPROFILE%` and falls back to `$HOME` for parity with the TypeScript
/// plugin's `expandPath`.
fn home_dir(system: &dyn System) -> Result<String, ExpandPathError> {
    #[cfg(windows)]
    {
        if let Ok(value) = system.env_var("USERPROFILE") {
            return Ok(value);
        }
    }
    system
        .env_var("HOME")
        .map_err(|_err| ExpandPathError::UndefinedVariable(String::from("HOME")))
}

/// Expand `$VAR`, `${VAR}`, and (on Windows) `%VAR%` references in `raw`.
///
/// POSIX `$` forms are recognized on every platform. Windows `%VAR%` is
/// only recognized when compiled for Windows — on POSIX a literal `%` is
/// preserved as-is (paths with `%` in them are legal on POSIX).
fn expand_env_vars(system: &dyn System, raw: &str) -> Result<String, ExpandPathError> {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut idx = 0;

    while idx < bytes.len() {
        let ch = bytes[idx];

        if ch == b'$' {
            idx = consume_dollar(system, raw, bytes, idx, &mut out)?;
        } else {
            #[cfg(windows)]
            if ch == b'%' {
                idx = consume_percent(system, raw, bytes, idx, &mut out)?;
                continue;
            }
            out.push(ch as char);
            idx += 1;
        }
    }

    Ok(out)
}

/// Handle a `$` at byte index `idx`. Consumes the variable reference and
/// appends the expanded value (or the literal `$` if no name follows) to
/// `out`. Returns the new index.
fn consume_dollar(
    system: &dyn System,
    raw: &str,
    bytes: &[u8],
    idx: usize,
    out: &mut String,
) -> Result<usize, ExpandPathError> {
    // `$` at end of string → literal.
    let next_idx = idx + 1;
    if next_idx >= bytes.len() {
        out.push('$');
        return Ok(next_idx);
    }

    let next = bytes[next_idx];

    // `${...}` form.
    if next == b'{' {
        let body_start = next_idx + 1;
        let Some(rel_close) = bytes[body_start..].iter().position(|b| *b == b'}') else {
            return Err(ExpandPathError::InvalidSyntax(format!(
                "unclosed `${{` in {raw:?}"
            )));
        };
        let close_idx = body_start + rel_close;
        let name = &raw[body_start..close_idx];
        if name.is_empty() {
            return Err(ExpandPathError::InvalidSyntax(format!(
                "empty `${{}}` in {raw:?}"
            )));
        }
        let value = system
            .env_var(name)
            .map_err(|_err| ExpandPathError::UndefinedVariable(name.to_owned()))?;
        out.push_str(&value);
        return Ok(close_idx + 1);
    }

    // `$VAR` form (bare name).
    let name_end = bytes[next_idx..]
        .iter()
        .position(|b| !is_var_name_byte(*b))
        .map_or(bytes.len(), |rel| next_idx + rel);
    if name_end == next_idx {
        // `$` not followed by a var character → literal `$`.
        out.push('$');
        return Ok(next_idx);
    }
    let name = &raw[next_idx..name_end];
    let value = system
        .env_var(name)
        .map_err(|_err| ExpandPathError::UndefinedVariable(name.to_owned()))?;
    out.push_str(&value);
    Ok(name_end)
}

/// Handle a `%` at byte index `idx` on Windows. `%VAR%` expands; `%` with
/// no closing `%` is `InvalidSyntax`.
#[cfg(windows)]
fn consume_percent(
    system: &dyn System,
    raw: &str,
    bytes: &[u8],
    idx: usize,
    out: &mut String,
) -> Result<usize, ExpandPathError> {
    let body_start = idx + 1;
    let Some(rel_close) = bytes[body_start..].iter().position(|b| *b == b'%') else {
        return Err(ExpandPathError::InvalidSyntax(format!(
            "unclosed `%` in {raw:?}"
        )));
    };
    let close_idx = body_start + rel_close;
    let name = &raw[body_start..close_idx];
    if name.is_empty() {
        return Err(ExpandPathError::InvalidSyntax(format!(
            "empty `%%` in {raw:?}"
        )));
    }
    let value = system
        .env_var(name)
        .map_err(|_err| ExpandPathError::UndefinedVariable(name.to_owned()))?;
    out.push_str(&value);
    Ok(close_idx + 1)
}

/// Bytes allowed in a `$VAR` bare name: ASCII letters, digits, and `_`.
/// First byte must be a letter or `_`, but the consume loop enforces the
/// same rule via the "no-chars-consumed → literal `$`" branch.
const fn is_var_name_byte(byte: u8) -> bool {
    matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
}

#[cfg(test)]
mod tests;
