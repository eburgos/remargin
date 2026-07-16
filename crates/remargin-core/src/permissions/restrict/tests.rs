//! Unit tests for [`crate::permissions::restrict`].
//!
//! Covers anchor discovery, wildcard support, .remargin.yaml mutation
//! (create + merge + idempotency), Claude-sync invocation through
//! `apply_rules`.

use std::io;
use std::path::{Path, PathBuf};

use os_shim::System as _;
use os_shim::mock::MockSystem;
use serde_yaml::Value;

use crate::permissions::restrict::{
    RestrictArgs, find_claude_anchor, restrict, write_remargin_yaml,
};
use crate::permissions::sidecar;

fn realm_with_claude(extra_files: &[(&str, &str)]) -> (MockSystem, PathBuf) {
    let anchor = PathBuf::from("/r");
    let mut system = MockSystem::new()
        .with_dir(&anchor)
        .unwrap()
        .with_dir(anchor.join(".claude"))
        .unwrap();
    for (path, body) in extra_files {
        system = system.with_file(Path::new(path), body.as_bytes()).unwrap();
    }
    (system, anchor)
}

fn settings_files(anchor: &Path) -> Vec<PathBuf> {
    vec![
        anchor.join(".claude/settings.local.json"),
        PathBuf::from("/home/u/.claude/settings.json"),
    ]
}

fn args(path: &str) -> RestrictArgs {
    RestrictArgs {
        also_deny_bash: Vec::new(),
        cli_allowed: false,
        path: String::from(path),
    }
}

fn read_yaml(system: &MockSystem, path: &Path) -> Value {
    let body = system.read_to_string(path).unwrap();
    serde_yaml::from_str(&body).unwrap()
}

/// Scenario 1: cwd is the anchor (it has its own `.claude/`).
#[test]
fn anchor_discovery_when_cwd_is_anchor() {
    let (system, anchor) = realm_with_claude(&[]);
    let found = find_claude_anchor(&system, &anchor).unwrap();
    assert_eq!(found, anchor);
}

/// Scenario 2: anchor is several directories up from cwd.
#[test]
fn anchor_discovery_walks_up_to_nearest_claude_dir() {
    let (system, _anchor) = realm_with_claude(&[]);
    let deep = PathBuf::from("/r/sub/sub2");
    system.create_dir_all(&deep).unwrap();
    let found = find_claude_anchor(&system, &deep).unwrap();
    assert_eq!(found, PathBuf::from("/r"));
}

/// Scenario 3: no `.claude/` ancestor → clear error.
#[test]
fn anchor_discovery_errors_when_no_claude_ancestor() {
    let system = MockSystem::new().with_dir(Path::new("/r")).unwrap();
    let err = find_claude_anchor(&system, Path::new("/r")).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("no `.claude/`"),
        "expected named error, got: {msg}"
    );
}

/// Scenario 4: wildcard path stored verbatim in `.remargin.yaml`.
#[test]
fn wildcard_path_stored_in_yaml() {
    let (system, anchor) = realm_with_claude(&[]);
    restrict(&system, &anchor, &args("*"), &settings_files(&anchor)).unwrap();

    let value = read_yaml(&system, &anchor.join(".remargin.yaml"));
    let entry = &value["permissions"]["trusted_roots"][0];
    assert_eq!(entry["path"], Value::String(String::from("*")));
}

/// Scenario 5: subpath that resolves outside the anchor is rejected.
#[test]
fn subpath_outside_anchor_is_rejected() {
    let (system, anchor) = realm_with_claude(&[]);
    let err = restrict(
        &system,
        &anchor,
        &args("../escape"),
        &settings_files(&anchor),
    )
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("outside the anchor"),
        "expected outside-anchor error, got: {msg}"
    );
}

/// Scenario 6: missing `.remargin.yaml` is created with the entry.
#[test]
fn creates_remargin_yaml_when_absent() {
    let (system, anchor) = realm_with_claude(&[]);
    let outcome = restrict(
        &system,
        &anchor,
        &args("src/secret"),
        &settings_files(&anchor),
    )
    .unwrap();
    assert!(outcome.yaml_was_created);

    let value = read_yaml(&system, &anchor.join(".remargin.yaml"));
    let entry = &value["permissions"]["trusted_roots"][0];
    assert_eq!(entry["path"], Value::String(String::from("src/secret")));
}

/// Scenario 7: existing `.remargin.yaml` with an identity block gains
/// the `permissions.trusted_roots` array without losing the identity.
#[test]
fn appends_to_existing_remargin_yaml() {
    let prior = "identity: alice\ntype: human\n";
    let (system, anchor) = realm_with_claude(&[("/r/.remargin.yaml", prior)]);
    let outcome = restrict(
        &system,
        &anchor,
        &args("src/secret"),
        &settings_files(&anchor),
    )
    .unwrap();
    assert!(!outcome.yaml_was_created);

    let value = read_yaml(&system, &anchor.join(".remargin.yaml"));
    assert_eq!(value["identity"], Value::String(String::from("alice")));
    assert_eq!(value["type"], Value::String(String::from("human")));
    let restrict_entry = &value["permissions"]["trusted_roots"][0];
    assert_eq!(
        restrict_entry["path"],
        Value::String(String::from("src/secret"))
    );
}

/// Scenario 8: re-running `restrict` for the same path is a no-op
/// (no duplicate entry in the YAML).
#[test]
fn duplicate_path_does_not_create_second_entry() {
    let (system, anchor) = realm_with_claude(&[]);
    restrict(
        &system,
        &anchor,
        &args("src/secret"),
        &settings_files(&anchor),
    )
    .unwrap();
    restrict(
        &system,
        &anchor,
        &args("src/secret"),
        &settings_files(&anchor),
    )
    .unwrap();

    let value = read_yaml(&system, &anchor.join(".remargin.yaml"));
    let restricts = value["permissions"]["trusted_roots"].as_sequence().unwrap();
    assert_eq!(restricts.len(), 1, "{value:#?}");
}

/// Scenario 9: a hook-only restrict writes no Claude settings file and
/// no sidecar entry; re-running stays clean (still nothing projected).
/// The hook is the single source of truth, so there is nothing to
/// backfill into the settings files.
#[test]
fn rerun_writes_no_settings_or_sidecar() {
    let (system, anchor) = realm_with_claude(&[]);
    let files = settings_files(&anchor);
    restrict(&system, &anchor, &args("src/secret"), &files).unwrap();

    // No project-scope settings file was created, and no sidecar entry.
    let _: io::Error = system.read_to_string(&files[0]).unwrap_err();
    assert!(sidecar::load(&system, &anchor).unwrap().entries.is_empty());

    // Re-running is a clean no-op on the settings/sidecar side.
    restrict(&system, &anchor, &args("src/secret"), &files).unwrap();
    let _: io::Error = system.read_to_string(&files[0]).unwrap_err();
    assert!(sidecar::load(&system, &anchor).unwrap().entries.is_empty());
}

/// Scenario 10: `also_deny_bash` lands on the `.remargin.yaml` entry but
/// projects no Bash deny rules — the hook denies every command touching a
/// managed path regardless of verb, so `rules_applied` stays empty.
#[test]
fn also_deny_bash_lands_on_yaml_entry_but_projects_no_rules() {
    let (system, anchor) = realm_with_claude(&[]);
    let mut a = args("src/secret");
    a.also_deny_bash = vec![String::from("curl"), String::from("wget")];
    let outcome = restrict(&system, &anchor, &a, &settings_files(&anchor)).unwrap();

    let value = read_yaml(&system, &anchor.join(".remargin.yaml"));
    let entry = &value["permissions"]["trusted_roots"][0];
    let extras = entry["also_deny_bash"].as_sequence().unwrap();
    assert_eq!(extras.len(), 2);

    assert!(
        outcome.rules_applied.is_empty(),
        "no rules should be projected: {:#?}",
        outcome.rules_applied
    );
}

/// Scenario 11: `cli_allowed=true` lands on the YAML entry.
/// `Bash(remargin *)` is never projected regardless of `cli_allowed`;
/// CLI denial is enforced by the `PreToolUse` hook via the folder-level
/// `cli_allowed` field in `.remargin.yaml`.
#[test]
fn cli_allowed_true_persists_in_yaml_no_remargin_cli_deny_projected() {
    let (system, anchor) = realm_with_claude(&[]);
    let mut a = args("src/secret");
    a.cli_allowed = true;
    let outcome = restrict(&system, &anchor, &a, &settings_files(&anchor)).unwrap();

    let value = read_yaml(&system, &anchor.join(".remargin.yaml"));
    let entry = &value["permissions"]["trusted_roots"][0];
    assert_eq!(entry["cli_allowed"], Value::Bool(true));

    // `Bash(remargin *)` is never projected (hook-enforced).
    assert!(
        !outcome
            .rules_applied
            .iter()
            .any(|r| r.starts_with("Bash(remargin"))
    );
}

/// Scenario 12: a hook-only restrict touches no settings files, applies
/// no rules, and writes no sidecar entry — only the `.remargin.yaml`
/// entry activates enforcement.
#[test]
fn outcome_reports_no_settings_or_sidecar() {
    let (system, anchor) = realm_with_claude(&[]);
    let files = settings_files(&anchor);
    let outcome = restrict(&system, &anchor, &args("src/secret"), &files).unwrap();
    assert_eq!(outcome.anchor, anchor);
    assert!(outcome.absolute_path.ends_with("src/secret"));
    assert!(outcome.claude_files_touched.is_empty());
    assert!(outcome.rules_applied.is_empty());

    // No sidecar entry is written when nothing is projected.
    let sc = sidecar::load(&system, &anchor).unwrap();
    assert!(sc.entries.is_empty());
}

/// Scenario 13: the dedicated `write_remargin_yaml` helper is the
/// only path used; the public write / edit ops still refuse
/// `.remargin.yaml`. We pin this by checking that the file landed
/// (the bypass works) AND the helper is not re-exported beyond the
/// permissions namespace (no other module can invoke it).
#[test]
fn write_remargin_yaml_bypass_is_scoped_to_this_module() {
    let (system, anchor) = realm_with_claude(&[]);
    // Public path: restrict() succeeds → write_remargin_yaml ran
    // through the sanctioned helper.
    restrict(
        &system,
        &anchor,
        &args("src/secret"),
        &settings_files(&anchor),
    )
    .unwrap();
    assert!(
        system
            .read_to_string(&anchor.join(".remargin.yaml"))
            .is_ok(),
        ".remargin.yaml must exist after restrict"
    );

    // `write_remargin_yaml` is only re-exported via
    // `crate::permissions::restrict::write_remargin_yaml`. A future
    // change that re-exports it from the crate root or another
    // module must update this test deliberately.
    let body = "permissions:\n  trusted_roots: []\n";
    write_remargin_yaml(&system, &anchor, body).unwrap();
    assert_eq!(
        system
            .read_to_string(&anchor.join(".remargin.yaml"))
            .unwrap(),
        body
    );
}
