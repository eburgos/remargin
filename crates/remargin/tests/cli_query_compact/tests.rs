use core::str;
use std::fs;
use std::path::{Path, PathBuf};

use std::process::Output;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const CONFIG: &str = "identity: alice\ntype: human\nmode: open\n";

/// Two comments: a directed root (unacked) and a reply carrying
/// `reply_to` / `thread` and an ack — exercises nullable columns, the
/// ack-string compaction, and the integrity columns.
const DOC: &str = "\
---
title: Compact
---

```remargin
---
id: aaa
author: alice
type: human
ts: 2026-04-06T10:00:00-04:00
to: [bob]
checksum: sha256:aaa
---
First comment.
```

```remargin
---
id: bbb
author: bob
type: agent
ts: 2026-04-06T11:00:00-04:00
reply-to: aaa
thread: aaa
checksum: sha256:bbb
ack:
  - alice@2026-04-06T12:00:00-04:00
---
Reply comment.
```
";

fn setup() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".remargin.yaml"), CONFIG).unwrap();
    fs::write(tmp.path().join("doc.md"), DOC).unwrap();
    let path = tmp.path().to_path_buf();
    (tmp, path)
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::cargo_bin("remargin")
        .unwrap()
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

fn row_for<'row>(comments: &'row [Value], id: &str) -> &'row Vec<Value> {
    comments
        .iter()
        .map(|c| c.as_array().unwrap())
        .find(|row| row[0].as_str() == Some(id))
        .unwrap()
}

#[test]
fn query_compact_columnar_minified() {
    let (_tmp, cwd) = setup();
    let out = run(&cwd, &["query", ".", "--json", "--compact"]);
    assert!(out.status.success(), "command failed: {out:?}");

    let raw = str::from_utf8(&out.stdout).unwrap();
    // Minified: only the trailing newline breaks the single payload line.
    assert_eq!(
        raw.trim_end_matches('\n').lines().count(),
        1,
        "compact payload must be minified: {raw:?}"
    );

    let payload: Value = serde_json::from_str(raw.trim()).unwrap();
    // Base header: 14 columns, `content` last, no integrity / file columns.
    let cols = payload["comment_cols"].as_array().unwrap();
    assert_eq!(cols.len(), 14);
    assert_eq!(cols[13], "content");
    assert!(
        !cols
            .iter()
            .any(|c| c == "checksum" || c == "signature" || c == "file")
    );

    let results = payload["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    let comments = results[0]["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);

    // Root comment: nullable reply_to is null; content last.
    let aaa = row_for(comments, "aaa");
    assert_eq!(aaa.len(), 14);
    assert_eq!(aaa[2].as_str().unwrap(), "alice");
    assert_eq!(aaa[3].as_str().unwrap(), "human");
    assert!(aaa[5].is_null(), "reply_to null: {aaa:?}");
    assert_eq!(aaa[13].as_str().unwrap(), "First comment.");

    // Reply: reply_to / thread populated; ack compacts to "author@ts".
    let bbb = row_for(comments, "bbb");
    assert_eq!(bbb[5].as_str().unwrap(), "aaa");
    assert_eq!(bbb[6].as_str().unwrap(), "aaa");
    let acks = bbb[8].as_array().unwrap();
    assert_eq!(acks.len(), 1);
    assert_eq!(acks[0].as_str().unwrap(), "alice@2026-04-06T12:00:00-04:00");
}

#[test]
fn query_compact_include_integrity_widens_rows() {
    let (_tmp, cwd) = setup();
    let out = run(
        &cwd,
        &["query", ".", "--json", "--compact", "--include-integrity"],
    );
    assert!(out.status.success(), "command failed: {out:?}");

    let payload: Value = serde_json::from_slice(&out.stdout).unwrap();
    let cols = payload["comment_cols"].as_array().unwrap();
    assert_eq!(cols.len(), 16);
    assert_eq!(cols[13], "checksum");
    assert_eq!(cols[14], "signature");
    assert_eq!(cols[15], "content");

    let comments = payload["results"][0]["comments"].as_array().unwrap();
    let aaa = row_for(comments, "aaa");
    assert_eq!(aaa.len(), 16);
    assert_eq!(aaa[13].as_str().unwrap(), "sha256:aaa");
    assert!(aaa[14].is_null(), "unsigned signature null: {aaa:?}");
    assert_eq!(aaa[15].as_str().unwrap(), "First comment.");
}

/// Regression: `--json` (no `--compact`) keeps today's verbose, pretty
/// payload — named-field comment objects carrying checksum, no columnar
/// header. Compact must not leak in.
#[test]
fn query_verbose_json_unchanged() {
    let (_tmp, cwd) = setup();
    let out = run(&cwd, &["query", ".", "--json"]);
    assert!(out.status.success(), "command failed: {out:?}");

    let raw = str::from_utf8(&out.stdout).unwrap();
    assert!(
        raw.lines().count() > 3,
        "verbose stays pretty-printed: {raw:?}"
    );

    let payload: Value = serde_json::from_str(raw).unwrap();
    assert!(payload.get("comment_cols").is_none());
    let comments = payload["results"][0]["comments"].as_array().unwrap();
    let first = comments[0].as_object().unwrap();
    assert!(first.contains_key("id"));
    assert!(first.contains_key("checksum"));
    assert!(first.contains_key("file"));
}

#[test]
fn query_include_integrity_requires_compact() {
    let (_tmp, cwd) = setup();
    let out = run(&cwd, &["query", ".", "--json", "--include-integrity"]);
    assert!(!out.status.success(), "expected clap failure: {out:?}");
    let stderr = str::from_utf8(&out.stderr).unwrap();
    assert!(
        stderr.contains("--compact"),
        "clap requires error must name --compact: {stderr}"
    );
}

#[test]
fn query_compact_requires_json() {
    let (_tmp, cwd) = setup();
    let out = run(&cwd, &["query", ".", "--compact"]);
    assert!(!out.status.success(), "expected clap failure: {out:?}");
    let stderr = str::from_utf8(&out.stderr).unwrap();
    assert!(
        stderr.contains("--json"),
        "clap requires error must name --json: {stderr}"
    );
}
