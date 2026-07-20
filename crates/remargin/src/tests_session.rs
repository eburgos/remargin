//! Tests for `remargin session launch --dry-run` (gated on the `session`
//! feature). Exercises `handlers::cmd_session` directly over mock trees.

use std::path::Path;

use os_shim::mock::MockSystem;
use serde_json::Value;

use crate::handlers::cmd_session;
use crate::io::IoSinks;
use crate::{OutputArgs, SessionAction};

fn launch(dry_run: bool, print: bool, identity: Vec<String>, json: bool) -> SessionAction {
    SessionAction::Launch {
        backend: String::from("claude"),
        dry_run,
        identity,
        multiplexer: String::from("tmux"),
        output_args: OutputArgs {
            compact: false,
            json,
            verbose: false,
        },
        print,
    }
}

/// Run `cmd_session` over a mock tree, returning its result and whatever it
/// wrote to stdout. Any error text reaches the user through `dispatch::run`,
/// not the sinks, so the stderr buffer is not inspected here.
fn run(system: &MockSystem, cwd: &str, action: &SessionAction) -> (anyhow::Result<()>, String) {
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let result = {
        let mut sinks = IoSinks::new(&mut stdout, &mut stderr);
        cmd_session(&mut sinks, system, Path::new(cwd), action)
    };
    (result, String::from_utf8(stdout).unwrap())
}

/// Root and one child realm, each with a launchable `session:` block.
fn launchable_tree() -> MockSystem {
    MockSystem::new()
        .with_file(
            Path::new("/demo/.remargin.yaml"),
            b"identity: root_agent\nsystem_prompt:\n  name: Root\n  prompt: body\nsession:\n  loop: 30s\n  goal: process pending\n",
        )
        .unwrap()
        .with_file(
            Path::new("/demo/finance/.remargin.yaml"),
            b"identity: finance\nsession:\n  loop: 30s\n  goal: reconcile\n",
        )
        .unwrap()
}

#[test]
fn dry_run_lists_all_identities() {
    let system = launchable_tree();
    let (result, stdout) = run(&system, "/demo", &launch(true, false, Vec::new(), false));
    result.unwrap();
    assert!(stdout.contains("IDENTITY"), "header missing: {stdout}");
    assert!(stdout.contains("root_agent"), "stdout: {stdout}");
    assert!(stdout.contains("finance"), "stdout: {stdout}");
    assert!(stdout.contains("demo/finance"), "folder path: {stdout}");
    assert!(
        stdout.contains("2 identities; all launchable."),
        "summary: {stdout}"
    );
}

#[test]
fn dry_run_flags_missing_goal_and_exits_nonzero() {
    let system = MockSystem::new()
        .with_file(
            Path::new("/demo/.remargin.yaml"),
            b"identity: root_agent\nsession:\n  loop: 30s\n  goal: go\n",
        )
        .unwrap()
        .with_file(
            Path::new("/demo/ops/.remargin.yaml"),
            b"identity: ops\nsession:\n  loop: 30s\n",
        )
        .unwrap();
    let (result, stdout) = run(&system, "/demo", &launch(true, false, Vec::new(), false));
    assert!(result.is_err(), "a missing goal must exit non-zero");
    assert!(stdout.contains("MISSING goal"), "flag missing: {stdout}");
    assert!(stdout.contains("1 not launchable"), "summary: {stdout}");
}

#[test]
fn dry_run_json_is_structured_array() {
    let system = launchable_tree();
    let (result, stdout) = run(&system, "/demo", &launch(true, false, Vec::new(), true));
    result.unwrap();
    let parsed: Value = serde_json::from_str(&stdout).unwrap();
    let array = parsed.as_array().unwrap();
    assert_eq!(array.len(), 2);
    for entry in array {
        assert_eq!(entry["launchable"], Value::Bool(true));
        assert!(entry.get("identity").is_some(), "identity key: {entry}");
        assert!(entry.get("loop").is_some(), "loop key: {entry}");
        assert!(entry.get("goal").is_some(), "goal key: {entry}");
    }
}

#[test]
fn dry_run_identity_filter_restricts_rows() {
    let system = launchable_tree();
    let (result, stdout) = run(
        &system,
        "/demo",
        &launch(true, false, vec![String::from("finance")], false),
    );
    result.unwrap();
    assert!(stdout.contains("finance"), "stdout: {stdout}");
    assert!(
        !stdout.contains("root_agent"),
        "filter should drop root_agent: {stdout}"
    );
    assert!(
        stdout.contains("1 identities; all launchable."),
        "summary: {stdout}"
    );
}

#[test]
fn bare_launch_reports_not_available() {
    let system = launchable_tree();
    let (result, stdout) = run(&system, "/demo", &launch(false, false, Vec::new(), false));
    let err = result.unwrap_err();
    assert!(
        format!("{err:#}").contains("not available yet"),
        "message: {err:#}"
    );
    assert!(
        stdout.is_empty(),
        "bare launch must not print a table: {stdout}"
    );
}

#[test]
fn print_flag_defers_to_task_85() {
    let system = launchable_tree();
    let (result, _stdout) = run(&system, "/demo", &launch(false, true, Vec::new(), false));
    let err = result.unwrap_err();
    assert!(format!("{err:#}").contains("task 85"), "message: {err:#}");
}
