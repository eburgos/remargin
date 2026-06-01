//! Regression test for: `remargin comment <file> "- bullet"`
//! must accept content that starts with a hyphen.
//!
//! Clap rejects positional values starting with `-` by default. The
//! `comment` subcommand annotates `content` with `allow_hyphen_values`
//! so markdown bullets (and any other dash-led prose) round-trip.

#[cfg(test)]
#[path = "cli_comment_hyphen_content/tests.rs"]
mod tests;
