//! Tests for the multiplexer engine (task 86). These exercise the *pure*
//! construction — session names, the exact tmux argv vectors, and the full
//! trust-dismiss + seed send-keys sequence — plus the parse/attach surface
//! and the no-supervision invariant. The real-process execution layer is
//! deliberately never spawned here (see the module docs); only the two
//! pre-spawn guard paths of [`launch_into_multiplexer`] are asserted.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::{
    Multiplexer, Tab, build_herdr_plan, build_tmux_plan, default_multiplexer,
    herdr_unavailable_error, launch_into_multiplexer, pane_shows_ready_prompt,
    pane_shows_trust_dialog, parse_pane_id, parse_workspace_id, session_name, substitute,
};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap()
}

fn tab(identity: &str, cwd: &str, launch: &[&str], seeds: &[&str]) -> Tab {
    Tab::new(
        identity.to_owned(),
        PathBuf::from(cwd),
        launch.iter().map(|s| (*s).to_owned()).collect(),
        seeds.iter().map(|s| (*s).to_owned()).collect(),
    )
}

fn strs(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn session_name_is_basename_plus_short_hex() {
    let name = session_name(Path::new("/home/x/demo"), at(1_700_000_000));

    let suffix = name.strip_prefix("demo-").unwrap();
    assert_eq!(suffix.len(), 8, "8 hex chars: {name}");
    assert!(
        suffix.bytes().all(|b| b.is_ascii_hexdigit()),
        "hex suffix: {name}"
    );
}

#[test]
fn session_name_is_deterministic_for_a_fixed_now() {
    let cwd = Path::new("/home/x/demo");
    assert_eq!(session_name(cwd, at(42)), session_name(cwd, at(42)));
}

#[test]
fn session_name_differs_across_two_nows() {
    let cwd = Path::new("/home/x/demo");
    assert_ne!(session_name(cwd, at(42)), session_name(cwd, at(43)));
}

#[test]
fn session_name_sanitizes_unsafe_basename_chars() {
    let name = session_name(Path::new("/vault/TOP OF MIND.base"), at(7));
    assert!(
        name.starts_with("TOP_OF_MIND_base-"),
        "dots and spaces become underscores: {name}"
    );
}

#[test]
fn session_name_falls_back_when_cwd_has_no_basename() {
    let name = session_name(Path::new("/"), at(7));
    assert!(name.starts_with("remargin-"), "root fallback: {name}");
}

#[test]
fn multiplexer_parses_known_values() {
    assert_eq!(Multiplexer::parse("herdr").unwrap(), Multiplexer::Herdr);
    assert_eq!(Multiplexer::parse("tmux").unwrap(), Multiplexer::Tmux);
}

#[test]
fn multiplexer_parse_rejects_unknown_naming_allowed() {
    let err = Multiplexer::parse("screen").unwrap_err().to_string();
    assert!(err.contains("screen"), "names the offender: {err}");
    assert!(err.contains("herdr"), "lists herdr: {err}");
    assert!(err.contains("tmux"), "lists tmux: {err}");
}

#[test]
fn attach_hint_is_multiplexer_specific() {
    assert_eq!(
        Multiplexer::Herdr.attach_hint("demo-abcd"),
        "herdr session attach demo-abcd"
    );
    assert_eq!(
        Multiplexer::Tmux.attach_hint("demo-abcd"),
        "tmux attach -t demo-abcd"
    );
}

#[test]
fn default_multiplexer_prefers_herdr_when_available() {
    assert_eq!(default_multiplexer(true), Multiplexer::Herdr);
    assert_eq!(default_multiplexer(false), Multiplexer::Tmux);
}

#[test]
fn tmux_plan_first_tab_is_new_session_rest_are_new_windows() {
    let tabs = [
        tab("root_agent", "/demo", &["claude", "--foo"], &["/loop 30s"]),
        tab(
            "finance",
            "/demo/finance",
            &["claude", "-n", "finance"],
            &["/loop 1h"],
        ),
    ];

    let plan = build_tmux_plan("demo-abcd", &tabs);

    assert_eq!(
        plan.launch[0],
        strs(&[
            "tmux",
            "new-session",
            "-d",
            "-s",
            "demo-abcd",
            "-n",
            "root_agent",
            "-c",
            "/demo",
            "--",
            "claude",
            "--foo",
        ])
    );
    assert_eq!(
        plan.launch[1],
        strs(&[
            "tmux",
            "new-window",
            "-t",
            "demo-abcd",
            "-n",
            "finance",
            "-c",
            "/demo/finance",
            "--",
            "claude",
            "-n",
            "finance",
        ])
    );
    assert_eq!(plan.launch.len(), 2);
}

#[test]
fn tmux_tab_seed_targets_window_by_name() {
    let tabs = [tab(
        "root_agent",
        "/demo",
        &["claude"],
        &["/loop 30s", "/goal go"],
    )];

    let plan = build_tmux_plan("demo-abcd", &tabs);
    let seed = &plan.tabs[0];

    assert_eq!(seed.identity, "root_agent");
    assert_eq!(
        seed.capture,
        strs(&["tmux", "capture-pane", "-t", "demo-abcd:root_agent", "-p"])
    );
    assert_eq!(
        seed.dismiss_trust,
        strs(&["tmux", "send-keys", "-t", "demo-abcd:root_agent", "Enter"])
    );
}

#[test]
fn tmux_seed_lines_type_each_line_then_submit() {
    let tabs = [tab(
        "root_agent",
        "/demo",
        &["claude"],
        &["/loop 30s", "/goal go"],
    )];

    let plan = build_tmux_plan("demo-abcd", &tabs);

    // The full trust-dismiss + seed send-keys sequence, flattened and asserted
    // command-for-command in order.
    let seed = &plan.tabs[0];
    let mut full: Vec<Vec<String>> = vec![seed.dismiss_trust.clone()];
    full.extend(seed.seed_lines.iter().cloned());
    assert_eq!(
        full,
        vec![
            strs(&["tmux", "send-keys", "-t", "demo-abcd:root_agent", "Enter"]),
            strs(&[
                "tmux",
                "send-keys",
                "-t",
                "demo-abcd:root_agent",
                "-l",
                "/loop 30s",
            ]),
            strs(&["tmux", "send-keys", "-t", "demo-abcd:root_agent", "Enter"]),
            strs(&[
                "tmux",
                "send-keys",
                "-t",
                "demo-abcd:root_agent",
                "-l",
                "/goal go",
            ]),
            strs(&["tmux", "send-keys", "-t", "demo-abcd:root_agent", "Enter"]),
        ]
    );
}

#[test]
fn pane_readiness_predicates_match_expected_markers() {
    assert!(pane_shows_trust_dialog(
        "Is this a project you trust? 1. Yes"
    ));
    assert!(!pane_shows_trust_dialog("just a normal prompt"));
    assert!(pane_shows_ready_prompt("> type here  ? for shortcuts"));
    assert!(!pane_shows_ready_prompt(""));
}

#[test]
fn launch_rejects_empty_tabs_without_spawning() {
    let err = launch_into_multiplexer(Multiplexer::Tmux, "demo-abcd", &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("no sessions to launch"), "message: {err}");
}

#[test]
fn herdr_plan_creates_workspace_then_starts_and_seeds_each_tab() {
    let tabs = [
        tab("root_agent", "/demo", &["claude", "--foo"], &["/loop 30s"]),
        tab(
            "finance",
            "/demo/finance",
            &["claude", "-n", "finance"],
            &["/loop 1h", "/goal reconcile"],
        ),
    ];

    let plan = build_herdr_plan("demo-abcd", &tabs);

    // Workspace is created once, rooted at the first tab's cwd.
    assert_eq!(
        plan.create_workspace,
        strs(&[
            "herdr",
            "workspace",
            "create",
            "--cwd",
            "/demo",
            "--label",
            "demo-abcd",
            "--no-focus",
        ])
    );
    assert_eq!(plan.tabs.len(), 2);

    // The agent-start argv reuses the launch_argv verbatim after `--`, with a
    // `<WS>` placeholder for the workspace id resolved at run time.
    let first = &plan.tabs[0];
    assert_eq!(first.identity, "root_agent");
    assert_eq!(
        first.agent_start,
        strs(&[
            "herdr",
            "agent",
            "start",
            "root_agent",
            "--workspace",
            "<WS>",
            "--cwd",
            "/demo",
            "--no-focus",
            "--",
            "claude",
            "--foo",
        ])
    );
    let second = &plan.tabs[1];
    assert_eq!(
        second.agent_start,
        strs(&[
            "herdr",
            "agent",
            "start",
            "finance",
            "--workspace",
            "<WS>",
            "--cwd",
            "/demo/finance",
            "--no-focus",
            "--",
            "claude",
            "-n",
            "finance",
        ])
    );
}

#[test]
fn herdr_tab_wait_ready_is_trust_then_enter_then_idle() {
    let tabs = [tab("root_agent", "/demo", &["claude"], &["/loop 30s"])];
    let plan = build_herdr_plan("demo-abcd", &tabs);

    assert_eq!(
        plan.tabs[0].wait_ready,
        vec![
            strs(&[
                "herdr",
                "wait",
                "output",
                "<PANE>",
                "--match",
                "trust",
                "--timeout",
                "20000",
            ]),
            strs(&["herdr", "pane", "send-keys", "<PANE>", "enter"]),
            strs(&[
                "herdr",
                "wait",
                "agent-status",
                "<PANE>",
                "--status",
                "idle",
                "--timeout",
                "35000",
            ]),
        ]
    );
}

#[test]
fn herdr_seed_sends_each_line_by_name_then_submits() {
    let tabs = [tab(
        "root_agent",
        "/demo",
        &["claude"],
        &["/loop 30s", "/goal go"],
    )];
    let plan = build_herdr_plan("demo-abcd", &tabs);

    // Addressed by agent name (`agent send root_agent …`), each followed by a
    // `send-keys <PANE> enter` submit.
    assert_eq!(
        plan.tabs[0].seed,
        vec![
            strs(&["herdr", "agent", "send", "root_agent", "/loop 30s"]),
            strs(&["herdr", "pane", "send-keys", "<PANE>", "enter"]),
            strs(&["herdr", "agent", "send", "root_agent", "/goal go"]),
            strs(&["herdr", "pane", "send-keys", "<PANE>", "enter"]),
        ]
    );
}

#[test]
fn parses_workspace_id_from_create_json() {
    let json = r#"{"result":{"root_pane":{"pane_id":"w4:p1","terminal_id":"term_6570da89722875","workspace_id":"w4"},"workspace":{"workspace_id":"w4","label":"remargin-smoke"}}}"#;
    assert_eq!(parse_workspace_id(json).unwrap(), "w4");
}

#[test]
fn parses_pane_id_from_agent_start_json() {
    let json =
        r#"{"result":{"agent":{"pane_id":"w4:p2","terminal_id":"term_abc123","agent":"claude"}}}"#;
    assert_eq!(parse_pane_id(json).unwrap(), "w4:p2");
}

#[test]
fn parse_workspace_id_errors_on_malformed_json() {
    let err = parse_workspace_id("{not json").unwrap_err().to_string();
    assert!(err.contains("workspace create"), "names the source: {err}");
}

#[test]
fn substitute_replaces_only_exact_placeholder_matches() {
    let argv = strs(&["herdr", "agent", "start", "<WS>", "--cwd", "<WS>path"]);
    assert_eq!(
        substitute(&argv, "<WS>", "w4"),
        strs(&["herdr", "agent", "start", "w4", "--cwd", "<WS>path"])
    );
}

#[test]
fn herdr_unavailable_error_names_the_fix() {
    let err = herdr_unavailable_error().to_string();
    assert!(err.contains("herdr"), "names herdr: {err}");
    assert!(
        err.contains("--multiplexer tmux"),
        "names the tmux fallback: {err}"
    );
}

/// The no-supervision invariant (discussion decisions 3 & 5): the engine
/// must write no PID/registry file. Scan the module source and assert it
/// never references a `.remargin/sessions/` path.
#[test]
fn engine_writes_no_session_registry_path() {
    let src = include_str!("multiplexer.rs");
    assert!(
        !src.contains(".remargin/sessions"),
        "engine must not write a session registry file"
    );
}
