//! `remargin search` smoke tests.

#[cfg(test)]
mod tests {
    use core::str;
    use std::fs;

    use assert_cmd::Command;
    use tempfile::TempDir;

    const DOC: &str = "---\ntitle: Test\n---\n\n# Hello\n\nneedle is in the body\n";

    #[test]
    fn search_finds_literal_pattern_in_body() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("doc.md"), DOC).unwrap();

        let output = Command::cargo_bin("remargin")
            .unwrap()
            .current_dir(tmp.path())
            .env("HOME", tmp.path())
            .arg("search")
            .arg("needle")
            .output()
            .unwrap();
        assert!(output.status.success(), "command failed: {output:?}");
        let stdout = str::from_utf8(&output.stdout).unwrap();
        assert!(
            stdout.contains("needle"),
            "stdout missing match: {stdout:?}"
        );
    }

    #[test]
    fn search_json_mode_emits_match_array() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("doc.md"), DOC).unwrap();

        let output = Command::cargo_bin("remargin")
            .unwrap()
            .current_dir(tmp.path())
            .env("HOME", tmp.path())
            .arg("search")
            .arg("needle")
            .arg("--json")
            .output()
            .unwrap();
        assert!(output.status.success(), "command failed: {output:?}");

        let stdout = str::from_utf8(&output.stdout).unwrap();
        let json: serde_json::Value = serde_json::from_str(stdout).unwrap();
        let matches = json["matches"].as_array().unwrap();
        assert!(!matches.is_empty(), "expected matches: {stdout}");
    }
}
