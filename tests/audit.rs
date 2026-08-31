//! End-to-end audit runs against the checked-in Python fixture package.
//!
//! `tyf` is stubbed (see `common::fake_tyf`) so these tests are hermetic: they
//! need neither `ty` nor a Python environment. The stub's payloads mirror what
//! real `tyf` returns for `tests/fixtures/sample_pkg/sample_pkg/service.py`.

mod common;

use assert_cmd::prelude::*;
use predicates::str::contains;
use tempfile::TempDir;

use common::{fake_tyf, gerenuk, sample_pkg, SERVICE_OUTLINE};

/// Reference payloads shaped like real `tyf` output for `sample_pkg/service.py`.
///
/// Two quirks are reproduced on purpose, because gerenuk has to survive both:
///
/// * `tyf` files **every** reference under `test_references` here — its test
///   heuristic looks at the whole absolute path, and the fixture package lives
///   under `tests/fixtures/`. gerenuk must re-derive the buckets from paths
///   relative to the workspace root.
/// * `tyf` includes the symbol's own definition among the references. gerenuk
///   must not count a definition as a usage — otherwise nothing is ever unused.
///
/// Reference states: `describe` is used from `cli.py`, `pipelines.py` and the
/// `described` fixture in `tests/conftest.py`; `summary` from `cli.py`;
/// `seniors` only from `tests/test_service.py`; `legacy_export` nowhere.
fn refs_fixtures() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "describe",
            r#"{"symbol": "describe", "reference_count": 0, "references": [],
                "test_reference_count": 8, "test_references": [
                  {"file": "sample_pkg/service.py", "line": 16, "column": 5, "context": "describe"},
                  {"file": "sample_pkg/cli.py", "line": 6, "column": 48, "context": "module scope"},
                  {"file": "sample_pkg/cli.py", "line": 24, "column": 15, "context": "main"},
                  {"file": "sample_pkg/pipelines.py", "line": 12, "column": 32, "context": "module scope"},
                  {"file": "sample_pkg/pipelines.py", "line": 23, "column": 17, "context": "Enricher.run"},
                  {"file": "tests/conftest.py", "line": 27, "column": 32, "context": "module scope"},
                  {"file": "tests/conftest.py", "line": 43, "column": 13, "context": "described"},
                  {"file": "tests/test_service.py", "line": 13, "column": 12, "context": "test_describe_marks_seniors"}
                ]}"#,
        ),
        (
            "ShelterService.summary",
            r#"{"symbol": "ShelterService.summary", "reference_count": 0, "references": [],
                "test_reference_count": 3, "test_references": [
                  {"file": "sample_pkg/service.py", "line": 28, "column": 9, "context": "summary"},
                  {"file": "sample_pkg/cli.py", "line": 23, "column": 19, "context": "main"},
                  {"file": "tests/test_service.py", "line": 21, "column": 12, "context": "test_summary_counts_animals_and_species"}
                ]}"#,
        ),
        (
            "ShelterService.seniors",
            r#"{"symbol": "ShelterService.seniors", "reference_count": 0, "references": [],
                "test_reference_count": 2, "test_references": [
                  {"file": "sample_pkg/service.py", "line": 34, "column": 9, "context": "seniors"},
                  {"file": "tests/test_service.py", "line": 30, "column": 25, "context": "test_seniors_returns_only_old_animals"}
                ]}"#,
        ),
        (
            "legacy_export",
            r#"{"symbol": "legacy_export", "reference_count": 0, "references": [],
                "test_reference_count": 1, "test_references": [
                  {"file": "sample_pkg/service.py", "line": 43, "column": 5, "context": "legacy_export"}
                ]}"#,
        ),
    ]
}

/// A truncated answer: counts are reported but the lists are withheld, the
/// shape `tyf` returns without `--tests` or under `--references-limit`.
const TRUNCATED_REFS: &str = r#"{"symbol": "describe", "reference_count": 0, "references": [],
    "test_reference_count": 7, "test_references": []}"#;

#[test]
fn audit_flags_the_unused_symbol_and_the_test_only_one() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, SERVICE_OUTLINE, &refs_fixtures());

    let assert = gerenuk(&sample_pkg(), &tyf)
        .args(["audit", "sample_pkg/service.py"])
        .assert()
        // Findings were reported, so the exit code is 1.
        .code(1);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");

    assert!(
        stdout.contains("legacy_export") && stdout.contains("no references"),
        "the unreferenced function should be warned about, got:\n{stdout}"
    );
    assert!(
        stdout.contains("ShelterService.seniors") && stdout.contains("only from tests"),
        "the test-only method should be noted, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("describe,") && !stdout.contains("`describe`"),
        "a symbol used from cli.py must not be flagged, got:\n{stdout}"
    );
    assert!(!stdout.contains("_sorted_by_age"), "private helpers are skipped, got:\n{stdout}");
    assert!(!stdout.contains("__init__"), "dunder methods are skipped, got:\n{stdout}");
}

#[test]
fn audit_reports_locations_relative_to_the_workspace() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, SERVICE_OUTLINE, &refs_fixtures());

    gerenuk(&sample_pkg(), &tyf)
        .args(["audit", "sample_pkg/service.py"])
        .assert()
        .code(1)
        // legacy_export's selectionRange starts on zero-based line 42.
        .stdout(contains("sample_pkg/service.py:43"));
}

#[test]
fn json_output_is_machine_readable() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, SERVICE_OUTLINE, &refs_fixtures());

    let assert = gerenuk(&sample_pkg(), &tyf)
        .args(["--format", "json", "audit", "sample_pkg/service.py"])
        .assert()
        .code(1);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("--format json must emit valid JSON ({e}), got:\n{stdout}"));

    let findings = value["findings"].as_array().expect("findings should be an array");
    assert_eq!(findings.len(), 2, "exactly two symbols should be flagged, got:\n{stdout}");

    let symbols: Vec<&str> = findings.iter().filter_map(|f| f["symbol"].as_str()).collect();
    assert!(symbols.contains(&"legacy_export"), "unused symbol missing from JSON: {symbols:?}");
    assert!(
        symbols.contains(&"ShelterService.seniors"),
        "test-only symbol missing from JSON: {symbols:?}"
    );
    assert_eq!(
        value["files"][0], "sample_pkg/service.py",
        "audited file should be listed relatively"
    );
}

#[test]
fn a_file_where_everything_is_used_exits_clean() {
    let tmp = TempDir::new().expect("temp dir");
    // Only `describe` in the outline, and it has production references.
    let outline = r#"[
      {"name": "describe", "kind": 12,
       "range": {"start": {"line": 15, "character": 0}, "end": {"line": 18, "character": 48}},
       "selectionRange": {"start": {"line": 15, "character": 4}, "end": {"line": 15, "character": 12}},
       "children": []}
    ]"#;
    let tyf = fake_tyf(&tmp, outline, &refs_fixtures());

    gerenuk(&sample_pkg(), &tyf)
        .args(["audit", "sample_pkg/service.py"])
        .assert()
        .success()
        .stdout(contains("No findings."))
        .stdout(contains("0 warn, 0 note"));
}

#[test]
fn an_empty_file_is_audited_without_complaint() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &[]);

    gerenuk(&sample_pkg(), &tyf)
        .args(["audit", "sample_pkg/__init__.py"])
        .assert()
        .success()
        .stdout(contains("1 file(s), 0 symbol(s) checked"));
}

#[test]
fn a_definition_is_not_counted_as_a_usage() {
    // `legacy_export`'s only "reference" is its own definition line, so it must
    // still be reported as having no references at all.
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, SERVICE_OUTLINE, &refs_fixtures());

    gerenuk(&sample_pkg(), &tyf)
        .args(["audit", "sample_pkg/service.py"])
        .assert()
        .code(1)
        .stdout(contains("`legacy_export` has no references"));
}

#[test]
fn withheld_reference_lists_fall_back_to_tyf_counts() {
    let tmp = TempDir::new().expect("temp dir");
    let outline = r#"[
      {"name": "describe", "kind": 12,
       "range": {"start": {"line": 15, "character": 0}, "end": {"line": 18, "character": 48}},
       "selectionRange": {"start": {"line": 15, "character": 4}, "end": {"line": 15, "character": 12}},
       "children": []}
    ]"#;
    let tyf = fake_tyf(&tmp, outline, &[("describe", TRUNCATED_REFS)]);

    gerenuk(&sample_pkg(), &tyf)
        .args(["audit", "sample_pkg/service.py"])
        .assert()
        .code(1)
        // With no list to inspect, the reported test count is all we have.
        .stdout(contains("referenced only from tests (7)"));
}
