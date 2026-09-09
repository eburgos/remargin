//! Unit tests for `permissions::goose_pretool` — the ten-scenario QA
//! matrix for the goose adapter, minus the two scenarios that can only be
//! observed from the CLI (the two-channel verdict render and the
//! install/uninstall lifecycle).
//!
//! Every test feeds a synthetic goose `PreToolUse` envelope through
//! `goose_pretool()` against a `MemorySystem` realm. The core function is
//! pure, so the binary never spawns.

use std::path::Path;

use os_shim::mock::MemorySystem;
use serde_json::{Value, json};

use crate::permissions::goose_pretool::{BlockDecision, GooseVerdict, goose_pretool};
use crate::permissions::pretool::ToolPrefix;

fn mock_with(files: &[(&str, &str)]) -> MemorySystem {
    let mut system = MemorySystem::new();
    for (path, body) in files {
        system = system.with_file(Path::new(path), body.as_bytes()).unwrap();
    }
    system
}

/// A realm at `/r` whose `secret/` subtree is remargin-managed.
fn realm() -> MemorySystem {
    mock_with(&[(
        "/r/.remargin.yaml",
        "permissions:\n  trusted_roots:\n    - path: secret\n",
    )])
}

fn event_json(tool_name: &str, working_dir: &str, tool_input: &Value) -> Vec<u8> {
    let envelope = json!({
        "event": "PreToolUse",
        "session_id": "test",
        "tool_name": tool_name,
        "tool_input": tool_input,
        "matcher_context": Value::Null,
        "working_dir": working_dir,
    });
    serde_json::to_vec(&envelope).unwrap()
}

fn expect_block(verdict: GooseVerdict) -> String {
    assert!(
        matches!(verdict, GooseVerdict::Block { .. }),
        "expected Block, got {verdict:?}",
    );
    let GooseVerdict::Block { reason } = verdict else {
        return String::new();
    };
    reason
}

fn assert_allow(verdict: &GooseVerdict) {
    assert_eq!(verdict, &GooseVerdict::Allow, "expected Allow");
}

/// A goose session reaches remargin's ops as `remargin__*` and has no tool
/// named `mcp__remargin__*` at all, so a reason spelling the op Claude
/// Code's way names something uncallable — the retry loop the guidance
/// exists to end. Asserting the absence is the load-bearing half: the goose
/// prefix is a substring of Claude's.
fn assert_goose_namespaced(reason: &str) {
    assert!(
        reason.contains(ToolPrefix::GOOSE.as_str()),
        "reason names no remargin op: {reason}",
    );
    assert!(
        !reason.contains(ToolPrefix::CLAUDE.as_str()),
        "reason carries Claude Code's tool prefix: {reason}",
    );
}

// ---- 1. managed path via the text editor -------------------------------

/// A `write` onto a managed path blocks and the reason names the remargin
/// write op and the path, so the agent has its next call spelled out.
#[test]
fn text_editor_write_on_managed_path_blocks_with_write_guidance() {
    let stdin = event_json(
        "developer__text_editor",
        "/r",
        &json!({ "command": "write", "path": "/r/secret/foo.md", "file_text": "x" }),
    );
    let reason = expect_block(goose_pretool(&realm(), &stdin));
    assert!(reason.contains("remargin__write"), "reason: {reason}");
    assert!(reason.contains("/r/secret/foo.md"), "reason: {reason}");
    assert_goose_namespaced(&reason);
}

/// `str_replace` and `insert` are edit-class verbs, so they redirect to the
/// edit op rather than the whole-file write op.
#[test]
fn text_editor_str_replace_and_insert_block_with_edit_guidance() {
    for command in ["str_replace", "insert"] {
        let stdin = event_json(
            "developer__text_editor",
            "/r",
            &json!({ "command": command, "path": "/r/secret/foo.md" }),
        );
        let reason = expect_block(goose_pretool(&realm(), &stdin));
        assert!(
            reason.contains("remargin__edit"),
            "{command} reason: {reason}",
        );
        assert_goose_namespaced(&reason);
    }
}

/// `view` is the read-class verb and redirects to the read op.
#[test]
fn text_editor_view_on_managed_path_blocks_with_get_guidance() {
    let stdin = event_json(
        "developer__text_editor",
        "/r",
        &json!({ "command": "view", "path": "/r/secret/foo.md" }),
    );
    let reason = expect_block(goose_pretool(&realm(), &stdin));
    assert!(reason.contains("remargin__get"), "reason: {reason}");
    assert_goose_namespaced(&reason);
}

/// A relative path is rooted at `working_dir`, so the realm is found even
/// though the envelope never names it absolutely.
#[test]
fn text_editor_relative_path_is_rooted_at_working_dir() {
    let stdin = event_json(
        "developer__text_editor",
        "/r/secret",
        &json!({ "command": "write", "path": "foo.md" }),
    );
    let reason = expect_block(goose_pretool(&realm(), &stdin));
    assert!(reason.contains("/r/secret/foo.md"), "reason: {reason}");
}

// ---- 2. shell touching a managed path ----------------------------------

/// A shell word that lands inside the managed subtree blocks, with the
/// per-verb redirect the engine already owns.
#[test]
fn shell_word_inside_managed_subtree_blocks() {
    let stdin = event_json(
        "developer__shell",
        "/tmp",
        &json!({ "command": "cat /r/secret/foo.md" }),
    );
    let reason = expect_block(goose_pretool(&realm(), &stdin));
    assert!(reason.contains("remargin__get"), "reason: {reason}");
    assert_goose_namespaced(&reason);
}

/// In-realm `working_dir`: bare words pass, path-evidenced words block.
#[test]
fn shell_from_in_realm_working_dir_follows_per_word_scan() {
    let bare = event_json("developer__shell", "/r/secret", &json!({ "command": "ls" }));
    assert_allow(&goose_pretool(&realm(), &bare));

    let evidenced = event_json(
        "developer__shell",
        "/r/secret",
        &json!({ "command": "cat ./idea.md" }),
    );
    let reason = expect_block(goose_pretool(&realm(), &evidenced));
    assert_goose_namespaced(&reason);
}

// ---- 3. unmanaged paths ------------------------------------------------

/// An unmanaged path is allowed for both gated tools — the guard is silent
/// outside the managed subtree.
#[test]
fn unmanaged_path_allows_on_both_gated_tools() {
    let system = realm();
    let editor = event_json(
        "developer__text_editor",
        "/r",
        &json!({ "command": "write", "path": "/r/public/foo.md" }),
    );
    assert_allow(&goose_pretool(&system, &editor));

    let shell = event_json(
        "developer__shell",
        "/tmp",
        &json!({ "command": "ls /r/public" }),
    );
    assert_allow(&goose_pretool(&system, &shell));
}

// ---- 4. remargin's own MCP tools ---------------------------------------

/// The remargin extension is the sanctioned surface and is never
/// intercepted, even when its argument is a managed path — intercepting it
/// would leave the agent with no way to touch managed content at all.
#[test]
fn remargin_mcp_tools_are_never_intercepted() {
    let system = realm();
    for tool in ["remargin__write", "mcp__remargin__write"] {
        let stdin = event_json(tool, "/r/secret", &json!({ "path": "/r/secret/foo.md" }));
        assert_allow(&goose_pretool(&system, &stdin));
    }
}

// ---- 5. malformed / uncertain payloads (fail closed) -------------------

/// Truncated JSON is not a payload the guard can reason about, and goose
/// treats a silent hook as permission to proceed — so it blocks.
#[test]
fn truncated_payload_blocks() {
    let reason = expect_block(goose_pretool(&realm(), b"{\"tool_name\": \"developer__"));
    assert!(reason.contains("remargin"), "reason: {reason}");
}

/// An empty payload blocks for the same reason.
#[test]
fn empty_payload_blocks() {
    let _reason = expect_block(goose_pretool(&realm(), b""));
}

/// A gated tool missing the field that names its target is an uncertain
/// state, not a safe one.
#[test]
fn gated_tool_missing_required_field_blocks() {
    let system = realm();
    let editor_no_path = event_json(
        "developer__text_editor",
        "/r",
        &json!({ "command": "write" }),
    );
    let reason = expect_block(goose_pretool(&system, &editor_no_path));
    assert!(reason.contains("path"), "reason: {reason}");

    let editor_no_command = event_json(
        "developer__text_editor",
        "/r",
        &json!({ "path": "/r/x.md" }),
    );
    let _editor = expect_block(goose_pretool(&system, &editor_no_command));

    let shell_no_command = event_json("developer__shell", "/r", &json!({}));
    let _shell = expect_block(goose_pretool(&system, &shell_no_command));
}

/// A text-editor verb the adapter does not recognize could touch the path
/// in a way the engine has no mapping for, so it blocks rather than guess.
#[test]
fn unrecognized_text_editor_command_blocks() {
    let stdin = event_json(
        "developer__text_editor",
        "/r",
        &json!({ "command": "teleport", "path": "/r/public/foo.md" }),
    );
    let reason = expect_block(goose_pretool(&realm(), &stdin));
    assert!(reason.contains("teleport"), "reason: {reason}");
}

/// Without `working_dir` a relative target cannot be rooted, so a gated
/// tool blocks instead of resolving against an assumed directory.
#[test]
fn gated_tool_without_working_dir_blocks() {
    let envelope = json!({
        "event": "PreToolUse",
        "tool_name": "developer__shell",
        "tool_input": { "command": "ls" },
    });
    let stdin = serde_json::to_vec(&envelope).unwrap();
    let reason = expect_block(goose_pretool(&realm(), &stdin));
    assert!(reason.contains("working_dir"), "reason: {reason}");
}

/// A tool outside the gated set still goes through the engine when its
/// input names a shape the engine gates — an unknown extension reaching a
/// managed path is the same reach by another name.
#[test]
fn ungated_tool_naming_a_managed_path_blocks() {
    let system = realm();
    for key in ["path", "file_path"] {
        let stdin = event_json("other__tool", "/r", &json!({ key: "/r/secret/foo.md" }));
        let reason = expect_block(goose_pretool(&system, &stdin));
        assert!(reason.contains("/r/secret/foo.md"), "reason: {reason}");
    }

    let shell_shaped = event_json("other__tool", "/tmp", &json!({ "command": "rm /r/secret" }));
    let _reason = expect_block(goose_pretool(&system, &shell_shaped));
}

/// A tool outside the gated set that names no path or command shape has
/// nothing for the engine to resolve and is allowed.
#[test]
fn ungated_tool_without_a_gated_shape_allows() {
    let stdin = event_json("other__tool", "/r/secret", &json!({ "query": "hello" }));
    assert_allow(&goose_pretool(&realm(), &stdin));
}

// ---- 6. host tool namespacing ------------------------------------------

/// Every deny family reachable from goose renders its op names in goose's
/// namespacing. Walks the families one by one — the per-tool message, the
/// per-verb shell redirect, the ancestor-destructive deny, and the
/// `cli_allowed` deny — because each builds its own string off the shared
/// registry and a missed one would still hand the agent a tool it cannot
/// call.
#[test]
fn every_deny_family_names_goose_namespaced_ops() {
    let system = realm();
    let cases = [
        event_json(
            "developer__text_editor",
            "/r",
            &json!({ "command": "write", "path": "/r/secret/foo.md" }),
        ),
        event_json(
            "developer__shell",
            "/tmp",
            &json!({ "command": "cat /r/secret/foo.md" }),
        ),
        event_json(
            "developer__shell",
            "/tmp",
            &json!({ "command": "rm -rf /r/secret" }),
        ),
    ];
    for stdin in &cases {
        assert_goose_namespaced(&expect_block(goose_pretool(&system, stdin)));
    }

    let cli_denied = mock_with(&[("/r/.remargin.yaml", "permissions:\n  cli_allowed: false\n")]);
    let stdin = event_json(
        "developer__shell",
        "/r",
        &json!({ "command": "remargin write /r/x.md" }),
    );
    let reason = expect_block(goose_pretool(&cli_denied, &stdin));
    assert!(reason.contains("cli_allowed: false"), "reason: {reason}");
    assert_goose_namespaced(&reason);
}

// ---- 7. verdict payload shape ------------------------------------------

/// The stdout channel carries goose's documented block object verbatim.
#[test]
fn block_decision_serializes_to_goose_shape() {
    let payload = serde_json::to_value(BlockDecision::new("nope")).unwrap();
    assert_eq!(payload, json!({ "decision": "block", "reason": "nope" }));
}
