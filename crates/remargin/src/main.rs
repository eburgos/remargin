//! Remargin CLI binary.

#[cfg(feature = "obsidian")]
mod obsidian;

pub(crate) mod handlers;
mod io;
mod params;
mod render;

use std::env;
use std::io::{Read as _, stderr as stderr_handle, stdin as stdin_handle, stdout as stdout_handle};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use os_shim::System;
use os_shim::real::RealSystem;
use serde_json::json;

use crate::io::{
    IoSinks, expand_cli_path, expand_cli_pathbuf, inject_elapsed_ms, parse_line_range,
    resolve_comment_content,
};
use crate::params::{
    AckParams, ActivityParams, CommentParams, CpParams, EditParams, GetImageParams, GetParams,
    MvParams, PromptSetParams, ReactParams, ReplaceParams, RestrictParams, SearchParams,
    SignParams, WriteParams,
};
use remargin_core::config::identity::IdentityFlags;
use remargin_core::config::{self, ResolvedConfig};
use remargin_core::document;
use remargin_core::operations;
use remargin_core::operations::replace;
use remargin_core::parser;
use remargin_core::permissions::pretool::{PretoolOutcome, pretool};

const EXIT_ERROR: u8 = 1;
const EXIT_LINT: u8 = 2;
const EXIT_INTEGRITY: u8 = 3;
const EXIT_ATTACHMENT: u8 = 4;
const EXIT_PRESERVATION: u8 = 5;
const EXIT_SKILL: u8 = 6;
const EXIT_NOT_FOUND: u8 = 7;
const EXIT_AMBIGUOUS: u8 = 8;
/// Claude Code's `PreToolUse` hook contract maps exit 2 to "block the
/// tool call and feed stderr back to the model". Use the same value
/// for fail-closed pretool outcomes so the hook signal is intact.
const EXIT_PRETOOL_FAIL: u8 = 2;
/// Marker prefix in the error message so the top-level error mapper
/// can route pretool failures to exit code 2 (Claude Code's blocking
/// signal) without mistaking them for general CLI errors.
const PRETOOL_FAIL_SENTINEL: &str = "__remargin_pretool_fail__:";
/// Gitignore-style "no match" sentinel returned by
/// `permissions check` when the path is unrestricted.
/// Numerically equal to [`EXIT_ERROR`] so existing tooling that branches
/// on `1 vs 0` still works; the `main` harness recognises the sentinel
/// to skip the "error: ..." render that would otherwise prepend the
/// gitignore-style result.
const EXIT_NOT_RESTRICTED: u8 = 1;
/// Internal marker substring used by [`cmd_permissions`] to communicate
/// "not restricted" to [`classify_error`] without leaking through
/// stderr.
pub(crate) const PERMISSIONS_NOT_RESTRICTED_MARKER: &str =
    "__remargin_permissions_check_not_restricted__";

/// Default user-scope settings file used by `remargin claude restrict`.
/// Resolved through [`expand_path`] so `$HOME` follows the active
/// [`System`] (the `obsidian` feature already exercises this pattern;
/// we follow the same approach so tests stay hermetic via the
/// `--user-settings` flag).
pub(crate) const DEFAULT_USER_SETTINGS: &str = "~/.claude/settings.json";

pub(crate) const PLUGIN_MARKETPLACE_SOURCE: &str = "tixena/remargin";
pub(crate) const PLUGIN_MARKETPLACE_NAME: &str = "remargin-marketplace";
pub(crate) const PLUGIN_REF: &str = "remargin@remargin-marketplace";

#[derive(Parser)]
#[command(
    name = "remargin",
    version,
    about = "Enhanced inline review protocol for markdown"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Per-subcommand identity group.
///
/// Flattened only into subcommands that resolve an author identity
/// (comment, edit, ack, react, sign, write, delete, batch, purge,
/// plan, verify, sandbox, mcp). Read-only / utility
/// subcommands do not flatten this group so clap rejects any attempt
/// to pass `--config` / `--identity` / `--type` / `--key` to them.
#[derive(clap::Args, Default)]
pub(crate) struct IdentityArgs {
    /// Path to the config file. Declares a complete identity on its
    /// own — conflicts with --identity, --type, and --key so a caller
    /// cannot mix "config file" and "manual declaration" halves.
    #[arg(long, conflicts_with_all = ["identity", "type", "key"])]
    config: Option<PathBuf>,

    /// Identity (author name) for this operation.
    #[arg(long)]
    identity: Option<String>,

    /// Path to signing key.
    #[arg(long)]
    key: Option<String>,

    /// Author type: human or agent.
    #[arg(long, value_name = "human|agent")]
    r#type: Option<String>,
}

/// Per-subcommand output group.
///
/// Controls how the subcommand renders its result. Flattened into
/// every subcommand that emits a payload. Unlike the old
/// `GlobalFlags`, these flags are scoped to the subcommand — this
/// matches the "per-concern, per-subcommand" structure the rest of
/// the refactor establishes. Invocations that previously placed
/// `--json` before the subcommand must now place it after.
#[derive(clap::Args, Default)]
pub(crate) struct OutputArgs {
    /// Output as JSON.
    #[arg(long)]
    json: bool,

    /// Enable verbose/tracing output.
    #[arg(long)]
    verbose: bool,
}

/// Per-subcommand `--assets-dir` flag.
///
/// Flattened ONLY into subcommands that write attachments: comment,
/// edit, batch. Everything else errors at parse time. Supplied as the
/// `assets_dir_flag` argument to
/// [`remargin_core::config::ResolvedConfig::resolve`] when set.
#[derive(clap::Args, Default)]
struct AssetsArgs {
    /// Path to assets directory.
    #[arg(long)]
    assets_dir: Option<String>,
}

/// Per-subcommand unrestricted escape hatch.
///
/// Compile-gated behind the `unrestricted` feature; flattened into the
/// ops that touch arbitrary filesystem paths (get, ls, metadata, rm,
/// write).
#[cfg(feature = "unrestricted")]
#[derive(clap::Args, Default)]
struct UnrestrictedArgs {
    /// Bypass path sandbox checks (requires compile-time feature).
    #[arg(long)]
    unrestricted: bool,
}

#[cfg(not(feature = "unrestricted"))]
#[derive(clap::Args, Default)]
struct UnrestrictedArgs;

#[cfg(not(feature = "unrestricted"))]
impl UnrestrictedArgs {
    #[expect(
        clippy::unused_self,
        reason = "sibling unrestricted-feature impl reads self.unrestricted; keep the signature uniform"
    )]
    const fn unrestricted(&self) -> bool {
        false
    }
}

#[cfg(feature = "unrestricted")]
impl UnrestrictedArgs {
    const fn unrestricted(&self) -> bool {
        self.unrestricted
    }
}

/// Available subcommands.
#[derive(clap::Subcommand)]
pub(crate) enum Commands {
    /// Acknowledge one or more comments.
    Ack {
        /// Path to the document (use - for stdin). Omit to resolve by ID across the folder tree.
        #[arg(long)]
        file: Option<String>,
        /// Comment IDs to acknowledge.
        #[arg(required = true)]
        ids: Vec<String>,
        /// Base directory to search when resolving by ID (default: .).
        #[arg(long, default_value = ".")]
        path: String,
        /// Remove this identity's ack instead of adding one.
        #[arg(long)]
        remove: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Show "what's new since X" across managed `.md` files.
    ///
    /// Walks `<path>` (file or directory; defaults to cwd) and
    /// returns per-file change records (comments, acks,
    /// sandbox-adds) sorted by ts. When `--since` is omitted, the
    /// per-file cutoff is the caller's last action in that file —
    /// files where the caller has never acted return everything.
    ///
    /// Identity is read-only here (no signature); the quartet is
    /// used only to resolve the caller name that drives the
    /// cutoff.
    Activity {
        /// Path to scan. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Cutoff timestamp (ISO 8601). Omit to derive per-file
        /// from the caller's last action.
        #[arg(long)]
        since: Option<String>,
        /// Render a human-readable timeline instead of JSON.
        #[arg(long)]
        pretty: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Create multiple comments atomically (JSON ops via --ops).
    Batch {
        /// Path to the document.
        file: String,
        /// JSON array of operations.
        #[arg(long)]
        ops: String,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        assets_args: AssetsArgs,
    },
    /// Claude Code integration: manage which paths Claude is allowed to
    /// edit + project the deny rules into both Claude settings files.
    Claude {
        /// Subcommand: `restrict`, `unrestrict`.
        #[command(subcommand)]
        action: ClaudeAction,
    },
    /// Create a comment in a document.
    Comment {
        /// Path to the document (use - for stdin).
        file: String,
        /// Comment body text (mutually exclusive with --comment-file).
        #[arg(allow_hyphen_values = true)]
        content: Option<String>,
        /// Insert after this comment ID.
        #[arg(long, conflicts_with_all = ["after_heading", "after_line"])]
        after_comment: Option<String>,
        /// Insert after the ATX heading addressed by this `>`-separated
        /// path. Setext (underline) headings are NOT
        /// supported in v1.
        #[arg(long, conflicts_with_all = ["after_comment", "after_line"])]
        after_heading: Option<String>,
        /// Insert after this line number (1-indexed).
        #[arg(long, conflicts_with_all = ["after_comment", "after_heading"])]
        after_line: Option<usize>,
        /// Attachments to include.
        #[arg(long)]
        attach: Vec<PathBuf>,
        /// Acknowledge the parent comment when replying. Default (omitted):
        /// auto-ack iff parent.author differs from the caller — replies to
        /// your own comment don't auto-ack. Pass --no-auto-ack to force skip.
        #[arg(long, conflicts_with = "no_auto_ack")]
        auto_ack: bool,
        /// Force-skip the auto-ack of the parent comment. Mutually exclusive
        /// with --auto-ack.
        #[arg(long = "no-auto-ack", conflicts_with = "auto_ack")]
        no_auto_ack: bool,
        /// Read comment body from a file (use - for stdin).
        #[arg(long, short = 'F', conflicts_with = "content")]
        comment_file: Option<PathBuf>,
        /// Classification tag for the new comment. Repeat to attach
        /// multiple (e.g. `--kind question --kind action-item`). Values
        /// must match `[A-Za-z0-9_ \-]{1,15}` — see `remargin_kind`
        /// validation in `remargin-core::kind`.
        #[arg(long = "kind")]
        remargin_kind: Vec<String>,
        /// ID of the comment to reply to.
        #[arg(long)]
        reply_to: Option<String>,
        /// Atomically stage the file in the caller's sandbox in the same write.
        #[arg(long)]
        sandbox: bool,
        /// Addressees of the comment.
        #[arg(long)]
        to: Vec<String>,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        assets_args: AssetsArgs,
    },
    /// List comments in a document.
    Comments {
        /// Path to the document (use - for stdin).
        file: String,
        /// Repeatable `remargin_kind` filter (OR semantics). Omit to
        /// return every comment regardless of tag.
        #[arg(long = "kind")]
        remargin_kind: Vec<String>,
        /// Pretty-print comments as a threaded tree.
        #[arg(long)]
        pretty: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Copy a single tracked file without touching the source.
    ///
    /// Non-markdown and comment-free markdown copy verbatim. A
    /// comment-bearing markdown file is copied body-only — the duplicate
    /// carries no comment blocks, so no cross-tree ID ambiguity and no
    /// broken signatures. The source is always left byte-for-byte unchanged.
    /// Both endpoints flow through the same `trusted_roots` / `deny_ops` /
    /// sandbox guards every other mutating op uses.
    Cp {
        /// Source path.
        src: String,
        /// Destination path.
        dst: String,
        /// Overwrite an existing destination.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
    /// Delete one or more comments.
    Delete {
        /// Path to the document.
        file: String,
        /// Comment IDs to delete.
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Run health checks on the remargin permission stack.
    ///
    /// Checks (in order):
    ///
    /// 1. **Hook-installed** — verifies the `PreToolUse` hook is wired into
    ///    Claude settings. When absent from both user- and project-scope,
    ///    no enforcement is active and subsequent checks are skipped
    ///    (all would be moot without the hook).
    ///
    /// Exit code: 0 when clean, 1 when findings are present.
    Doctor {
        /// User-scope settings file. Defaults to `~/.claude/settings.json`.
        /// Pass an explicit path to keep hermetic test runs out of the
        /// user's real home.
        #[arg(long)]
        user_settings: Option<PathBuf>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Edit a comment (cascading ack clear).
    Edit {
        /// Path to the document.
        file: String,
        /// Comment ID to edit.
        id: String,
        /// New comment body.
        content: String,
        /// Replacement classification tag list. Repeat to set multiple
        /// (e.g. `--kind question --kind action-item`). Omit every
        /// `--kind` to leave the stored tag list untouched. Pass
        /// `--kind ""` to clear — validation rejects empty strings so
        /// a single `--kind ''` errors; the right way to clear today
        /// is to run `remargin edit` without any `--kind` flags, then
        /// use the forthcoming tag editor to drop entries.
        #[arg(long = "kind")]
        remargin_kind: Vec<String>,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        assets_args: AssetsArgs,
    },
    /// Read a file's contents. Add `--binary` to fetch non-markdown files as
    /// bytes (base64 in `--json` mode, raw bytes to stdout otherwise, or
    /// written to `--out <path>`). Run `remargin metadata <path>` first to
    /// check `size_bytes` and `mime` before pulling large blobs.
    Get {
        /// Path to the file.
        path: String,
        /// Fetch as bytes. Rejects `.md` (use the text path for markdown).
        /// Mime is derived from the file extension.
        #[arg(long)]
        binary: bool,
        /// End line (1-indexed, inclusive). Text mode only.
        #[arg(long)]
        end: Option<usize>,
        /// Show line numbers in output. Text mode only.
        #[arg(short = 'n', long = "line-numbers")]
        line_numbers: bool,
        /// Write the fetched bytes to this path (binary mode only). Stdout
        /// receives a summary instead of the bytes. The caller names the
        /// target path — no auto-cleanup.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Start line (1-indexed). Text mode only.
        #[arg(long)]
        start: Option<usize>,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
    /// Return a downscaled / cropped raster image sized to fit a
    /// caller-specified byte budget. Use when `get --binary` would
    /// exceed an inline limit. Accepts PNG / JPEG / GIF / WebP.
    GetImage {
        /// Path to the image attachment.
        path: String,
        /// Optional pixel crop applied before scaling, formatted
        /// `X,Y,W,H` (origin top-left). Clamped to the image bounds.
        #[arg(long)]
        crop: Option<String>,
        /// Output format: `jpeg`, `jpg`, or `png`. Defaults to `jpeg`
        /// for photographic source formats (JPEG / WebP) and `png`
        /// for lossless source formats (PNG / GIF).
        #[arg(long)]
        format: Option<String>,
        /// Target ceiling on the encoded output size in bytes. JPEG
        /// quality is stepped down (and then the dimension cap halved)
        /// until this fits. Defaults to 262144 (256 KiB).
        #[arg(long)]
        max_bytes: Option<u64>,
        /// Upper bound (in pixels) on the longer edge of the output.
        /// Defaults to 1024.
        #[arg(long)]
        max_dimension: Option<u32>,
        /// Write the encoded bytes to this path. Stdout gets a summary
        /// instead of the bytes.
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
    /// Resolve, print, or materialize an identity.
    ///
    /// With no subcommand (or `show`), resolves and prints the
    /// effective identity under the supplied [`IdentityArgs`] — the
    /// pre-existing diagnostic surface that tooling (Obsidian plugin,
    /// scripts) polls on startup.
    ///
    /// With `create`, prints a ready-to-use identity YAML block to
    /// stdout so users can redirect into `.remargin.yaml`:
    ///
    /// ```sh
    /// remargin identity create --identity alice --type human > .remargin.yaml
    /// ```
    ///
    /// Resolution for `show` routes through the same three-branch
    /// resolver every mutating subcommand uses: `--config` (branch 1),
    /// manual `--identity/--type/--key` (branch 2), or walk-up
    /// (branch 3).
    Identity {
        /// Subcommand. Omit to invoke `show` (backward-compatible
        /// with the pre-existing surface).
        #[command(subcommand)]
        action: Option<IdentityAction>,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Generate a new Ed25519 signing key pair.
    Keygen {
        /// Output path for the private key (public key gets .pub suffix).
        #[arg(default_value = "remargin_key")]
        output: PathBuf,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Run structural lint checks.
    Lint {
        /// Path to the document (use - for stdin).
        file: String,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// List files and directories.
    Ls {
        /// Directory path to list.
        #[arg(default_value = ".")]
        path: String,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
    /// MCP server management and execution.
    Mcp {
        /// Subcommand: run, install, uninstall, test.
        #[command(subcommand)]
        action: Option<McpAction>,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Get document metadata.
    Metadata {
        /// Path to the document.
        path: String,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
    /// Move or rename a single tracked file.
    ///
    /// Same-FS moves use an atomic filesystem rename. Cross-FS moves
    /// fall back to copy + remove (the source is removed only after
    /// the destination write returns Ok). Both endpoints flow through
    /// the same sandbox / forbidden-target / per-op-guard checks every
    /// other mutating op uses, so a `restrict` entry covering either
    /// side refuses the call.
    ///
    /// Idempotent: `remargin mv a a` is a no-op; re-running after a
    /// successful move (`src` missing, `dst` already in place) returns
    /// success with `bytes_moved = 0`.
    Mv {
        /// Source path.
        src: String,
        /// Destination path.
        dst: String,
        /// Overwrite an existing destination.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
    /// Install or uninstall the Obsidian plugin in a vault.
    #[cfg(feature = "obsidian")]
    Obsidian {
        #[command(subcommand)]
        action: ObsidianAction,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Inspect the resolved permissions for the current directory.
    ///
    /// Read-only surface over `permissions::inspect`. `show` prints the
    /// parent-walked `.remargin.yaml` permissions (with `trusted_roots`
    /// recursive expansion); `check <path>` answers gitignore-style
    /// "is this path restricted?" with exit-code semantics
    /// (0 = restricted, 1 = not).
    ///
    /// No identity flags — both subcommands are pure observers.
    Permissions {
        #[command(subcommand)]
        action: PermissionsAction,
    },
    /// Structured pre-commit prediction for a mutating op.
    ///
    /// Per-op subcommand routing wires this to the in-memory projection
    /// of each mutating op. This crate ships the shared shape +
    /// subcommand tree; individual op wiring lands in follow-ups.
    ///
    /// Identity is flattened on the parent so every projection inherits
    /// the same `--identity` / `--type` / `--config` / `--key`. Output
    /// flags, by contrast, belong on each sub-action so `remargin plan
    /// <op> … --json` parses cleanly.
    Plan {
        /// Which mutating op to plan.
        #[command(subcommand)]
        action: PlanAction,
        #[command(flatten)]
        identity_args: IdentityArgs,
    },
    /// Folder-scoped system-prompt resolver.
    ///
    /// Read-only walk-up that mirrors the identity resolver but anchors
    /// on the `system_prompt:` block of a `.remargin.yaml`. Identity
    /// flags are accepted for surface symmetry and never gate the
    /// resolution. Falls through to the locked Default body when the
    /// walk exhausts.
    Prompt {
        /// Subcommand: resolve.
        #[command(subcommand)]
        action: PromptAction,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Strip all comments from a document.
    ///
    /// With `--recursive`, treat `file` as a directory and purge every
    /// visible markdown file under it. Per-file `op_guard`
    /// checks fire individually so a single `deny_ops` or allow-list
    /// refusal does not abort the whole batch.
    Purge {
        /// Path to the document (or directory when `--recursive` is set).
        file: String,
        /// Recursively purge every `.md` file under the directory at `file`.
        #[arg(long)]
        recursive: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Search across documents for comments.
    Query {
        /// Base directory to search.
        #[arg(default_value = ".")]
        path: String,
        /// Only documents with comments by this author.
        #[arg(long)]
        author: Option<String>,
        /// Only documents containing a comment with this structural ID.
        #[arg(long)]
        comment_id: Option<String>,
        /// Regex applied to comment content; composes with metadata filters.
        #[arg(long)]
        content_regex: Option<String>,
        /// Include individual matching comments in each result (default behavior).
        #[arg(long)]
        expanded: bool,
        /// Case-insensitive match for `--content-regex`.
        #[arg(long, short = 'i')]
        ignore_case: bool,
        /// Only documents with pending (unacked) comments. Matches
        /// both directed (unacked recipients) and broadcast (no acks
        /// at all) shapes.
        #[arg(long)]
        pending: bool,
        /// Only surface unacked broadcast (no-`to`) comments the
        /// current identity has not acknowledged. Resolves the
        /// identity the same way every other subcommand does.
        #[arg(long)]
        pending_broadcast: bool,
        /// Only pending for this recipient.
        #[arg(long)]
        pending_for: Option<String>,
        /// Sugar for `--pending-for <current-identity>`. Surfaces
        /// directed comments addressed to the caller that the caller
        /// has not acked yet.
        #[arg(long)]
        pending_for_me: bool,
        /// Pretty-print results grouped by file.
        #[arg(long)]
        pretty: bool,
        /// Repeatable `remargin_kind` filter (OR semantics). Matches any
        /// comment whose tag list contains at least one of the supplied
        /// values.
        #[arg(long = "kind")]
        remargin_kind: Vec<String>,
        /// Only activity after this ISO 8601 timestamp.
        #[arg(long)]
        since: Option<String>,
        /// Return only counts/summary, suppress comment data.
        #[arg(long)]
        summary: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Add or remove an emoji reaction.
    React {
        /// Path to the document.
        file: String,
        /// Comment ID.
        id: String,
        /// Emoji to add/remove.
        emoji: String,
        /// Remove instead of add.
        #[arg(long)]
        remove: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Manage the registry file.
    Registry {
        /// Subcommand: show.
        #[command(subcommand)]
        action: RegistryAction,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Find/replace across document body text (never inside comments).
    ///
    /// Substitutes `PATTERN` with `REPLACEMENT` in document body text
    /// only, over a single file or a whole directory tree. Comment
    /// blocks are never in scope — a pattern that occurs only inside a
    /// comment is a no-op, and a comment is left byte-identical even
    /// when the body around it changes. Each per-file write flows
    /// through the same comment-preservation and post-verify subset gate
    /// `write` uses, so a replace can never corrupt a comment or
    /// introduce an integrity anomaly. In folder mode, a file the gate
    /// refuses is skipped and recorded; the run finishes the rest.
    Replace {
        /// Text or regex to find.
        pattern: String,
        /// Replacement text. In `--regex` mode, `$1` / `${name}` expand
        /// to capture groups; otherwise the text is inserted verbatim.
        replacement: String,
        /// Target file or directory.
        #[arg(long, default_value = ".")]
        path: String,
        /// Treat pattern as a regex (default: literal).
        #[arg(long)]
        regex: bool,
        /// Case-insensitive matching.
        #[arg(long, short = 'i')]
        ignore_case: bool,
        /// Report per-file replacement counts and the subset-gate
        /// verdict; write nothing.
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
    /// Resolve the effective enforcement mode for a directory.
    ///
    /// Walks up from `--cwd` (or the current directory) looking for the
    /// nearest `.remargin.yaml` and returns its `mode:` field. Unlike
    /// `identity`, this does NOT filter by author type — mode is a
    /// directory-tree property. Falls back to `open` when no config is found.
    ///
    /// Prints a JSON object like `{"mode":"strict","source":"/path/to/.remargin.yaml"}`
    /// under `--json`; prints a short human-readable summary otherwise.
    ResolveMode {
        /// Starting directory for the walk-up. Defaults to the process's
        /// current directory.
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Remove a file from the managed document tree.
    Rm {
        /// Path to the file.
        file: String,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
    /// Manage per-identity sandbox staging for markdown files.
    Sandbox {
        /// Subcommand: add, list, or remove.
        #[command(subcommand)]
        action: SandboxAction,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Search across documents for text matches.
    Search {
        /// Text or regex pattern to search for.
        pattern: String,
        /// Base directory to search.
        #[arg(long, default_value = ".")]
        path: String,
        /// Treat pattern as a regex.
        #[arg(long)]
        regex: bool,
        /// Search scope: all, body, or comments.
        #[arg(long, default_value = "all")]
        scope: String,
        /// Lines of context around matches.
        #[arg(long, short = 'C', default_value = "0")]
        context: usize,
        /// Case-insensitive matching.
        #[arg(long, short = 'i')]
        ignore_case: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Back-sign missing-signature comments authored by the current
    /// identity.
    ///
    /// Adds an SSH signature to each selected comment. The canonical
    /// signed payload excludes ack / reactions / checksum, so signing
    /// never invalidates an existing comment — it only promotes an
    /// unsigned artifact into one that verifies under the
    /// participant-registry pubkey.
    ///
    /// The op refuses to sign comments authored by anyone other than
    /// the resolved identity (forgery guard). Already-signed comments
    /// listed under `--ids` are reported as skipped, not re-signed;
    /// `--all-mine` silently excludes them.
    Sign {
        /// Path to the document.
        file: String,
        /// Comment ids to sign. Mutually exclusive with `--all-mine`.
        #[arg(long, value_delimiter = ',', conflicts_with = "all_mine")]
        ids: Vec<String>,
        /// Sign every unsigned comment authored by the current
        /// identity. Mutually exclusive with `--ids`.
        #[arg(long)]
        all_mine: bool,
        /// Recompute each target comment's stored checksum from its
        /// current content before signing. The forgery guard still
        /// applies — you can only repair comments you authored.
        /// Without this flag a stale checksum fails the verify gate
        /// and the op refuses to sign.
        #[arg(long)]
        repair_checksum: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Verify comment integrity (checksums and signatures) against the
    /// participant registry.
    ///
    /// No flags: the registry is the single source of truth for pubkeys.
    /// Per-comment resolution runs unconditionally and the aggregate
    /// pass/fail follows the mode-driven severity table (see
    /// `operations::verify`).
    Verify {
        /// Path to the document.
        file: String,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Print version information.
    Version,
    /// Write document contents (comment-preserving).
    Write {
        /// Path to the file.
        path: String,
        /// File content to write (read from stdin if omitted).
        content: Option<String>,
        /// Content is base64-encoded binary data (implies --raw).
        /// Not supported for markdown (.md) files.
        #[arg(long)]
        binary: bool,
        /// Create a new file, creating any missing parent directories; the file itself must not already exist.
        #[arg(long)]
        create: bool,
        /// Replace only lines `START-END` (1-indexed, inclusive) and leave
        /// every other line byte-identical. Comment blocks inside the
        /// range must be reincluded (by id) in the replacement; writes
        /// that would destroy a comment are rejected. Incompatible with
        /// --create, --raw, and --binary.
        #[arg(long, value_name = "START-END")]
        lines: Option<String>,
        /// Write content exactly as provided, skipping frontmatter and comment
        /// preservation. Not supported for markdown (.md) files.
        #[arg(long)]
        raw: bool,
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
        #[command(flatten)]
        unrestricted_args: UnrestrictedArgs,
    },
}

/// `remargin claude` subcommands. Cohesion bucket for ops whose
/// effects are scoped entirely to Claude Code's permission surface
/// (`.claude/settings.local.json`, `~/.claude/settings.json`, and the
/// `.remargin-restrictions.json` sidecar).
#[derive(clap::Subcommand)]
pub(crate) enum ClaudeAction {
    /// Manage the remargin Claude Code plugin.
    Plugin {
        /// Subcommand: install, uninstall, test.
        #[command(subcommand)]
        action: PluginAction,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Claude Code `PreToolUse` hook surface.
    ///
    /// With no subcommand (or `dispatch`), reads a `PreToolUse` event
    /// JSON envelope from stdin and emits Claude Code's decision JSON
    /// on stdout. `install` / `uninstall` / `test` manage the hook
    /// entry in the target Claude settings file.
    Pretool {
        #[command(subcommand)]
        action: Option<PretoolAction>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Restrict an agent-edit subpath.
    ///
    /// Adds a `permissions.trusted_roots` entry to the nearest
    /// `.claude/`-bearing ancestor's `.remargin.yaml` and projects
    /// the equivalent rules into both Claude settings files
    /// (`<anchor>/.claude/settings.local.json` and
    /// `~/.claude/settings.json`). Idempotent.
    ///
    /// No identity flags — `restrict` is a sanctioned config write
    /// that the user is presumed to have authority over.
    Restrict {
        /// Subpath relative to the anchor, OR the literal `*` for
        /// realm-wide.
        path: String,
        /// Extra Bash commands to deny on the restricted path. The
        /// default deny list already covers every common
        /// file-modifying command surface (`rm`, `chmod`, editors,
        /// scriptable interpreters, archivers, shells, VCS, etc. —
        /// see `BASH_MUTATORS` in `claude_sync.rs`); this flag is for
        /// project-specific extras the defaults do not catch.
        /// Comma-separated or repeat the flag:
        /// `--also-deny-bash curl,wget` or
        /// `--also-deny-bash curl --also-deny-bash wget`.
        /// Both forms are equivalent.
        #[arg(long = "also-deny-bash", value_delimiter = ',')]
        also_deny_bash: Vec<String>,
        /// When set, allow `Bash(remargin *)` on the path so the CLI
        /// stays usable. The MCP / agent surfaces are still blocked.
        #[arg(long)]
        cli_allowed: bool,
        /// User-scope settings file. Defaults to
        /// `~/.claude/settings.json`. Pass an explicit path to keep
        /// hermetic test runs out of the user's real home.
        #[arg(long)]
        user_settings: Option<PathBuf>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Reverse a previous `claude restrict`.
    ///
    /// Removes the matching `permissions.trusted_roots` entry from the
    /// nearest `.claude/`-bearing ancestor's `.remargin.yaml` AND
    /// scrubs the sidecar-tracked rules from both Claude settings
    /// files. Idempotent. Surfaces manual-edit divergences as
    /// warnings (never errors).
    ///
    /// No identity flags — symmetric with `restrict`.
    Unrestrict {
        /// Subpath to unrestrict (matches the on-disk `path` field of
        /// the original restrict entry), OR the literal `*` for the
        /// realm-wide wildcard restrict.
        path: String,
        /// Exit non-zero when `<path>` is not currently restricted
        /// instead of the default warn-and-no-op. For scripts that
        /// want hard-fail-on-miss semantics.
        #[arg(long)]
        strict: bool,
        /// User-scope settings file. Defaults to
        /// `~/.claude/settings.json`. Symmetric with `restrict`'s
        /// flag so hermetic test runs can stay out of the user's
        /// real home.
        #[arg(long)]
        user_settings: Option<PathBuf>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
}

/// Registry subcommands.
/// Plan subcommands. One variant per mutating op; per-op
/// wiring is tracked /.
#[derive(clap::Subcommand)]
pub(crate) enum PlanAction {
    /// Project an `ack` op.
    Ack {
        /// Path to the document.
        path: String,
        /// Comment IDs to ack (or un-ack with `--remove`).
        #[arg(required = true)]
        ids: Vec<String>,
        /// Remove the current identity's ack from each comment.
        #[arg(long)]
        remove: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `batch` op.
    ///
    /// Reads the sub-op list from a JSON file (same shape as the
    /// `batch` subcommand): an array of objects with `content` (required)
    /// plus optional `reply_to`, `after_comment`, `after_line`,
    /// `attach_names`, `auto_ack`, `to`.
    Batch {
        /// Path to the document.
        path: String,
        /// JSON file containing the `ops` array. Use `-` for stdin.
        ops_file: String,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `claude restrict` / `claude unrestrict` op.
    ///
    /// Mirrors `remargin claude <op>` arg-for-arg; routes through the
    /// canonical plan dispatcher. Surfaces the multi-file config diff
    /// (`.remargin.yaml`, project + user settings, sidecar) and any
    /// detectable conflicts. No flags are consumed or written.
    Claude {
        /// Subcommand: `restrict` or `unrestrict`.
        #[command(subcommand)]
        action: PlanClaudeAction,
    },
    /// Project a `comment` creation op.
    Comment {
        /// Path to the document.
        path: String,
        /// Comment body text (read from stdin if omitted).
        content: Option<String>,
        /// Insert after this comment ID.
        #[arg(long, conflicts_with_all = ["after_heading", "after_line"])]
        after_comment: Option<String>,
        /// Project insertion after the ATX heading addressed by this
        /// `>`-separated path. Setext (underline) headings
        /// are NOT supported in v1.
        #[arg(long, conflicts_with_all = ["after_comment", "after_line"])]
        after_heading: Option<String>,
        /// Insert after this line number (1-indexed).
        #[arg(long, conflicts_with_all = ["after_comment", "after_heading"])]
        after_line: Option<usize>,
        /// Attachment basenames to record on the projected comment.
        /// Bytes are *not* copied — `plan` stays side-effect-free. The
        /// caller is responsible for the corresponding files existing
        /// when the mutating `comment` op runs.
        #[arg(long = "attach-name")]
        attach_names: Vec<String>,
        /// Acknowledge the parent comment when replying. Default (omitted):
        /// auto-ack iff parent.author differs from the caller. Pass
        /// --no-auto-ack to force skip.
        #[arg(long, conflicts_with = "no_auto_ack")]
        auto_ack: bool,
        /// Force-skip the auto-ack of the parent comment. Mutually exclusive
        /// with --auto-ack.
        #[arg(long = "no-auto-ack", conflicts_with = "auto_ack")]
        no_auto_ack: bool,
        /// ID of the comment to reply to.
        #[arg(long)]
        reply_to: Option<String>,
        /// Atomically project a sandbox entry in the frontmatter.
        #[arg(long)]
        sandbox: bool,
        /// Addressees of the comment.
        #[arg(long)]
        to: Vec<String>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `cp` op.
    ///
    /// Surfaces the canonical src/dst, whether the destination exists
    /// (and would therefore require `--force`), the copy kind
    /// (`verbatim`, `body_only`, or `noop`), and the number of comment
    /// blocks that would be dropped. No bytes are written — dry-run only.
    Cp {
        /// Source path.
        src: String,
        /// Destination path.
        dst: String,
        /// Project the `--force` semantics.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `delete` op.
    Delete {
        /// Path to the document.
        path: String,
        /// Comment IDs to delete.
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project an `edit` op.
    Edit {
        /// Path to the document.
        path: String,
        /// Comment ID to edit.
        id: String,
        /// New comment body.
        content: String,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project an `mv` op.
    ///
    /// Surfaces the canonical src/dst, whether the destination exists
    /// (and would therefore require `--force`), and whether the live
    /// op would settle as a no-op (same canonical path) or
    /// idempotently as a no-op (src missing, dst already in place).
    /// No bytes are moved, no markdown is rewritten — `mv` does not
    /// change document content.
    Mv {
        /// Source path.
        src: String,
        /// Destination path.
        dst: String,
        /// Project the `--force` semantics.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `purge` op. Pass `--recursive` to project
    /// a directory-level purge.
    Purge {
        /// Path to the document (or directory when `--recursive` is set).
        path: String,
        /// Project a recursive purge over every visible `.md` file
        /// under the directory at `path`.
        #[arg(long)]
        recursive: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `react` op.
    React {
        /// Path to the document.
        path: String,
        /// Comment ID to react to.
        id: String,
        /// Emoji to add (or remove with `--remove`).
        emoji: String,
        /// Remove the current identity's reaction with this emoji.
        #[arg(long)]
        remove: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `sandbox add` op.
    SandboxAdd {
        /// Path to the document.
        path: String,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `sandbox remove` op.
    SandboxRemove {
        /// Path to the document.
        path: String,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `sign` op.
    Sign {
        /// Path to the document.
        path: String,
        /// Comment ids to sign. Mutually exclusive with `--all-mine`.
        #[arg(long, value_delimiter = ',', conflicts_with = "all_mine")]
        ids: Vec<String>,
        /// Sign every unsigned comment authored by the current
        /// identity. Mutually exclusive with `--ids`.
        #[arg(long)]
        all_mine: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `write` op.
    Write {
        /// Path to the file.
        path: String,
        /// File content to write (read from stdin if omitted).
        content: Option<String>,
        /// Content is base64-encoded binary data (implies --raw). Not
        /// supported for markdown (.md) files and not representable as
        /// a structured plan — the report will carry a `reject_reason`.
        #[arg(long)]
        binary: bool,
        /// Create a new file, creating any missing parent directories; the file itself must not already exist.
        #[arg(long)]
        create: bool,
        /// Replace only lines `START-END` (1-indexed, inclusive) and
        /// leave every other line byte-identical. See `write --lines`
        /// for the full semantics.
        #[arg(long, value_name = "START-END")]
        lines: Option<String>,
        /// Write content exactly as provided, skipping frontmatter and
        /// comment preservation. Not supported for markdown (.md) files
        /// and not representable as a structured plan — the report will
        /// carry a `reject_reason`.
        #[arg(long)]
        raw: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
}

/// `remargin plan claude` subcommands. Mirror the live `ClaudeAction`
/// shape; route through the canonical plan dispatcher.
#[derive(clap::Subcommand)]
pub(crate) enum PlanClaudeAction {
    /// Project a `claude restrict` op.
    Restrict {
        /// Subpath relative to the anchor, OR the literal `*` for
        /// realm-wide. Same shape as `remargin claude restrict`.
        path: String,
        /// Extra Bash commands to deny on the restricted path,
        /// layered on top of the broad default deny list.
        /// Comma-separated or repeat the flag.
        #[arg(long = "also-deny-bash", value_delimiter = ',')]
        also_deny_bash: Vec<String>,
        /// When set, the projection allows `Bash(remargin *)` on the
        /// path so the CLI stays usable.
        #[arg(long)]
        cli_allowed: bool,
        /// User-scope settings file. Defaults to
        /// `~/.claude/settings.json`. Pin an explicit path for
        /// hermetic test runs.
        #[arg(long)]
        user_settings: Option<PathBuf>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Project a `claude unrestrict` op.
    Unrestrict {
        /// Subpath relative to the anchor (matches the on-disk
        /// `path` field of the original restrict entry), OR the
        /// literal `*` for realm-wide. Same shape as `remargin
        /// claude unrestrict`.
        path: String,
        /// User-scope settings file. Defaults to
        /// `~/.claude/settings.json`. Accepted for surface symmetry
        /// but not consulted by the projection (the sidecar's
        /// `added_to_files` list pins the actual targets).
        #[arg(long)]
        user_settings: Option<PathBuf>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
}

#[derive(clap::Subcommand)]
pub(crate) enum RegistryAction {
    /// Show the current registry.
    Show,
}

/// `remargin permissions` subcommands.
#[derive(clap::Subcommand)]
pub(crate) enum PermissionsAction {
    /// Gitignore-style: exit 0 when `path` is restricted, 1 otherwise.
    Check {
        /// Path to test.
        path: PathBuf,
        /// Print the matching rule (kind, source file, rule text) when
        /// the path is restricted. Adds detail to both text and JSON
        /// output.
        #[arg(long)]
        why: bool,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Print the resolved permissions for the current directory.
    Show {
        #[command(flatten)]
        output_args: OutputArgs,
    },
}

/// `remargin identity` subcommands. Default action
/// (no subcommand) is `show` — the pre-existing diagnostic surface.
#[derive(clap::Subcommand)]
pub(crate) enum IdentityAction {
    /// Print a ready-to-use identity YAML block to stdout. Users
    /// redirect to `.remargin.yaml` themselves (no `--write` flag —
    /// bans writes to `.remargin.yaml`).
    ///
    /// `--identity` and `--type` are required; `--key` is optional
    /// (valid in non-strict modes — pairs with `remargin keygen`).
    /// `mode:` is never emitted because mode is a tree property
    /// resolved by walk-up, not an identity-level declaration.
    Create {
        /// Identity (author name) to record.
        #[arg(long)]
        identity: String,
        /// Author type (`human` or `agent`).
        #[arg(long, value_name = "human|agent")]
        r#type: String,
        /// Optional path to the signing key. Emitted verbatim into
        /// the YAML — no existence check (pairs with `remargin
        /// keygen`). Bare names like `mykey` are fine; `.remargin.yaml`
        /// resolves them against `~/.ssh/` at load time.
        #[arg(long)]
        key: Option<String>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Resolve and print the effective identity (pre-existing
    /// behavior). Kept as an explicit alternative to the bare
    /// `remargin identity` form.
    Show {
        #[command(flatten)]
        identity_args: IdentityArgs,
        #[command(flatten)]
        output_args: OutputArgs,
    },
}

/// `remargin prompt` subcommands. Room for `set` / `unset` / `list`
/// later — only `resolve` ships in v1 (the inline editor lives in the
/// Obsidian plugin).
#[derive(clap::Subcommand)]
pub(crate) enum PromptAction {
    /// Strip the `system_prompt:` block from `<folder>/.remargin.yaml`.
    /// Idempotent: a missing block (or missing file) succeeds. The
    /// `.remargin.yaml` file is preserved even if it ends up empty.
    Delete {
        /// Folder containing the `.remargin.yaml`. Defaults to CWD.
        #[arg(default_value = ".")]
        folder: String,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Recursively list every `.remargin.yaml` under the given folder
    /// that declares a `system_prompt:` block.
    List {
        /// Root folder. Defaults to CWD.
        #[arg(default_value = ".")]
        folder: String,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Resolve the nearest folder-scoped system prompt for a file or
    /// directory. Falls through to the locked Default body when the
    /// walk exhausts.
    Resolve {
        /// File or directory to resolve a prompt for. Directories are
        /// treated as the starting directory; files have their parent
        /// walked.
        file: String,
        #[command(flatten)]
        output_args: OutputArgs,
    },
    /// Create or replace the `system_prompt:` block in
    /// `<folder>/.remargin.yaml`. Body comes from `--prompt` when set,
    /// else stdin (when stdin is not a TTY).
    Set {
        /// Folder containing (or to contain) the `.remargin.yaml`.
        /// Defaults to the current working directory when omitted.
        #[arg(default_value = ".")]
        folder: String,
        /// Human-readable display label. Required.
        #[arg(long)]
        name: String,
        /// Prompt body. Required. Pass `-` (or omit) and pipe via
        /// stdin for multi-line content.
        #[arg(long)]
        prompt: Option<String>,
        #[command(flatten)]
        output_args: OutputArgs,
    },
}

/// Sandbox subcommands.
#[derive(clap::Subcommand)]
pub(crate) enum SandboxAction {
    /// Stage one or more markdown files in the caller's sandbox.
    Add {
        /// Markdown files to stage.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// List every markdown file staged for the caller.
    List {
        /// Emit absolute paths instead of paths relative to `--path`.
        #[arg(long)]
        absolute: bool,
        /// Base directory to walk (defaults to current directory).
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Remove the caller's sandbox entry from one or more markdown files.
    Remove {
        /// Markdown files to unstage.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

/// `remargin claude pretool` subcommands.
#[derive(clap::Subcommand)]
pub(crate) enum PretoolAction {
    /// Read a `PreToolUse` event from stdin and emit the decision JSON.
    Dispatch,
    /// Wire the `PreToolUse` hook into `~/.claude/settings.json`
    /// (default) or `.claude/settings.json` with `--local`.
    Install {
        #[arg(long)]
        local: bool,
    },
    /// Report whether the `PreToolUse` hook is wired.
    Test {
        #[arg(long)]
        local: bool,
    },
    /// Remove the `PreToolUse` hook entry. Preserves unrelated entries.
    Uninstall {
        #[arg(long)]
        local: bool,
    },
}

/// Plugin subcommands.
#[derive(clap::Subcommand)]
pub(crate) enum PluginAction {
    /// Register the marketplace and install the remargin plugin.
    Install {
        /// Install at project scope instead of the default user scope.
        #[arg(long)]
        local: bool,
    },
    /// Check plugin installation status.
    Test {
        /// Check at project scope instead of the default user scope.
        #[arg(long)]
        local: bool,
    },
    /// Uninstall the remargin plugin.
    Uninstall {
        /// Uninstall from project scope instead of the default user scope.
        #[arg(long)]
        local: bool,
    },
}

/// MCP subcommands.
#[derive(clap::Subcommand)]
pub(crate) enum McpAction {
    /// Register remargin as an MCP server in Claude Code.
    Install {
        /// Install at user scope (default is project scope).
        #[arg(long)]
        user: bool,
    },
    /// Start the MCP server (stdio transport). This is the default.
    Run,
    /// Check MCP registration status.
    Test,
    /// Remove remargin MCP server from Claude Code.
    Uninstall,
}

/// Obsidian plugin install/uninstall actions.
#[cfg(feature = "obsidian")]
#[derive(clap::Subcommand)]
pub(crate) enum ObsidianAction {
    /// Install or upgrade the plugin in the current vault.
    Install {
        /// Vault directory. Defaults to the current working directory.
        #[arg(long)]
        vault_path: Option<PathBuf>,
    },
    /// Remove the plugin from the current vault.
    Uninstall {
        /// Vault directory. Defaults to the current working directory.
        #[arg(long)]
        vault_path: Option<PathBuf>,
    },
}

pub(crate) const fn author_type_str(at: &parser::AuthorType) -> &'static str {
    at.as_str()
}

/// Pull the subcommand's [`OutputArgs`] reference for the top-level
/// harness (main + error rendering).
///
/// Returns `None` for subcommands that do not flatten [`OutputArgs`] —
/// currently only `Version`. Callers treat `None` as "no `--json`, no
/// `--verbose`" (the all-defaults case).
const fn subcommand_output(cmd: &Commands) -> Option<&OutputArgs> {
    match cmd {
        Commands::Ack { output_args, .. }
        | Commands::Activity { output_args, .. }
        | Commands::Batch { output_args, .. }
        | Commands::Comment { output_args, .. }
        | Commands::Comments { output_args, .. }
        | Commands::Delete { output_args, .. }
        | Commands::Doctor { output_args, .. }
        | Commands::Edit { output_args, .. }
        | Commands::Get { output_args, .. }
        | Commands::Identity { output_args, .. }
        | Commands::Keygen { output_args, .. }
        | Commands::Lint { output_args, .. }
        | Commands::Ls { output_args, .. }
        | Commands::Cp { output_args, .. }
        | Commands::Mcp { output_args, .. }
        | Commands::Metadata { output_args, .. }
        | Commands::Mv { output_args, .. }
        | Commands::Prompt { output_args, .. }
        | Commands::Purge { output_args, .. }
        | Commands::Query { output_args, .. }
        | Commands::React { output_args, .. }
        | Commands::Replace { output_args, .. }
        | Commands::Registry { output_args, .. }
        | Commands::ResolveMode { output_args, .. }
        | Commands::Rm { output_args, .. }
        | Commands::GetImage { output_args, .. }
        | Commands::Sandbox { output_args, .. }
        | Commands::Search { output_args, .. }
        | Commands::Sign { output_args, .. }
        | Commands::Verify { output_args, .. }
        | Commands::Write { output_args, .. } => Some(output_args),
        #[cfg(feature = "obsidian")]
        Commands::Obsidian { output_args, .. } => Some(output_args),
        Commands::Claude { action } => Some(claude_action_output(action)),
        Commands::Permissions { action } => Some(permissions_action_output(action)),
        Commands::Plan { action, .. } => Some(plan_action_output(action)),
        Commands::Version => None,
    }
}

/// Pull the per-action [`OutputArgs`] from a [`ClaudeAction`] variant.
const fn claude_action_output(action: &ClaudeAction) -> &OutputArgs {
    match action {
        ClaudeAction::Plugin { output_args, .. }
        | ClaudeAction::Pretool { output_args, .. }
        | ClaudeAction::Restrict { output_args, .. }
        | ClaudeAction::Unrestrict { output_args, .. } => output_args,
    }
}

/// Pull the per-action [`OutputArgs`] from a [`PlanClaudeAction`] variant.
const fn plan_claude_action_output(action: &PlanClaudeAction) -> &OutputArgs {
    match action {
        PlanClaudeAction::Restrict { output_args, .. }
        | PlanClaudeAction::Unrestrict { output_args, .. } => output_args,
    }
}

/// Pull the per-action [`OutputArgs`] from a [`PermissionsAction`]
/// variant. Both `show` and `check` flatten an `OutputArgs`.
const fn permissions_action_output(action: &PermissionsAction) -> &OutputArgs {
    match action {
        PermissionsAction::Show { output_args } | PermissionsAction::Check { output_args, .. } => {
            output_args
        }
    }
}

/// Pull the per-action [`OutputArgs`] from a [`PlanAction`] variant.
/// Every plan sub-action flattens an `OutputArgs`.
const fn plan_action_output(action: &PlanAction) -> &OutputArgs {
    match action {
        PlanAction::Ack { output_args, .. }
        | PlanAction::Batch { output_args, .. }
        | PlanAction::Comment { output_args, .. }
        | PlanAction::Cp { output_args, .. }
        | PlanAction::Delete { output_args, .. }
        | PlanAction::Edit { output_args, .. }
        | PlanAction::Mv { output_args, .. }
        | PlanAction::Purge { output_args, .. }
        | PlanAction::React { output_args, .. }
        | PlanAction::SandboxAdd { output_args, .. }
        | PlanAction::SandboxRemove { output_args, .. }
        | PlanAction::Sign { output_args, .. }
        | PlanAction::Write { output_args, .. } => output_args,
        PlanAction::Claude { action: claude } => plan_claude_action_output(claude),
    }
}

fn main() -> ExitCode {
    // Capture the start time before parsing so `elapsed_ms` includes clap's
    // argument-parsing overhead.
    let _: Result<_, _> = io::START_TIME.set(Instant::now());

    let cli = Cli::parse();

    let output = subcommand_output(&cli.command);
    let verbose = output.is_some_and(|o| o.verbose);

    let system = RealSystem::new();
    if verbose {
        let env_filter_directives = system.env_var("RUST_LOG").unwrap_or_default();
        let base_filter = tracing_subscriber::EnvFilter::try_new(&env_filter_directives)
            .unwrap_or_else(|_err| tracing_subscriber::EnvFilter::new(""));
        tracing_subscriber::fmt()
            .with_env_filter(base_filter.add_directive(tracing::Level::DEBUG.into()))
            .with_writer(stderr_handle)
            .init();
    }

    let cwd = match system.current_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("error: could not determine current directory: {err}");
            return ExitCode::from(EXIT_ERROR);
        }
    };

    let mut stdout = stdout_handle().lock();
    let mut stderr = stderr_handle().lock();
    let mut sinks = IoSinks::new(&mut stdout, &mut stderr);

    // Non-JSON mode does not emit a timing footer on any stream:
    // stdout stays pure command output and stderr stays clean. The timing
    // value survives as `elapsed_ms` inside the JSON payload.
    run(&cli, &system, &cwd, &mut sinks)
}

fn classify_error(err: &anyhow::Error) -> u8 {
    let msg = format!("{err:#}");
    if msg.contains(PERMISSIONS_NOT_RESTRICTED_MARKER) {
        EXIT_NOT_RESTRICTED
    } else if msg.contains(PRETOOL_FAIL_SENTINEL) {
        EXIT_PRETOOL_FAIL
    } else if msg.contains("Lint error") {
        EXIT_LINT
    } else if msg.contains("checksum") || msg.contains("signature") || msg.contains("integrity") {
        EXIT_INTEGRITY
    } else if msg.contains("attachment not found") {
        EXIT_ATTACHMENT
    } else if msg.contains("was removed") || msg.contains("preservation") {
        EXIT_PRESERVATION
    } else if msg.contains("skill") && msg.contains("not installed") {
        EXIT_SKILL
    } else if msg.contains("ambiguous: comment") {
        EXIT_AMBIGUOUS
    } else if msg.contains("not found") {
        EXIT_NOT_FOUND
    } else {
        EXIT_ERROR
    }
}

/// Build an [`IdentityFlags`] plus an optional `--assets-dir` value from
/// per-subcommand arg groups. The adapter boundary is where `~` /
/// `$VAR` get expanded, so the core never sees unexpanded path sigils.
///
/// The returned flags are consumed by
/// [`config::ResolvedConfig::resolve`], which picks the appropriate
/// branch of [`config::identity::resolve_identity`] — a single whole
/// identity comes out, never a mixture of fields from different files.
pub(crate) fn build_identity_flags(
    system: &dyn System,
    identity_args: &IdentityArgs,
    assets_args: Option<&AssetsArgs>,
) -> Result<(IdentityFlags, Option<String>)> {
    let assets_dir = match assets_args.and_then(|a| a.assets_dir.as_deref()) {
        Some(raw) => Some(expand_cli_path(system, raw)?.to_string_lossy().into_owned()),
        None => None,
    };

    let config_path = match identity_args.config.as_deref() {
        Some(raw) => Some(expand_cli_path(system, &raw.to_string_lossy())?),
        None => None,
    };

    let key = match identity_args.key.as_deref() {
        Some(raw) => {
            // `--key` accepts a bare name shorthand (e.g. `mykey` →
            // `~/.ssh/mykey`). Expand only when the raw value contains
            // a path sigil — bare names are resolved later by
            // `resolve_key_path`.
            if raw.starts_with('~') || raw.contains('$') {
                Some(expand_cli_path(system, raw)?.to_string_lossy().into_owned())
            } else {
                Some(String::from(raw))
            }
        }
        None => None,
    };

    let author_type = match identity_args.r#type.as_deref() {
        Some(raw) => Some(config::parse_author_type(raw)?),
        None => None,
    };

    let mut flags = IdentityFlags::default();
    flags.author_type = author_type;
    flags.config_path = config_path;
    flags.identity.clone_from(&identity_args.identity);
    flags.key = key;

    Ok((flags, assets_dir))
}

/// A handful of subcommands run entirely without a [`ResolvedConfig`]
/// (`Version`, `Identity`, `ResolveMode`, `Keygen`, `Skill`, `Obsidian`).
/// `Identity` is a read-only diagnostic — it calls
/// [`config::ResolvedConfig::resolve`] inside its own handler so a
/// branch-3 walk miss surfaces as `{ "found": false }` instead of
/// bailing the whole process. Returning `true` here
/// short-circuits the config load in [`run`].
const fn subcommand_is_config_free(cmd: &Commands) -> bool {
    match cmd {
        Commands::Version
        | Commands::Activity { .. }
        | Commands::Claude { .. }
        | Commands::Doctor { .. }
        | Commands::Identity { .. }
        | Commands::Permissions { .. }
        | Commands::ResolveMode { .. }
        | Commands::Keygen { .. } => true,
        #[cfg(feature = "obsidian")]
        Commands::Obsidian { .. } => true,
        Commands::Ack { .. }
        | Commands::Batch { .. }
        | Commands::Comment { .. }
        | Commands::Comments { .. }
        | Commands::Cp { .. }
        | Commands::Delete { .. }
        | Commands::Edit { .. }
        | Commands::Get { .. }
        | Commands::Lint { .. }
        | Commands::Ls { .. }
        | Commands::Mcp { .. }
        | Commands::Metadata { .. }
        | Commands::Mv { .. }
        | Commands::Plan { .. }
        | Commands::Prompt { .. }
        | Commands::Purge { .. }
        | Commands::Query { .. }
        | Commands::React { .. }
        | Commands::Replace { .. }
        | Commands::Registry { .. }
        | Commands::Rm { .. }
        | Commands::GetImage { .. }
        | Commands::Sandbox { .. }
        | Commands::Search { .. }
        | Commands::Sign { .. }
        | Commands::Verify { .. }
        | Commands::Write { .. } => false,
    }
}

/// Fetch the [`IdentityArgs`] flatten for subcommands that declare one.
///
/// Subcommands that do not resolve identity (lint, query, search, ls,
/// get, metadata, registry, comments, version, keygen, resolve-mode,
/// skill, obsidian) return `None`; callers use the
/// [`IdentityArgs::default`] to build an empty [`IdentityFlags`].
const fn subcommand_identity(cmd: &Commands) -> Option<&IdentityArgs> {
    match cmd {
        Commands::Ack { identity_args, .. }
        | Commands::Activity { identity_args, .. }
        | Commands::Batch { identity_args, .. }
        | Commands::Comment { identity_args, .. }
        | Commands::Cp { identity_args, .. }
        | Commands::Delete { identity_args, .. }
        | Commands::Edit { identity_args, .. }
        | Commands::Identity { identity_args, .. }
        | Commands::Mcp { identity_args, .. }
        | Commands::Mv { identity_args, .. }
        | Commands::Plan { identity_args, .. }
        | Commands::Prompt { identity_args, .. }
        | Commands::Purge { identity_args, .. }
        | Commands::Query { identity_args, .. }
        | Commands::React { identity_args, .. }
        | Commands::Replace { identity_args, .. }
        | Commands::Rm { identity_args, .. }
        | Commands::Sandbox { identity_args, .. }
        | Commands::Sign { identity_args, .. }
        | Commands::Verify { identity_args, .. }
        | Commands::Write { identity_args, .. } => Some(identity_args),
        Commands::Claude { .. }
        | Commands::Comments { .. }
        | Commands::Doctor { .. }
        | Commands::Get { .. }
        | Commands::Keygen { .. }
        | Commands::Lint { .. }
        | Commands::Ls { .. }
        | Commands::Metadata { .. }
        | Commands::Permissions { .. }
        | Commands::Registry { .. }
        | Commands::ResolveMode { .. }
        | Commands::GetImage { .. }
        | Commands::Search { .. }
        | Commands::Version => None,
        #[cfg(feature = "obsidian")]
        Commands::Obsidian { .. } => None,
    }
}

/// Fetch the [`AssetsArgs`] flatten for subcommands that write
/// attachments.
const fn subcommand_assets(cmd: &Commands) -> Option<&AssetsArgs> {
    match cmd {
        Commands::Batch { assets_args, .. }
        | Commands::Comment { assets_args, .. }
        | Commands::Edit { assets_args, .. } => Some(assets_args),
        Commands::Ack { .. }
        | Commands::Activity { .. }
        | Commands::Claude { .. }
        | Commands::Comments { .. }
        | Commands::Cp { .. }
        | Commands::Delete { .. }
        | Commands::Doctor { .. }
        | Commands::Get { .. }
        | Commands::Identity { .. }
        | Commands::Keygen { .. }
        | Commands::Lint { .. }
        | Commands::Ls { .. }
        | Commands::Mcp { .. }
        | Commands::Metadata { .. }
        | Commands::Mv { .. }
        | Commands::Permissions { .. }
        | Commands::Plan { .. }
        | Commands::Prompt { .. }
        | Commands::Purge { .. }
        | Commands::Query { .. }
        | Commands::React { .. }
        | Commands::Replace { .. }
        | Commands::Registry { .. }
        | Commands::ResolveMode { .. }
        | Commands::Rm { .. }
        | Commands::GetImage { .. }
        | Commands::Sandbox { .. }
        | Commands::Search { .. }
        | Commands::Sign { .. }
        | Commands::Verify { .. }
        | Commands::Version
        | Commands::Write { .. } => None,
        #[cfg(feature = "obsidian")]
        Commands::Obsidian { .. } => None,
    }
}

/// Fetch the [`UnrestrictedArgs`] flatten for subcommands that touch
/// arbitrary filesystem paths.
const fn subcommand_unrestricted(cmd: &Commands) -> Option<&UnrestrictedArgs> {
    match cmd {
        Commands::Cp {
            unrestricted_args, ..
        }
        | Commands::Get {
            unrestricted_args, ..
        }
        | Commands::Ls {
            unrestricted_args, ..
        }
        | Commands::Metadata {
            unrestricted_args, ..
        }
        | Commands::Rm {
            unrestricted_args, ..
        }
        | Commands::GetImage {
            unrestricted_args, ..
        }
        | Commands::Replace {
            unrestricted_args, ..
        }
        | Commands::Write {
            unrestricted_args, ..
        } => Some(unrestricted_args),
        Commands::Ack { .. }
        | Commands::Activity { .. }
        | Commands::Batch { .. }
        | Commands::Claude { .. }
        | Commands::Comment { .. }
        | Commands::Comments { .. }
        | Commands::Delete { .. }
        | Commands::Doctor { .. }
        | Commands::Edit { .. }
        | Commands::Identity { .. }
        | Commands::Keygen { .. }
        | Commands::Lint { .. }
        | Commands::Mcp { .. }
        | Commands::Mv { .. }
        | Commands::Permissions { .. }
        | Commands::Plan { .. }
        | Commands::Prompt { .. }
        | Commands::Purge { .. }
        | Commands::Query { .. }
        | Commands::React { .. }
        | Commands::Registry { .. }
        | Commands::ResolveMode { .. }
        | Commands::Sandbox { .. }
        | Commands::Search { .. }
        | Commands::Sign { .. }
        | Commands::Verify { .. }
        | Commands::Version => None,
        #[cfg(feature = "obsidian")]
        Commands::Obsidian { .. } => None,
    }
}

fn run(cli: &Cli, system: &dyn System, cwd: &Path, sinks: &mut IoSinks<'_>) -> ExitCode {
    let json_mode = subcommand_output(&cli.command).is_some_and(|o| o.json);

    match dispatch(cli, system, cwd, sinks) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let err_msg = format!("{err:#}");
            let is_silent_sentinel = err_msg.contains(PERMISSIONS_NOT_RESTRICTED_MARKER);
            let exit_code = classify_error(&err);
            let verify_failure = err.downcast_ref::<operations::verify::VerifyFailure>();
            let subset_failure = err.downcast_ref::<operations::verify::SubsetGateFailure>();
            if is_silent_sentinel {
                // Sentinel for `permissions check`.
                // Output already emitted on the success path; we only
                // need the gitignore-style exit code, no "error: ..."
                // render.
            } else if let Some(reason) = err_msg.strip_prefix(PRETOOL_FAIL_SENTINEL) {
                // Pretool fail-closed: Claude Code reads stderr and
                // feeds it back to the model. No "error: " prefix —
                // just the bare reason.
                let _ = writeln!(sinks.stderr, "{reason}");
            } else if json_mode {
                let payload = subset_failure
                    .map(operations::verify::SubsetGateFailure::to_json)
                    .or_else(|| verify_failure.map(operations::verify::VerifyFailure::to_json))
                    .unwrap_or_else(|| json!({ "error": err_msg }));
                let error_json = inject_elapsed_ms(&payload);
                let _ = writeln!(
                    sinks.stderr,
                    "{}",
                    serde_json::to_string_pretty(&error_json).unwrap_or_default()
                );
            } else if let Some(sg) = subset_failure {
                let _ = writeln!(sinks.stderr, "error: {}\n\n{}", sg.headline(), sg.hint());
            } else if let Some(vf) = verify_failure {
                let _ = writeln!(sinks.stderr, "error: {}", vf.human_text());
            } else {
                let _ = writeln!(sinks.stderr, "error: {err_msg}");
            }
            ExitCode::from(exit_code)
        }
    }
}

fn dispatch(cli: &Cli, system: &dyn System, cwd: &Path, sinks: &mut IoSinks<'_>) -> Result<()> {
    let output = subcommand_output(&cli.command);
    let json_mode = output.is_some_and(|o| o.json);

    if try_dispatch_config_free(cli, system, cwd, sinks)?.is_some() {
        return Ok(());
    }

    let default_identity = IdentityArgs::default();
    // Feature-gated: with `unrestricted`, this is a derived `Default` on a
    // regular struct; without it, a unit struct. Both spell as `UnrestrictedArgs::default()`
    // but clippy flags the unit-struct case as `default_constructed_unit_structs`.
    #[cfg(feature = "unrestricted")]
    let default_unrestricted = UnrestrictedArgs::default();
    #[cfg(not(feature = "unrestricted"))]
    let default_unrestricted = UnrestrictedArgs;
    let identity_args = subcommand_identity(&cli.command).unwrap_or(&default_identity);
    let assets_args = subcommand_assets(&cli.command);
    let unrestricted_args = subcommand_unrestricted(&cli.command).unwrap_or(&default_unrestricted);

    let (flags, assets_dir) = build_identity_flags(system, identity_args, assets_args)?;

    // The Mcp subcommand forwards its flags directly to `mcp::run` so
    // per-tool identity fields can still be declared on each request.
    // Branch out early.
    if let Commands::Mcp { action, .. } = &cli.command {
        return handlers::cmd_mcp(
            sinks,
            system,
            cwd,
            &flags,
            assets_dir.as_deref(),
            action.as_ref(),
            json_mode,
        );
    }

    let mut final_config = ResolvedConfig::resolve(system, cwd, &flags, assets_dir.as_deref())?;
    final_config.unrestricted = unrestricted_args.unrestricted();

    dispatch_with_config(sinks, cli, system, cwd, &final_config)
}

/// Handle every config-free subcommand. Returns `Ok(Some(()))` when a
/// matching arm ran, `Ok(None)` when the subcommand needs the
/// config-aware dispatch path.
fn try_dispatch_config_free(
    cli: &Cli,
    system: &dyn System,
    cwd: &Path,
    sinks: &mut IoSinks<'_>,
) -> Result<Option<()>> {
    match &cli.command {
        Commands::Version => handle_version(sinks).map(Some),
        Commands::Identity { .. } => handle_identity(&cli.command, sinks, system, cwd).map(Some),
        Commands::ResolveMode { .. } => {
            handle_resolve_mode(&cli.command, sinks, system, cwd).map(Some)
        }
        Commands::Keygen { .. } => handle_keygen(&cli.command, sinks, system).map(Some),
        #[cfg(feature = "obsidian")]
        Commands::Obsidian { .. } => handle_obsidian(&cli.command, sinks, system, cwd).map(Some),
        Commands::Activity { .. } => handle_activity(&cli.command, sinks, system, cwd).map(Some),
        Commands::Doctor { .. } => handlers::cmd_doctor(sinks, system, cwd, &cli.command).map(Some),
        Commands::Permissions { action } => {
            handlers::cmd_permissions(sinks, system, cwd, action).map(Some)
        }
        Commands::Claude { action } => handle_claude(action, sinks, system, cwd).map(Some),
        _ => {
            debug_assert!(
                !subcommand_is_config_free(&cli.command),
                "config-free subcommand fell through short-circuit"
            );
            Ok(None)
        }
    }
}

fn handle_version(sinks: &mut IoSinks<'_>) -> Result<()> {
    writeln!(sinks.stderr, "remargin {}", env!("CARGO_PKG_VERSION")).context("writing to stderr")
}

fn handle_identity(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    let Commands::Identity {
        action,
        identity_args,
        output_args,
    } = command
    else {
        bail!("internal: handle_identity called with wrong subcommand");
    };
    handlers::cmd_identity(
        sinks,
        system,
        cwd,
        action.as_ref(),
        identity_args,
        output_args.json,
    )
}

fn handle_prompt(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Prompt {
        action,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_prompt called with wrong subcommand");
    };
    let cmd_json = output_args.json;
    match action {
        PromptAction::Resolve {
            file,
            output_args: a,
        } => handlers::cmd_prompt_resolve(sinks, system, cwd, file, cmd_json || a.json),
        PromptAction::Set {
            folder,
            name,
            prompt,
            output_args: a,
        } => handlers::cmd_prompt_set(
            sinks,
            system,
            &PromptSetParams {
                config,
                cwd,
                folder,
                json_mode: cmd_json || a.json,
                name,
                prompt_flag: prompt.as_deref(),
            },
        ),
        PromptAction::Delete {
            folder,
            output_args: a,
        } => handlers::cmd_prompt_delete(sinks, system, cwd, config, folder, cmd_json || a.json),
        PromptAction::List {
            folder,
            output_args: a,
        } => handlers::cmd_prompt_list(sinks, system, cwd, folder, cmd_json || a.json),
    }
}

fn handle_resolve_mode(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    let Commands::ResolveMode {
        cwd: cwd_arg,
        output_args,
    } = command
    else {
        bail!("internal: handle_resolve_mode called with wrong subcommand");
    };
    let cwd_expanded = cwd_arg
        .as_deref()
        .map(|c| expand_cli_pathbuf(system, c))
        .transpose()?;
    let start_dir = cwd_expanded.as_deref().unwrap_or(cwd);
    handlers::cmd_resolve_mode(sinks, system, start_dir, output_args.json)
}

fn handle_keygen(command: &Commands, sinks: &mut IoSinks<'_>, system: &dyn System) -> Result<()> {
    let Commands::Keygen {
        output: keygen_output,
        ..
    } = command
    else {
        bail!("internal: handle_keygen called with wrong subcommand");
    };
    let expanded_output = expand_cli_pathbuf(system, keygen_output)?;
    handlers::cmd_keygen(sinks, system, &expanded_output)
}

#[cfg(feature = "obsidian")]
fn handle_obsidian(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    let Commands::Obsidian {
        action,
        output_args,
    } = command
    else {
        bail!("internal: handle_obsidian called with wrong subcommand");
    };
    handlers::cmd_obsidian(sinks, system, cwd, action, output_args.json)
}

fn handle_plugin(
    sinks: &mut IoSinks<'_>,
    action: &PluginAction,
    output_args: &OutputArgs,
) -> Result<()> {
    handlers::cmd_plugin(sinks, action, output_args.json)
}

fn handle_activity(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    let Commands::Activity {
        path,
        since,
        pretty,
        identity_args,
        output_args,
    } = command
    else {
        bail!("internal: handle_activity called with wrong subcommand");
    };
    let p = ActivityParams {
        explicit_path: path.as_deref(),
        identity_args,
        json_mode: output_args.json,
        pretty: *pretty,
        since: since.as_deref(),
    };
    handlers::cmd_activity(sinks, system, cwd, &p)
}

fn handle_claude(
    action: &ClaudeAction,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    match action {
        ClaudeAction::Plugin {
            action: plugin_action,
            output_args,
        } => handle_plugin(sinks, plugin_action, output_args),
        ClaudeAction::Pretool {
            action: pretool_action,
            output_args,
        } => handle_claude_pretool_action(sinks, system, pretool_action.as_ref(), output_args.json),
        ClaudeAction::Restrict {
            path,
            also_deny_bash,
            cli_allowed,
            user_settings,
            output_args,
        } => {
            let p = RestrictParams {
                also_deny_bash,
                cli_allowed: *cli_allowed,
                json_mode: output_args.json,
                path,
                user_settings_explicit: user_settings.as_deref(),
            };
            handlers::cmd_restrict(sinks, system, cwd, &p)
        }
        ClaudeAction::Unrestrict {
            path,
            strict,
            user_settings,
            output_args,
        } => handlers::cmd_unprotect(
            sinks,
            system,
            cwd,
            path,
            *strict,
            user_settings.as_deref(),
            output_args.json,
        ),
    }
}

/// Route `remargin claude pretool [subcommand]`. With no subcommand
/// (or `dispatch`), runs the stdin/stdout hook dispatcher. The
/// install / uninstall / test variants manage the hook entry in a
/// Claude settings file.
fn handle_claude_pretool_action(
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    action: Option<&PretoolAction>,
    json_mode: bool,
) -> Result<()> {
    match action {
        None | Some(PretoolAction::Dispatch) => handle_claude_pretool_dispatch(sinks, system),
        Some(PretoolAction::Install { local }) => {
            handlers::cmd_pretool_install(sinks, system, *local, json_mode)
        }
        Some(PretoolAction::Uninstall { local }) => {
            handlers::cmd_pretool_uninstall(sinks, system, *local, json_mode)
        }
        Some(PretoolAction::Test { local }) => {
            handlers::cmd_pretool_test(sinks, system, *local, json_mode)
        }
    }
}

/// Reads the `PreToolUse` event JSON from stdin, runs the core
/// [`remargin_core::permissions::pretool::pretool`] function, and
/// emits the outcome. Fail-closed: any failure exits via
/// [`anyhow::bail!`] so the surrounding runner returns a non-zero
/// status (mapped to Claude Code's blocking semantics).
fn handle_claude_pretool_dispatch(sinks: &mut IoSinks<'_>, system: &dyn System) -> Result<()> {
    let mut buf = Vec::new();
    stdin_handle()
        .read_to_end(&mut buf)
        .context("reading stdin for claude pretool")?;
    match pretool(system, &buf) {
        PretoolOutcome::SilentAllow => Ok(()),
        PretoolOutcome::Deny(decision) => {
            let json = serde_json::to_string(&decision).context("serializing pretool decision")?;
            writeln!(sinks.stdout, "{json}").context("writing claude pretool decision")
        }
        PretoolOutcome::Fail(reason) => Err(anyhow::anyhow!("{PRETOOL_FAIL_SENTINEL}{reason}")),
        _ => Err(anyhow::anyhow!(
            "{PRETOOL_FAIL_SENTINEL}unexpected pretool outcome",
        )),
    }
}

fn dispatch_with_config(
    sinks: &mut IoSinks<'_>,
    cli: &Cli,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    match &cli.command {
        Commands::Ack { .. } => handle_ack(&cli.command, sinks, system, cwd, config),
        Commands::Batch { .. } => handle_batch(&cli.command, sinks, system, cwd, config),
        Commands::Comment { .. } => handle_comment(&cli.command, sinks, system, cwd, config),
        Commands::Comments { .. } => handle_comments(&cli.command, sinks, system, cwd),
        Commands::Cp { .. } => handle_cp(&cli.command, sinks, system, cwd, config),
        Commands::Delete { .. } => handle_delete(&cli.command, sinks, system, cwd, config),
        Commands::Edit { .. } => handle_edit(&cli.command, sinks, system, cwd, config),
        Commands::Get { .. } => handle_get(&cli.command, sinks, system, cwd, config),
        Commands::Lint { .. } => handle_lint(&cli.command, sinks, system, cwd),
        Commands::Ls { .. } => handle_ls(&cli.command, sinks, system, cwd, config),
        Commands::Metadata { .. } => handle_metadata(&cli.command, sinks, system, cwd, config),
        Commands::Mv { .. } => handle_mv(&cli.command, sinks, system, cwd, config),
        Commands::Plan { .. } => handle_plan(&cli.command, sinks, system, cwd, config),
        Commands::Prompt { .. } => handle_prompt(&cli.command, sinks, system, cwd, config),
        Commands::Purge { .. } => handle_purge(&cli.command, sinks, system, cwd, config),
        Commands::Query { .. } => handle_query(&cli.command, sinks, system, cwd, config),
        Commands::React { .. } => handle_react(&cli.command, sinks, system, cwd, config),
        Commands::Replace { .. } => handle_replace(&cli.command, sinks, system, cwd, config),
        Commands::Registry { .. } => handle_registry(&cli.command, sinks, system, cwd),
        Commands::Rm { .. } => handle_rm(&cli.command, sinks, system, cwd, config),
        Commands::GetImage { .. } => handle_get_image(&cli.command, sinks, system, cwd, config),
        Commands::Sandbox { .. } => handle_sandbox(&cli.command, sinks, system, cwd, config),
        Commands::Search { .. } => handle_search(&cli.command, sinks, system, cwd),
        Commands::Sign { .. } => handle_sign(&cli.command, sinks, system, cwd, config),
        Commands::Verify { .. } => handle_verify(&cli.command, sinks, system, cwd, config),
        Commands::Write { .. } => handle_write(&cli.command, sinks, system, cwd, config),
        Commands::Version
        | Commands::Activity { .. }
        | Commands::Claude { .. }
        | Commands::Doctor { .. }
        | Commands::Identity { .. }
        | Commands::Mcp { .. }
        | Commands::Keygen { .. }
        | Commands::Permissions { .. }
        | Commands::ResolveMode { .. } => Ok(()),
        #[cfg(feature = "obsidian")]
        Commands::Obsidian { .. } => Ok(()),
    }
}

fn handle_ack(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Ack {
        file,
        ids,
        path,
        remove,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_ack called with wrong subcommand");
    };
    let ap = AckParams {
        file: file.as_deref(),
        ids,
        json_mode: output_args.json,
        remove: *remove,
        search_path: path,
    };
    handlers::cmd_ack(sinks, system, cwd, config, &ap)
}

fn handle_batch(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Batch {
        file,
        ops,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_batch called with wrong subcommand");
    };
    handlers::cmd_batch(sinks, system, cwd, config, file, ops, output_args.json)
}

fn handle_comment(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Comment {
        file,
        content,
        after_comment,
        after_heading,
        after_line,
        attach,
        auto_ack,
        no_auto_ack,
        comment_file,
        remargin_kind,
        reply_to,
        sandbox,
        to,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_comment called with wrong subcommand");
    };
    let resolved_content =
        resolve_comment_content(system, cwd, content.as_ref(), comment_file.as_ref())?;
    let cp = CommentParams {
        after_comment: after_comment.as_deref(),
        after_heading: after_heading.as_deref(),
        after_line: *after_line,
        attachments: attach,
        auto_ack: tri_state_flag(*auto_ack, *no_auto_ack),
        content: &resolved_content,
        file,
        json_mode: output_args.json,
        remargin_kind,
        reply_to: reply_to.as_deref(),
        sandbox: *sandbox,
        to,
    };
    handlers::cmd_comment(sinks, system, cwd, config, &cp)
}

/// Map paired clap booleans to `Option<bool>`: `--flag` → Some(true),
/// `--no-flag` → Some(false), neither → None. The `conflicts_with` clap
/// attributes guarantee only one can be true at a time.
pub(crate) const fn tri_state_flag(yes: bool, no: bool) -> Option<bool> {
    if yes {
        Some(true)
    } else if no {
        Some(false)
    } else {
        None
    }
}

fn handle_comments(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    let Commands::Comments {
        file,
        pretty,
        remargin_kind,
        output_args,
    } = command
    else {
        bail!("internal: handle_comments called with wrong subcommand");
    };
    handlers::cmd_comments(
        sinks,
        system,
        cwd,
        file,
        remargin_kind,
        output_args.json,
        *pretty,
    )
}

fn handle_delete(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Delete {
        file,
        ids,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_delete called with wrong subcommand");
    };
    handlers::cmd_delete(sinks, system, cwd, config, file, ids, output_args.json)
}

fn handle_edit(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Edit {
        file,
        id,
        content,
        remargin_kind,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_edit called with wrong subcommand");
    };
    // When no --kind flags are provided we preserve the stored list; any
    // occurrence (even `--kind x` once) replaces the full list — consistent
    // with how `--to` works.
    let kind_replacement = (!remargin_kind.is_empty()).then_some(remargin_kind.as_slice());
    let p = EditParams {
        content,
        file,
        id,
        json_mode: output_args.json,
        remargin_kind: kind_replacement,
    };
    handlers::cmd_edit(sinks, system, cwd, config, &p)
}

fn handle_get(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Get {
        path,
        binary,
        start,
        end,
        line_numbers,
        out,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_get called with wrong subcommand");
    };
    let gp = GetParams {
        binary: *binary,
        end: *end,
        json_mode: output_args.json,
        line_numbers: *line_numbers,
        out: out.as_deref(),
        path,
        start: *start,
    };
    handlers::cmd_get(sinks, system, cwd, config, &gp)
}

fn handle_lint(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    let Commands::Lint { file, output_args } = command else {
        bail!("internal: handle_lint called with wrong subcommand");
    };
    handlers::cmd_lint(sinks, system, cwd, file, output_args.json)
}

fn handle_ls(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Ls {
        path, output_args, ..
    } = command
    else {
        bail!("internal: handle_ls called with wrong subcommand");
    };
    handlers::cmd_ls(sinks, system, cwd, config, path, output_args.json)
}

fn handle_metadata(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Metadata {
        path, output_args, ..
    } = command
    else {
        bail!("internal: handle_metadata called with wrong subcommand");
    };
    handlers::cmd_metadata(sinks, system, cwd, config, path, output_args.json)
}

fn handle_cp(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Cp {
        src,
        dst,
        force,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_cp called with wrong subcommand");
    };
    let p = CpParams {
        dst: dst.as_str(),
        force: *force,
        json_mode: output_args.json,
        src: src.as_str(),
    };
    handlers::cmd_cp(sinks, system, cwd, config, &p)
}

fn handle_mv(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Mv {
        src,
        dst,
        force,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_mv called with wrong subcommand");
    };
    let p = MvParams {
        dst: dst.as_str(),
        force: *force,
        json_mode: output_args.json,
        src: src.as_str(),
    };
    handlers::cmd_mv(sinks, system, cwd, config, &p)
}

fn handle_plan(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Plan { action, .. } = command else {
        bail!("internal: handle_plan called with wrong subcommand");
    };
    handlers::cmd_plan(
        sinks,
        system,
        cwd,
        config,
        action,
        plan_action_output(action).json,
    )
}

fn handle_purge(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Purge {
        file,
        output_args,
        recursive,
        ..
    } = command
    else {
        bail!("internal: handle_purge called with wrong subcommand");
    };
    handlers::cmd_purge(
        sinks,
        system,
        cwd,
        config,
        file,
        *recursive,
        output_args.json,
    )
}

fn handle_query(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let q = handlers::build_query_params(command)?;
    handlers::cmd_query(sinks, system, cwd, config, &q)
}

fn handle_react(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::React {
        file,
        id,
        emoji,
        remove,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_react called with wrong subcommand");
    };
    let r = ReactParams {
        emoji: emoji.as_str(),
        file: file.as_str(),
        id: id.as_str(),
        json_mode: output_args.json,
        remove: *remove,
    };
    handlers::cmd_react(sinks, system, cwd, config, &r)
}

fn handle_replace(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Replace {
        pattern,
        replacement,
        path,
        regex,
        ignore_case,
        dry_run,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_replace called with wrong subcommand");
    };
    let options = replace::ReplaceOptions::new(
        String::from(pattern.as_str()),
        String::from(replacement.as_str()),
    )
    .regex(*regex)
    .ignore_case(*ignore_case)
    .dry_run(*dry_run);
    let r = ReplaceParams {
        json_mode: output_args.json,
        options,
        path: path.as_str(),
    };
    handlers::cmd_replace(sinks, system, cwd, config, &r)
}

fn handle_registry(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    let Commands::Registry {
        action,
        output_args,
    } = command
    else {
        bail!("internal: handle_registry called with wrong subcommand");
    };
    handlers::cmd_registry(sinks, system, cwd, action, output_args.json)
}

fn handle_rm(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Rm {
        file, output_args, ..
    } = command
    else {
        bail!("internal: handle_rm called with wrong subcommand");
    };
    handlers::cmd_rm(sinks, system, cwd, config, file, output_args.json)
}

fn handle_sandbox(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Sandbox {
        action,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_sandbox called with wrong subcommand");
    };
    handlers::cmd_sandbox(sinks, system, cwd, config, action, output_args.json)
}

fn handle_get_image(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::GetImage {
        path,
        crop,
        format,
        max_bytes,
        max_dimension,
        out,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_get_image called with wrong subcommand");
    };
    let sp = GetImageParams {
        crop: crop.as_deref(),
        format: format.as_deref(),
        json_mode: output_args.json,
        max_bytes: *max_bytes,
        max_dimension: *max_dimension,
        out: out.as_deref(),
        path,
    };
    handlers::cmd_get_image(sinks, system, cwd, config, &sp)
}

fn handle_search(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
) -> Result<()> {
    let Commands::Search {
        pattern,
        path,
        regex,
        scope,
        context,
        ignore_case,
        output_args,
    } = command
    else {
        bail!("internal: handle_search called with wrong subcommand");
    };
    let s = SearchParams {
        context: *context,
        ignore_case: *ignore_case,
        json_mode: output_args.json,
        path: path.as_str(),
        pattern: pattern.as_str(),
        regex: *regex,
        scope: scope.as_str(),
    };
    handlers::cmd_search(sinks, system, cwd, &s)
}

fn handle_sign(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Sign {
        file,
        ids,
        all_mine,
        repair_checksum,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_sign called with wrong subcommand");
    };
    let sp = SignParams {
        all_mine: *all_mine,
        file,
        ids,
        json_mode: output_args.json,
        repair_checksum: *repair_checksum,
    };
    handlers::cmd_sign(sinks, system, cwd, config, &sp)
}

fn handle_verify(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Verify {
        file, output_args, ..
    } = command
    else {
        bail!("internal: handle_verify called with wrong subcommand");
    };
    handlers::cmd_verify(sinks, system, cwd, file, config, output_args.json)
}

fn handle_write(
    command: &Commands,
    sinks: &mut IoSinks<'_>,
    system: &dyn System,
    cwd: &Path,
    config: &ResolvedConfig,
) -> Result<()> {
    let Commands::Write {
        path,
        content,
        binary,
        create,
        lines,
        raw,
        output_args,
        ..
    } = command
    else {
        bail!("internal: handle_write called with wrong subcommand");
    };
    let line_range = lines.as_deref().map(parse_line_range).transpose()?;
    handlers::cmd_write(
        sinks,
        system,
        cwd,
        config,
        &WriteParams {
            content: content.as_deref(),
            json_mode: output_args.json,
            opts: document::WriteOptions::new()
                .binary(*binary)
                .create(*create)
                .lines(line_range)
                .raw(*raw),
            path,
        },
    )
}

#[cfg(test)]
mod tests;
