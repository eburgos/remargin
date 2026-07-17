use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_fixture(dir: &TempDir, name: &str, contents: &str) {
    fs::write(dir.path().join(name), contents).unwrap();
}

#[test]
fn get_json_returns_links_array() {
    let tmp = TempDir::new().unwrap();
    write_fixture(
        &tmp,
        "doc.md",
        "See [[Budget]] and [external](https://example.com/x).\nAlso [[Budget]] again.\n",
    );
    write_fixture(&tmp, "Budget.md", "---\ntitle: Q3 Budget\n---\n# Budget\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Content is unchanged (additive).
    assert!(payload["content"].as_str().unwrap().contains("[[Budget]]"));

    // Local links only: the external URL is dropped entirely.
    let links = payload["links"].as_array().unwrap();
    assert_eq!(links.len(), 1, "only the local link survives: {links:?}");
    assert!(
        links.iter().all(|l| l["target"] != "https://example.com/x"),
        "external URL must be absent: {links:?}"
    );

    let budget = links.iter().find(|l| l["target"] == "Budget").unwrap();
    assert_eq!(budget["path"], "Budget.md");
    assert_eq!(budget["title"], "Q3 Budget");
    assert_eq!(budget["count"], 2_i32);
    assert_eq!(budget["lines"].as_array().unwrap().len(), 2);

    // No null keys: absent optionals are omitted, every link has a path.
    let budget_map = budget.as_object().unwrap();
    assert!(!budget_map.values().any(serde_json::Value::is_null));
    assert!(budget_map.contains_key("path"));
}

#[test]
fn get_json_drops_broken_internal_links() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "[[Exists]] and [[Missing]].\n");
    write_fixture(&tmp, "Exists.md", "# Exists\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let links = payload["links"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"], "Exists");
}

#[test]
fn get_pretty_renders_links_block() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "Link to [[Notes]] here.\n");
    write_fixture(&tmp, "Notes.md", "# Notes\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Link to [[Notes]] here."));
    assert!(
        stdout.contains("Links (1)"),
        "missing links block: {stdout}"
    );
    assert!(stdout.contains("Notes"));
    assert!(stdout.contains("Notes.md"));
}

#[test]
fn get_pretty_suppresses_block_at_zero_links() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "No links at all here.\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("Links ("),
        "block should be suppressed: {stdout}"
    );
}

#[test]
fn get_json_empty_links_when_none() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "Nothing to see here.\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["links"].as_array().unwrap().len(), 0);
}

#[test]
fn get_compact_line_numbers_shape_minified() {
    let tmp = TempDir::new().unwrap();
    write_fixture(
        &tmp,
        "doc.md",
        "See [[Budget]] here.\nSecond line [[Budget]] again.\n",
    );
    write_fixture(&tmp, "Budget.md", "---\ntitle: Q3 Budget\n---\n# Budget\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md", "--json", "--compact", "--line-numbers"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let raw = String::from_utf8(output.stdout).unwrap();
    // Minified: one payload line (only the trailing newline).
    assert_eq!(
        raw.trim_end_matches('\n').lines().count(),
        1,
        "minified single line: {raw:?}"
    );

    let payload: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
    assert_eq!(payload["start_line"], 1_i32);
    let lines = payload["lines"].as_array().unwrap();
    assert!(lines[0].is_string(), "lines are bare strings: {lines:?}");
    assert!(lines[0].as_str().unwrap().contains("[[Budget]]"));
    assert_eq!(
        payload["links_cols"],
        serde_json::json!(["alias", "lines", "target", "title"])
    );
    let link_rows = payload["links"].as_array().unwrap();
    assert_eq!(link_rows.len(), 1);
    let row = link_rows[0].as_array().unwrap();
    assert_eq!(row.len(), 4, "count/path dropped: {row:?}");
    assert!(row[0].is_null());
    assert_eq!(row[2], "Budget");
    assert_eq!(row[3], "Q3 Budget");
    assert!(payload.get("content").is_none());
}

#[test]
fn get_compact_no_line_numbers_shape() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "See [[Budget]] here.\n");
    write_fixture(&tmp, "Budget.md", "---\ntitle: Q3 Budget\n---\n# Budget\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md", "--json", "--compact"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let raw = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        raw.trim_end_matches('\n').lines().count(),
        1,
        "minified single line: {raw:?}"
    );

    let payload: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
    assert!(payload["content"].as_str().unwrap().contains("[[Budget]]"));
    assert_eq!(
        payload["links_cols"],
        serde_json::json!(["alias", "lines", "target", "title"])
    );
    assert!(payload.get("start_line").is_none());
    assert!(payload.get("lines").is_none());
    assert_eq!(payload["links"][0][2], "Budget");
}

#[test]
fn get_compact_requires_json() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "hi\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md", "--compact"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected clap failure: {output:?}"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("--json"),
        "clap requires error must name --json: {stderr}"
    );
}

#[test]
fn get_compact_link_path_derivable_from_target() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "Bare [[Note]] and embed ![[img.png]].\n");
    write_fixture(&tmp, "Note.md", "# Note\n");
    write_fixture(&tmp, "img.png", "fakebytes");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md", "--json", "--compact"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let links = payload["links"].as_array().unwrap();
    for row in links {
        let cols = row.as_array().unwrap();
        assert_eq!(cols.len(), 4, "rows carry no path/count column: {cols:?}");
        let target = cols[2].as_str().unwrap();
        // Derive the on-disk path per the documented rule and confirm it
        // resolves to a real file in the vault.
        let derived = if Path::new(target).extension().is_some() {
            target.to_owned()
        } else {
            format!("{target}.md")
        };
        assert!(
            tmp.path().join(&derived).exists(),
            "derived path must exist: {derived}"
        );
    }
    let targets: Vec<&str> = links.iter().map(|r| r[2].as_str().unwrap()).collect();
    assert!(targets.contains(&"Note"));
    assert!(targets.contains(&"img.png"));
}

/// Regression: `--json` (no `--compact`) keeps today's verbose,
/// pretty-printed shape — `{line, text}` line objects and verbose link
/// rows carrying `count` + `path`. Compact must not leak in.
#[test]
fn get_verbose_json_line_numbers_unchanged() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "See [[Budget]] here.\n");
    write_fixture(&tmp, "Budget.md", "---\ntitle: Q3 Budget\n---\n# Budget\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["get", "doc.md", "--json", "--line-numbers"])
        .output()
        .unwrap();

    assert!(output.status.success(), "command failed: {output:?}");
    let raw = String::from_utf8(output.stdout).unwrap();
    // Verbose stays pretty-printed (multi-line).
    assert!(raw.lines().count() > 3, "pretty-printed: {raw:?}");

    let payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let lines = payload["lines"].as_array().unwrap();
    let first = lines[0].as_object().unwrap();
    assert!(first.contains_key("line") && first.contains_key("text"));
    assert!(payload.get("start_line").is_none());
    assert!(payload.get("links_cols").is_none());

    let link_rows = payload["links"].as_array().unwrap();
    let budget = link_rows.iter().find(|l| l["target"] == "Budget").unwrap();
    assert_eq!(budget["count"], 1_i32);
    assert_eq!(budget["path"], "Budget.md");
}

/// `--compact` on a subcommand that does not emit the compact contract is
/// rejected at the dispatch layer with a clear message, not silently
/// ignored. `comment` is the exemplar non-`get` subcommand.
#[test]
fn compact_rejected_for_unsupported_subcommand() {
    let tmp = TempDir::new().unwrap();
    write_fixture(&tmp, "doc.md", "Body text.\n");

    let output = Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(tmp.path())
        .args(["comment", "doc.md", "hi", "--json", "--compact"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "expected rejection: {output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("--compact is not supported for this subcommand"),
        "clear rejection message: {stderr}"
    );
}
