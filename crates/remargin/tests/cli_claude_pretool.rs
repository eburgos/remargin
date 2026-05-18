//! `remargin claude pretool` integration tests.
//!
//! Exercises the CLI subcommand against a real-filesystem temp realm.
//! The Claude Code stdin/stdout/exit-code contract is the source of
//! truth — every test pipes an envelope into the binary and asserts on
//! stdout, stderr, and exit code.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Output, Stdio};

    use assert_cmd::cargo::CommandCargoExt as _;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    fn realm_with_claude() -> TempDir {
        let realm = TempDir::new().unwrap();
        fs::create_dir_all(realm.path().join(".claude")).unwrap();
        realm
    }

    fn run_pretool(stdin_bytes: &[u8]) -> Output {
        use std::io::Write as _;
        let mut child = Command::cargo_bin("remargin")
            .unwrap()
            .args(["claude", "pretool"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(stdin_bytes)
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn restrict_in(realm_path: &Path, path: &str, user_settings: &Path) {
        let out = Command::cargo_bin("remargin")
            .unwrap()
            .current_dir(realm_path)
            .args([
                "claude",
                "restrict",
                path,
                "--user-settings",
                user_settings.to_str().unwrap(),
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

    fn envelope(tool: &str, cwd: &Path, tool_input: &Value) -> Vec<u8> {
        let event = json!({
            "session_id": "test",
            "transcript_path": "/tmp/t.jsonl",
            "cwd": cwd.to_string_lossy(),
            "hook_event_name": "PreToolUse",
            "tool_name": tool,
            "tool_input": tool_input,
        });
        serde_json::to_vec(&event).unwrap()
    }

    /// Scenario 21: end-to-end against a real `claude restrict`-ed
    /// realm. The hook denies the Read with the canonical message.
    #[test]
    fn end_to_end_against_real_claude_restricted_realm() {
        let realm = realm_with_claude();
        fs::create_dir_all(realm.path().join("secret")).unwrap();
        let user_settings = realm.path().join("hermetic-user-settings.json");
        restrict_in(realm.path(), "secret", &user_settings);

        let target = realm.path().join("secret/foo.md");
        let stdin = envelope("Read", realm.path(), &json!({ "file_path": target }));

        let out = run_pretool(&stdin);
        assert_eq!(out.status.code(), Some(0_i32));
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert!(!stdout.is_empty());
        let payload: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(
            payload["hookSpecificOutput"]["hookEventName"],
            json!("PreToolUse")
        );
        assert_eq!(
            payload["hookSpecificOutput"]["permissionDecision"],
            json!("deny")
        );
        let reason = payload["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap();
        assert!(reason.contains("mcp__remargin__get"));
    }

    /// Scenario 22: exit 0 with empty stdout when the path is
    /// unrestricted.
    #[test]
    fn unrestricted_call_exits_zero_with_empty_stdout() {
        let realm = realm_with_claude();
        fs::create_dir_all(realm.path().join("secret")).unwrap();
        fs::create_dir_all(realm.path().join("public")).unwrap();
        let user_settings = realm.path().join("hermetic-user-settings.json");
        restrict_in(realm.path(), "secret", &user_settings);

        let target = realm.path().join("public/foo.md");
        let stdin = envelope("Read", realm.path(), &json!({ "file_path": target }));

        let out = run_pretool(&stdin);
        assert_eq!(out.status.code(), Some(0_i32));
        assert!(
            out.stdout.is_empty(),
            "expected empty stdout for silent allow"
        );
    }

    /// Scenario 23: malformed stdin exits 2 with a non-empty stderr
    /// (Claude Code feeds stderr back to the model on exit 2).
    #[test]
    fn malformed_stdin_exits_two_with_stderr() {
        let out = run_pretool(b"not json");
        assert_eq!(out.status.code(), Some(2_i32));
        assert!(!out.stderr.is_empty(), "expected stderr diagnostic");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("malformed PreToolUse event"));
    }

    /// Scenario 24: env-var prefix on a Bash command does not hide the
    /// real verb from the extractor.
    #[test]
    fn env_var_prefix_does_not_hide_verb() {
        let realm = realm_with_claude();
        fs::create_dir_all(realm.path().join("secret")).unwrap();
        let user_settings = realm.path().join("hermetic-user-settings.json");
        restrict_in(realm.path(), "secret", &user_settings);

        let target = realm.path().join("secret/x");
        let command = format!("FOO=bar  rm {}", target.display());
        let stdin = envelope("Bash", realm.path(), &json!({ "command": command }));

        let out = run_pretool(&stdin);
        assert_eq!(out.status.code(), Some(0_i32));
        let stdout = String::from_utf8(out.stdout).unwrap();
        let payload: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(
            payload["hookSpecificOutput"]["permissionDecision"],
            json!("deny")
        );
    }

    /// Scenario 25: the JSON wire shape matches Claude Code's
    /// `PreToolUse` hook contract verbatim — keys are `camelCase`,
    /// decision is lowercase, `hookEventName` is exactly `"PreToolUse"`.
    #[test]
    fn decision_json_matches_claude_code_contract() {
        let realm = realm_with_claude();
        fs::create_dir_all(realm.path().join("secret")).unwrap();
        let user_settings = realm.path().join("hermetic-user-settings.json");
        restrict_in(realm.path(), "secret", &user_settings);

        let target = realm.path().join("secret/foo.md");
        let stdin = envelope(
            "Edit",
            realm.path(),
            &json!({
                "file_path": target,
                "old_string": "a",
                "new_string": "b",
            }),
        );

        let out = run_pretool(&stdin);
        assert_eq!(out.status.code(), Some(0_i32));
        let stdout = String::from_utf8(out.stdout).unwrap();
        let payload: Value = serde_json::from_str(&stdout).unwrap();
        let inner = &payload["hookSpecificOutput"];
        assert_eq!(inner["hookEventName"], json!("PreToolUse"));
        assert_eq!(inner["permissionDecision"], json!("deny"));
        assert!(
            inner["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("mcp__remargin__edit")
        );
        // No extra top-level keys snuck in.
        let obj = payload.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("hookSpecificOutput"));
    }
}
