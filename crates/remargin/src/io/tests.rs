//! Unit tests for [`crate::io`].

use os_shim::mock::MockSystem;

use super::{expand_cli_path, parse_line_range, truncate_content};

#[test]
fn parse_line_range_accepts_simple_pair() {
    let (s, e) = parse_line_range("10-20").unwrap();
    assert_eq!((s, e), (10, 20));
}

#[test]
fn parse_line_range_accepts_single_line_range() {
    let (s, e) = parse_line_range("7-7").unwrap();
    assert_eq!((s, e), (7, 7));
}

#[test]
fn parse_line_range_rejects_missing_dash() {
    let err = parse_line_range("100").unwrap_err();
    assert!(err.to_string().contains("START-END"));
}

#[test]
fn parse_line_range_rejects_non_numeric() {
    let err = parse_line_range("a-b").unwrap_err();
    assert!(err.to_string().contains("invalid start value"));
}

#[test]
fn parse_line_range_rejects_non_numeric_end() {
    let err = parse_line_range("1-b").unwrap_err();
    assert!(err.to_string().contains("invalid end value"));
}

#[test]
fn truncate_content_short_content() {
    assert_eq!(truncate_content("hello", 10), "hello");
}

#[test]
fn truncate_content_exact_limit() {
    assert_eq!(truncate_content("hello", 5), "hello");
}

#[test]
fn truncate_content_over_limit() {
    assert_eq!(truncate_content("hello world", 5), "hello...");
}

#[test]
fn truncate_content_first_line_only() {
    assert_eq!(truncate_content("line1\nline2", 10), "line1");
}

#[test]
fn truncate_content_empty() {
    assert_eq!(truncate_content("", 10), "");
}

#[test]
fn expand_cli_path_home_expansion() {
    let system = MockSystem::new().with_env("HOME", "/home/test").unwrap();
    let result = expand_cli_path(&system, "~/docs/file.md").unwrap();
    assert_eq!(result.to_string_lossy(), "/home/test/docs/file.md");
}
