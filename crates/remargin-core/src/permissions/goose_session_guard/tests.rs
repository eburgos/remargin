//! Unit tests for `permissions::goose_session_guard` — the diagnostic-only
//! `SessionStart` backstop.

use std::path::Path;

use os_shim::System as _;
use os_shim::mock::MemorySystem;
use serde_json::json;

use super::{GuardOutcome, goose_session_guard};

const EXE: &str = "/opt/bin/remargin";

/// A wired plugin manifest naming `binary` as the `PreToolUse` command.
fn hooks_json(binary: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "hooks": { "PreToolUse": [{ "hooks": [
            { "type": "command", "command": format!("{binary} goose pretool") },
        ] }] },
    }))
    .unwrap()
}

/// A mock with `HOME` set and the remargin binary on disk. The plugin is
/// left out so each case wires the scope it is about.
fn mock() -> MemorySystem {
    let system = MemorySystem::new()
        .with_dir(Path::new("/r"))
        .unwrap()
        .with_file(Path::new(EXE), b"binary")
        .unwrap();
    system.set_env_var("HOME", "/home/u");
    system
}

/// A mock whose user-scope plugin is wired — the healthy baseline.
fn mock_with_user_plugin() -> MemorySystem {
    mock()
        .with_file(
            Path::new("/home/u/.agents/plugins/remargin-guard/hooks/hooks.json"),
            hooks_json(EXE).as_bytes(),
        )
        .unwrap()
}

fn envelope(working_dir: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "event": "SessionStart",
        "session_id": "test",
        "working_dir": working_dir,
    }))
    .unwrap()
}

/// Destructure a `Fail` outcome without a `panic!` (denied by clippy). The
/// `matches!` assert carries the diagnostic; the else arm is unreachable.
fn expect_fail(outcome: GuardOutcome) -> String {
    assert!(
        matches!(outcome, GuardOutcome::Fail(_)),
        "expected GuardOutcome::Fail, got {outcome:?}",
    );
    let GuardOutcome::Fail(diagnostic) = outcome else {
        return String::new();
    };
    diagnostic
}

/// A wired plugin plus a parseable realm config is silence.
#[test]
fn wired_plugin_and_parseable_config_is_ok() {
    let system = mock_with_user_plugin()
        .with_file(
            Path::new("/r/.remargin.yaml"),
            b"identity: alice\ntype: human\n",
        )
        .unwrap();

    assert_eq!(
        goose_session_guard(&system, &envelope("/r"), Path::new("/r")),
        GuardOutcome::Ok,
    );
}

/// No realm config on the walk is not a failure — an absent config parses
/// vacuously, exactly as the Claude guard treats it.
#[test]
fn no_realm_config_is_ok_when_the_plugin_is_wired() {
    assert_eq!(
        goose_session_guard(&mock_with_user_plugin(), &envelope("/r"), Path::new("/r")),
        GuardOutcome::Ok,
    );
}

/// A project-scope install is a supported wiring, so it clears the check on
/// its own.
#[test]
fn project_scope_plugin_satisfies_the_wiring_check() {
    let system = mock()
        .with_file(
            Path::new("/r/.agents/plugins/remargin-guard/hooks/hooks.json"),
            hooks_json(EXE).as_bytes(),
        )
        .unwrap();

    assert_eq!(
        goose_session_guard(&system, &envelope("/r"), Path::new("/r")),
        GuardOutcome::Ok,
    );
}

/// No plugin in either scope: the diagnostic names both scopes it looked in
/// and the event that is going unguarded.
#[test]
fn absent_plugin_fails_and_names_both_scopes() {
    let diagnostic = expect_fail(goose_session_guard(
        &mock(),
        &envelope("/r"),
        Path::new("/r"),
    ));
    assert!(
        diagnostic.contains("/home/u/.agents/plugins/remargin-guard"),
        "diagnostic should name the user scope: {diagnostic}",
    );
    assert!(
        diagnostic.contains("/r/.agents/plugins/remargin-guard"),
        "diagnostic should name the project scope: {diagnostic}",
    );
    assert!(
        diagnostic.contains("PreToolUse"),
        "diagnostic should name the unguarded event: {diagnostic}",
    );
}

/// The fail-open trap this guard exists for: the plugin is present and
/// parses, but the binary its command names is gone, so goose spawns
/// nothing and waves every tool call through.
#[test]
fn plugin_pointing_at_a_missing_binary_fails_and_names_it() {
    let system = mock_with_user_plugin();
    system.remove_file(Path::new(EXE)).unwrap();

    let diagnostic = expect_fail(goose_session_guard(
        &system,
        &envelope("/r"),
        Path::new("/r"),
    ));
    assert!(
        diagnostic.contains(EXE),
        "diagnostic should name the missing binary: {diagnostic}",
    );
}

/// A plugin whose manifest declares no `PreToolUse` entry is a guard that
/// does not guard.
#[test]
fn plugin_without_the_pretool_entry_fails() {
    let system = mock()
        .with_file(
            Path::new("/home/u/.agents/plugins/remargin-guard/hooks/hooks.json"),
            b"{\"hooks\": {}}\n",
        )
        .unwrap();

    let diagnostic = expect_fail(goose_session_guard(
        &system,
        &envelope("/r"),
        Path::new("/r"),
    ));
    assert!(
        diagnostic.contains("PreToolUse"),
        "diagnostic should name the absent entry: {diagnostic}",
    );
}

/// An unparseable realm config fails even with the plugin wired, and the
/// diagnostic points at `remargin doctor`.
#[test]
fn unparseable_realm_config_fails() {
    let system = mock_with_user_plugin()
        .with_file(Path::new("/r/.remargin.yaml"), b": : not valid yaml : :")
        .unwrap();

    let diagnostic = expect_fail(goose_session_guard(
        &system,
        &envelope("/r"),
        Path::new("/r"),
    ));
    assert!(
        diagnostic.contains(".remargin.yaml"),
        "diagnostic should name the config: {diagnostic}",
    );
    assert!(
        diagnostic.contains("remargin doctor"),
        "diagnostic should point at doctor: {diagnostic}",
    );
}

/// The envelope's `working_dir` roots the realm walk, not the process cwd:
/// the broken config lives under the session's directory, and the guard is
/// invoked from somewhere else entirely.
#[test]
fn working_dir_from_the_envelope_roots_the_realm_check() {
    let system = mock_with_user_plugin()
        .with_dir(Path::new("/session"))
        .unwrap()
        .with_file(Path::new("/session/.remargin.yaml"), b": : bad : :")
        .unwrap();

    let diagnostic = expect_fail(goose_session_guard(
        &system,
        &envelope("/session"),
        Path::new("/r"),
    ));
    assert!(
        diagnostic.contains("/session"),
        "diagnostic should name the session's realm: {diagnostic}",
    );
}

/// An envelope the guard cannot read falls back to the process cwd and says
/// nothing about its own input — a healthy session must stay silent, or the
/// diagnostic stops being believed when it matters.
#[test]
fn unreadable_envelope_falls_back_to_cwd_and_stays_silent_when_healthy() {
    assert_eq!(
        goose_session_guard(&mock_with_user_plugin(), b"", Path::new("/r")),
        GuardOutcome::Ok,
    );
    assert_eq!(
        goose_session_guard(&mock_with_user_plugin(), b"{ not json", Path::new("/r")),
        GuardOutcome::Ok,
    );
}

/// Every failure is reported in one diagnostic rather than the first one
/// found — a session start is the only shot the guard gets.
#[test]
fn all_failures_land_in_one_diagnostic() {
    let system = mock()
        .with_file(Path::new("/r/.remargin.yaml"), b": : bad : :")
        .unwrap();

    let diagnostic = expect_fail(goose_session_guard(
        &system,
        &envelope("/r"),
        Path::new("/r"),
    ));
    assert!(
        diagnostic.contains("remargin-guard") && diagnostic.contains(".remargin.yaml"),
        "both failures should be named: {diagnostic}",
    );
}
