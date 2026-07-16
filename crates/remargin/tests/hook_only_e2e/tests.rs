//! Hook-only enforcement e2e (closer for the pretool single-source-of-truth
//! epic).
//!
//! Proves that once `remargin restrict` stops projecting deny rules, the
//! `PreToolUse` hook alone denies every native-tool and Bash access to a
//! managed path that the retired rules used to catch, that `remargin
//! doctor` flags a realm still carrying legacy projected rules, and that
//! `unrestrict` on such a (migrated) realm reverses cleanly with no
//! dangling sidecar entries. The six scenarios map 1:1 to the task's
//! Testing Plan.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use assert_cmd::cargo::CommandCargoExt as _;
use serde_json::{Value, json};
use tempfile::TempDir;

fn realm_with_claude() -> TempDir {
    let realm = TempDir::new().unwrap();
    fs::create_dir_all(realm.path().join(".claude")).unwrap();
    realm
}

fn user_settings(realm: &TempDir) -> PathBuf {
    realm.path().join("hermetic-user-settings.json")
}

fn restrict(realm_path: &Path, path: &str, user: &Path) {
    let out = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(realm_path)
        .args([
            "claude",
            "restrict",
            path,
            "--user-settings",
            user.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "restrict failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Drive the `PreToolUse` hook binary with a single event envelope.
fn run_pretool(cwd: &Path, tool: &str, tool_input: &Value) -> Output {
    let event = json!({
        "session_id": "e2e",
        "transcript_path": "/tmp/t.jsonl",
        "cwd": cwd.to_string_lossy(),
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": tool_input,
    });
    let bytes = serde_json::to_vec(&event).unwrap();
    let mut child = Command::cargo_bin("remargin")
        .unwrap()
        .args(["claude", "pretool"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(&bytes).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_deny(out: &Output, ctx: &str) {
    assert_eq!(
        out.status.code(),
        Some(0_i32),
        "{ctx}: hook should exit 0 for a deny; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "{ctx}: expected decision JSON on stdout, got empty output"
    );
    let payload: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        payload["hookSpecificOutput"]["permissionDecision"],
        json!("deny"),
        "{ctx}: expected deny, got: {stdout}",
    );
}

fn assert_allow(out: &Output, ctx: &str) {
    assert_eq!(
        out.status.code(),
        Some(0_i32),
        "{ctx}: hook should exit 0 for a silent allow",
    );
    assert!(
        out.stdout.is_empty(),
        "{ctx}: expected silent allow (empty stdout), got: {}",
        String::from_utf8_lossy(&out.stdout),
    );
}

/// Scenario 1: a fresh `restrict` writes no hook-covered deny rules — no
/// settings files, no sidecar, no gitignore. The `.remargin.yaml` entry
/// is the only artifact.
#[test]
fn scenario_1_fresh_restrict_writes_no_hook_covered_rules() {
    let realm = realm_with_claude();
    fs::create_dir_all(realm.path().join("secret")).unwrap();
    let user = user_settings(&realm);
    restrict(realm.path(), "secret", &user);

    assert!(
        !realm.path().join(".claude/settings.local.json").exists(),
        "no project-scope settings should be projected"
    );
    assert!(!user.exists(), "no user-scope settings should be projected");
    assert!(
        !realm
            .path()
            .join(".claude/.remargin-restrictions.json")
            .exists(),
        "no sidecar should be written"
    );
    assert!(
        !realm.path().join(".gitignore").exists(),
        "no gitignore line should be added"
    );
    let yaml = fs::read_to_string(realm.path().join(".remargin.yaml")).unwrap();
    assert!(
        yaml.contains("secret"),
        "yaml should carry the entry: {yaml}"
    );
}

/// Scenario 2: a realm still carrying a legacy projected deny rule is
/// flagged by `doctor` as leftover drift, and the hook still denies the
/// managed path regardless.
#[test]
fn scenario_2_doctor_flags_legacy_projected_rule_hook_still_denies() {
    let realm = realm_with_claude();
    fs::create_dir_all(realm.path().join("secret")).unwrap();
    let home = TempDir::new().unwrap();
    let user = home.path().join(".claude/settings.json");

    // Restrict (writes only the .remargin.yaml entry), then install the
    // enforcement hooks into the home-scope settings so `doctor` does not
    // short-circuit on a missing hook.
    restrict(realm.path(), "secret", &user);
    for cmd in [
        ["claude", "pretool", "install"],
        ["claude", "session-guard", "install"],
    ] {
        let out = Command::cargo_bin("remargin")
            .unwrap()
            .current_dir(realm.path())
            .env("HOME", home.path())
            .args(cmd)
            .output()
            .unwrap();
        assert!(out.status.success(), "{cmd:?} failed");
    }

    // Seed a leftover projected editor deny into the project-scope file —
    // the exact shape an older `restrict` would have written.
    let canonical = fs::canonicalize(realm.path()).unwrap();
    let leftover = format!("Edit({}/secret/**)", canonical.display());
    fs::write(
        realm.path().join(".claude/settings.local.json"),
        json!({ "permissions": { "deny": [leftover] } }).to_string(),
    )
    .unwrap();

    let doctor = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(realm.path())
        .args([
            "doctor",
            "--user-settings",
            user.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    let flagged = report["findings"].as_array().unwrap().iter().any(|f| {
        f["kind"] == "leftover_projected_rule"
            && f["message"].as_str().is_some_and(|m| m.contains(&leftover))
    });
    assert!(
        flagged,
        "doctor should flag the leftover projected rule: {report}"
    );

    // The hook denies the managed path from `.remargin.yaml` alone.
    let target = canonical.join("secret/foo.md");
    let out = run_pretool(realm.path(), "Read", &json!({ "file_path": target }));
    assert_deny(&out, "scenario 2 hook still denies");
}

/// Scenario 3: a hook-only realm denies every native path-touching tool
/// on a managed path.
#[test]
fn scenario_3_native_tools_denied_on_managed_path() {
    let realm = realm_with_claude();
    fs::create_dir_all(realm.path().join("secret")).unwrap();
    let user = user_settings(&realm);
    restrict(realm.path(), "secret", &user);
    let canonical = fs::canonicalize(realm.path()).unwrap();
    let secret = canonical.join("secret");

    for tool in ["Read", "Write", "Edit", "MultiEdit"] {
        let out = run_pretool(
            realm.path(),
            tool,
            &json!({ "file_path": secret.join("foo.md") }),
        );
        assert_deny(&out, &format!("scenario 3 {tool}"));
    }

    let notebook = run_pretool(
        realm.path(),
        "NotebookEdit",
        &json!({ "notebook_path": secret.join("nb.ipynb") }),
    );
    assert_deny(&notebook, "scenario 3 NotebookEdit");

    for tool in ["Grep", "Glob"] {
        let out = run_pretool(realm.path(), tool, &json!({ "path": secret }));
        assert_deny(&out, &format!("scenario 3 {tool}"));
    }
}

/// Scenario 4: a hook-only realm denies every Bash bypass on the
/// shell-parsing regression list (chained mutator, pipe tee, subshell cd,
/// plain cd, cat read, quoted prefix, glob segment) plus mv source- and
/// destination-side shapes. The symlink case is covered by a unit test.
#[test]
fn scenario_4_bash_bypasses_denied_on_managed_path() {
    let realm = realm_with_claude();
    fs::create_dir_all(realm.path().join("secret")).unwrap();
    let user = user_settings(&realm);
    restrict(realm.path(), "secret", &user);
    let canonical = fs::canonicalize(realm.path()).unwrap();
    let root = canonical.display();
    let secret = format!("{root}/secret");
    // The session cwd sits outside the realm so bare verbs cannot self-deny.
    let outside = TempDir::new().unwrap();

    let commands: [(&str, String); 9] = [
        ("chained mutator", format!("true && rm {secret}/x.md")),
        ("pipe tee", format!("echo hi | tee {secret}/x.md")),
        // cd into the (unrestricted) realm root, then a relative mutator
        // that the tracked cwd resolves back into the managed subtree.
        ("subshell cd", format!("(cd {root} && rm secret/x.md)")),
        ("plain cd", format!("cd {root} && rm secret/x.md")),
        ("cat read", format!("cat {secret}/x.md")),
        ("quoted prefix", format!("rm \"{secret}/x.md\"")),
        ("glob segment", format!("rm {root}/sec*/x.md")),
        ("mv source-side", format!("mv {secret}/x.md /tmp/out.md")),
        ("mv dest-side", format!("mv /tmp/in.md {secret}/x.md")),
    ];

    for (label, command) in &commands {
        let out = run_pretool(outside.path(), "Bash", &json!({ "command": command }));
        assert_deny(&out, &format!("scenario 4 {label}: {command}"));
    }
}

/// Scenario 5: a dot folder the realm explicitly re-allows via
/// `allow_dot_folders` is permitted, while other dot folders and non-dot
/// paths under the (wildcard) realm stay denied.
#[test]
fn scenario_5_allowed_dot_folder_is_permitted() {
    let realm = realm_with_claude();
    fs::create_dir_all(realm.path().join(".obsidian")).unwrap();
    let user = user_settings(&realm);
    restrict(realm.path(), "*", &user);

    // Augment the wildcard realm with an explicit dot-folder re-allow.
    let yaml = fs::read_to_string(realm.path().join(".remargin.yaml")).unwrap();
    fs::write(
        realm.path().join(".remargin.yaml"),
        format!("{yaml}  allow_dot_folders:\n    - .obsidian\n"),
    )
    .unwrap();
    let canonical = fs::canonicalize(realm.path()).unwrap();

    let allowed = run_pretool(
        realm.path(),
        "Read",
        &json!({ "file_path": canonical.join(".obsidian/workspace.json") }),
    );
    assert_allow(&allowed, "scenario 5 allowed dot folder");

    let unlisted = run_pretool(
        realm.path(),
        "Read",
        &json!({ "file_path": canonical.join(".git/config") }),
    );
    assert_deny(&unlisted, "scenario 5 unlisted dot folder");

    let non_dot = run_pretool(
        realm.path(),
        "Read",
        &json!({ "file_path": canonical.join("notes/a.md") }),
    );
    assert_deny(&non_dot, "scenario 5 non-dot path");
}

/// Scenario 6: `unrestrict` on a migrated realm (a legacy sidecar full of
/// projected rules that are still present in the settings files) reverses
/// cleanly — the rules are scrubbed, the sidecar entry is removed with no
/// dangling entry, and the `.remargin.yaml` entry is gone.
#[test]
fn scenario_6_unrestrict_migrated_realm_leaves_no_dangling_sidecar() {
    let realm = realm_with_claude();
    fs::create_dir_all(realm.path().join("secret")).unwrap();
    let user = user_settings(&realm);
    let project = realm.path().join(".claude/settings.local.json");
    let canonical = fs::canonicalize(realm.path()).unwrap();
    let key = format!("{}/secret", canonical.display());
    let rules = [format!("Edit({key}/**)"), format!("Write({key}/**)")];

    // Seed the legacy state: yaml entry + both settings files carrying the
    // projected rules + a sidecar tracking them.
    fs::write(
        realm.path().join(".remargin.yaml"),
        "permissions:\n  trusted_roots:\n    - path: secret\n",
    )
    .unwrap();
    let settings_body = json!({ "permissions": { "deny": rules } }).to_string();
    fs::write(&project, &settings_body).unwrap();
    fs::write(&user, &settings_body).unwrap();
    let sidecar = json!({
        "version": 1_u32,
        "entries": {
            key: {
                "added_at": "legacy",
                "added_to_files": [project.to_string_lossy(), user.to_string_lossy()],
                "allow": [],
                "deny": rules,
            }
        }
    });
    fs::write(
        realm.path().join(".claude/.remargin-restrictions.json"),
        sidecar.to_string(),
    )
    .unwrap();

    let out = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(realm.path())
        .args([
            "claude",
            "unrestrict",
            "secret",
            "--user-settings",
            user.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "unrestrict failed:\nstderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // No dangling sidecar entry.
    let sidecar_after: Value = serde_json::from_str(
        &fs::read_to_string(realm.path().join(".claude/.remargin-restrictions.json")).unwrap(),
    )
    .unwrap();
    assert!(
        sidecar_after["entries"].as_object().unwrap().is_empty(),
        "sidecar entries should be empty after unrestrict: {sidecar_after}"
    );

    // The legacy rules were scrubbed from both settings files.
    for file in [&project, &user] {
        let value: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        let deny = value["permissions"]["deny"].as_array().unwrap();
        assert!(
            deny.is_empty(),
            "legacy rules should be scrubbed from {}: {deny:?}",
            file.display()
        );
    }

    // The .remargin.yaml entry is gone.
    let yaml = fs::read_to_string(realm.path().join(".remargin.yaml")).unwrap_or_default();
    assert!(
        !yaml.contains("secret"),
        "yaml entry should be removed: {yaml}"
    );
}
