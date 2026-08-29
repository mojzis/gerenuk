//! End-to-end tests for `gerenuk impacted-tests`.
//!
//! Real `git` (via `common::TestRepo`) and a stubbed `tyf`, so the walk runs
//! against a genuine working tree while the reference graph stays pinned. The
//! degradation tests are the load-bearing ones: a pre-commit hook that fails
//! open is worse than no hook at all, so every way this can go wrong has to
//! come back as `run_all` and exit `0`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests should abort loudly on a failed assumption"
)]

mod common;

use std::path::Path;

use common::{
    crippled_git, failing_tyf, fake_tyf, garbled_tyf, gerenuk, gerenuk_no_tyf, json_output,
    TestRepo,
};
use serde_json::Value;
use tempfile::TempDir;

const CORE: &str = r#""""The changed module."""


def target(value):
    return value + 1
"#;

const SERVICE: &str = r#""""One hop above core."""

from mypkg.core import target


def middle(value):
    return target(value) * 2
"#;

const JOBS: &str = r#""""A registry function: a dead end for the walk."""

from mypkg import registry
from mypkg.core import target


@registry.transformation
def nightly(value):
    return target(value)
"#;

const SETTINGS: &str = r#""""Calls `target` at import time."""

from mypkg.core import target

VALUE = target(1)
"#;

/// A repository whose reference graph covers every rule the walk has:
/// a direct test edge, a two-hop edge, an import line, a module-level call and
/// a registry dead end.
fn graph_repo() -> TestRepo {
    let repo = TestRepo::new();
    repo.write(
        "pyproject.toml",
        "[project]\nname = \"mypkg\"\n\n[tool.gerenuk]\nignore-decorators = [\"registry.transformation\"]\n",
    );
    repo.write("src/mypkg/__init__.py", "");
    repo.write("src/mypkg/registry.py", "def transformation(func):\n    return func\n");
    repo.write("src/mypkg/core.py", CORE);
    repo.write("src/mypkg/service.py", SERVICE);
    repo.write("src/mypkg/jobs.py", JOBS);
    repo.write("src/mypkg/settings.py", SETTINGS);
    repo.write(
        "tests/test_core.py",
        "from mypkg.core import target\n\n\ndef test_target():\n    assert target(1) == 2\n",
    );
    repo.write(
        "tests/test_service.py",
        "from mypkg.service import middle\n\n\ndef test_middle():\n    assert middle(1) == 4\n",
    );
    repo.write(
        "tests/test_settings.py",
        "from mypkg import settings\n\n\ndef test_value():\n    assert settings.VALUE == 2\n",
    );
    repo.commit("base");
    repo
}

/// Change `target`'s body, and nothing else.
fn touch_target(repo: &TestRepo) {
    repo.write("src/mypkg/core.py", &CORE.replace("return value + 1", "return value + 2"));
}

/// Reference payloads shaped like real `tyf refs` output for [`graph_repo`].
///
/// Keyed by the `file:line:col` position gerenuk queries with, not by name:
/// `target` is on line 4 of `core.py` and `middle` on line 6 of `service.py`,
/// both at column 5 (`def ` plus one).
///
/// `target`'s own definition is included, as `tyf` includes it, and so is the
/// import line in `service.py` — gerenuk has to drop both itself.
fn refs_fixtures() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "src/mypkg/core.py:4:5",
            r#"{"symbol": "target", "reference_count": 5, "references": [
                  {"file": "src/mypkg/core.py", "line": 4, "column": 5, "context": "target"},
                  {"file": "src/mypkg/service.py", "line": 3, "column": 25, "context": "module scope"},
                  {"file": "src/mypkg/service.py", "line": 7, "column": 12, "context": "middle"},
                  {"file": "src/mypkg/jobs.py", "line": 9, "column": 12, "context": "nightly"},
                  {"file": "src/mypkg/settings.py", "line": 5, "column": 9, "context": "module scope"}
                ], "test_reference_count": 1, "test_references": [
                  {"file": "tests/test_core.py", "line": 5, "column": 12, "context": "test_target"}
                ]}"#,
        ),
        (
            "src/mypkg/service.py:6:5",
            r#"{"symbol": "middle", "reference_count": 0, "references": [],
                "test_reference_count": 1, "test_references": [
                  {"file": "tests/test_service.py", "line": 5, "column": 12, "context": "test_middle"}
                ]}"#,
        ),
    ]
}

/// Run `impacted-tests --format json` against a stubbed `tyf`.
fn impacted(repo: &TestRepo, tyf: &Path, extra: &[&str]) -> Value {
    json_output(gerenuk(repo.path(), tyf).args(["--format", "json", "impacted-tests"]).args(extra))
}

/// The `(file, symbol, via, origin)` of every selected test.
fn selected(report: &Value) -> Vec<(String, Option<String>, Vec<String>, String)> {
    report["impacted_tests"]
        .as_array()
        .unwrap_or_else(|| panic!("`impacted_tests` should be an array, got {report}"))
        .iter()
        .map(|test| {
            (
                test["file"].as_str().unwrap_or_default().to_string(),
                test["symbol"].as_str().map(ToString::to_string),
                test["via"]
                    .as_array()
                    .map(|via| {
                        via.iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect()
                    })
                    .unwrap_or_default(),
                test["origin"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

#[test]
fn every_edge_the_walk_knows_about_is_exercised_at_once() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    let report = impacted(&repo, &tyf, &[]);

    assert_eq!(report["verdict"], "selected", "the frontier emptied: {report}");
    assert!(report["reason"].is_null(), "a completed walk has no reason");
    assert_eq!(
        selected(&report),
        vec![
            (
                "tests/test_core.py".to_string(),
                Some("tests.test_core:test_target".to_string()),
                vec![],
                "mypkg.core:target".to_string(),
            ),
            (
                "tests/test_service.py".to_string(),
                Some("tests.test_service:test_middle".to_string()),
                vec!["mypkg.service:middle".to_string()],
                "mypkg.core:target".to_string(),
            ),
            (
                "tests/test_settings.py".to_string(),
                None,
                vec!["mypkg.settings".to_string()],
                "mypkg.core:target".to_string(),
            ),
        ],
        "a direct edge, a two-hop edge and a module-level edge, in that order"
    );
}

#[test]
fn a_registry_function_is_reported_rather_than_walked() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    let report = impacted(&repo, &tyf, &[]);
    let ignored = report["ignored_symbols"].as_array().expect("an array");

    assert_eq!(ignored.len(), 1, "exactly the one decorated symbol, got {report}");
    assert_eq!(ignored[0]["symbol"], "mypkg.jobs:nightly");
    assert_eq!(
        ignored[0]["ignored_by"], "registry.transformation",
        "the report says which config entry matched"
    );
    assert!(
        !report.to_string().contains("test_jobs"),
        "and nothing downstream of it is selected: {report}"
    );
}

#[test]
fn the_import_line_of_the_changed_symbol_selects_nothing_on_its_own() {
    // `service.py` imports `target` on line 3 and uses it on line 7. Only the
    // second is an edge; the first would otherwise pull in every importer.
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    let report = impacted(&repo, &tyf, &[]);
    assert_eq!(
        report["stats"]["visited"], 3,
        "target, middle and the settings module — the import line adds nothing: {report}"
    );
    assert_eq!(report["stats"]["tyf_calls"], 2, "one per BFS level, not per symbol: {report}");
    assert_eq!(report["stats"]["max_depth_reached"], 1, "two levels: {report}");
}

#[test]
fn an_unchanged_tree_selects_nothing_and_still_succeeds() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();

    let report = impacted(&repo, &tyf, &[]);
    assert_eq!(report["verdict"], "selected", "no change is a complete answer: {report}");
    assert!(selected(&report).is_empty(), "and it selects nothing");
    assert_eq!(report["stats"]["seeds"], 0);
}

#[test]
fn a_deleted_symbol_is_chased_through_a_textual_scan() {
    // `ty` cannot resolve references to a name with no definition left, so this
    // is the one edge the walk finds by reading the source rather than by
    // asking `tyf`. It over-matches on purpose.
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    repo.write("src/mypkg/core.py", "\"\"\"The changed module.\"\"\"\n");

    let report = impacted(&repo, &tyf, &[]);

    assert_eq!(
        selected(&report).into_iter().map(|(file, _, _, _)| file).collect::<Vec<_>>(),
        vec!["tests/test_core.py", "tests/test_service.py", "tests/test_settings.py"],
        "the orphaned call sites are found by name, and the walk continues from them: {report}"
    );
    assert_eq!(
        report["impacted_tests"][1]["via"][0], "mypkg.service:middle",
        "and the chain is the same as the resolved one would have been: {report}"
    );
    assert_eq!(
        report["stats"]["tyf_calls"], 1,
        "the deleted seed itself is never sent to tyf: {report}"
    );
}

#[test]
fn a_changed_test_file_is_passed_through_from_phase_one() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    repo.write(
        "tests/test_core.py",
        "from mypkg.core import target\n\n\ndef test_target():\n    assert target(1) == 2, \"sum\"\n",
    );

    let report = impacted(&repo, &tyf, &[]);
    assert_eq!(
        report["test_files_changed"][0], "tests/test_core.py",
        "a changed test selects itself, with no walking needed: {report}"
    );
}

#[test]
fn a_non_python_change_bails_out_before_tyf_is_ever_looked_for() {
    // The whole point of checking this verdict first: no `tyf`, no `ty`, no
    // Python, and the command still answers.
    let repo = graph_repo();
    repo.write("requirements.txt", "requests==2.0\n");

    let report =
        json_output(gerenuk_no_tyf(repo.path()).args(["--format", "json", "impacted-tests"]));

    assert_eq!(report["verdict"], "run_all", "a dependency bump can change anything");
    assert_eq!(report["reason"], "non_python_changes");
}

#[test]
fn a_file_phase_one_could_not_parse_bails_out_too() {
    let repo = graph_repo();
    repo.write("src/mypkg/core.py", "def broken(:\n    pass\n");

    let report =
        json_output(gerenuk_no_tyf(repo.path()).args(["--format", "json", "impacted-tests"]));

    assert_eq!(report["reason"], "parse_errors", "an unparseable file cannot be reasoned about");
}

#[test]
fn a_missing_tyf_degrades_to_run_all_rather_than_failing() {
    let repo = graph_repo();
    touch_target(&repo);

    let report =
        json_output(gerenuk_no_tyf(repo.path()).args(["--format", "json", "impacted-tests"]));

    assert_eq!(report["verdict"], "run_all", "exit 0 with a verdict, not exit 2");
    assert_eq!(report["reason"], "tyf_unavailable");
    assert!(
        report["errors"][0].as_str().is_some_and(|e| e.contains("tyf")),
        "the reason must name what is missing: {report}"
    );
}

#[test]
fn a_failing_tyf_degrades_to_run_all() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = failing_tyf(&tmp, "ty server did not start");
    let repo = graph_repo();
    touch_target(&repo);

    let report = impacted(&repo, &tyf, &[]);

    assert_eq!(report["reason"], "refs_failed", "a mid-walk failure is still an answer");
    assert!(
        report["errors"][0].as_str().is_some_and(|e| e.contains("ty server did not start")),
        "and tyf's own message survives: {report}"
    );
}

#[test]
fn output_that_is_not_json_degrades_to_run_all() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = garbled_tyf(&tmp);
    let repo = graph_repo();
    touch_target(&repo);

    let report = impacted(&repo, &tyf, &[]);
    assert_eq!(report["reason"], "refs_failed", "unparseable output is a failure like any other");
}

#[test]
fn a_working_tree_that_cannot_be_listed_degrades_to_run_all() {
    // `git ls-files` runs *after* the up-front gate and after tyf is found, so
    // a failure there has to become a verdict like every other late failure.
    // `--changed` is what makes it the only git call left in the run.
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    let saved = repo.changed_symbols(&[]);
    let path = tmp.path().join("changed.json");
    std::fs::write(&path, saved.to_string()).expect("write the saved report");

    let report = json_output(
        gerenuk(repo.path(), &tyf)
            .env("GERENUK_GIT", crippled_git(&tmp))
            .args(["--format", "json", "impacted-tests"])
            .args(["--changed", &path.display().to_string()]),
    );

    assert_eq!(report["verdict"], "run_all", "exit 0 with a verdict, not exit 2: {report}");
    assert_eq!(report["reason"], "index_failed");
    assert!(
        report["errors"][0].as_str().is_some_and(|e| e.contains("smudged")),
        "and git's own message survives: {report}"
    );
}

#[test]
fn the_depth_limit_can_be_set_from_the_command_line() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    let report = impacted(&repo, &tyf, &["--max-depth", "0"]);
    assert_eq!(report["verdict"], "run_all", "the walk did not finish: {report}");
    assert_eq!(report["reason"], "max_depth");
}

#[test]
fn the_symbol_limit_can_be_set_from_the_command_line() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    let report = impacted(&repo, &tyf, &["--max-symbols", "1"]);
    assert_eq!(report["reason"], "max_symbols");
}

#[test]
fn a_saved_phase_one_report_can_be_replayed() {
    // The phase-1 JSON is the interface between the phases; this is the test
    // that keeps it one.
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    let saved = repo.changed_symbols(&[]);
    let path = tmp.path().join("changed.json");
    std::fs::write(&path, saved.to_string()).expect("write the saved report");

    let replayed = impacted(&repo, &tyf, &["--changed", &path.display().to_string()]);
    let live = impacted(&repo, &tyf, &[]);

    assert_eq!(
        selected(&replayed),
        selected(&live),
        "replaying a saved report must reach the same tests"
    );
}

#[test]
fn a_missing_changed_file_is_an_operational_failure() {
    use assert_cmd::prelude::*;

    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &[]);
    let repo = graph_repo();

    gerenuk(repo.path(), &tyf)
        .args(["impacted-tests", "--changed", "/definitely/not/here.json"])
        .assert()
        // Exit 2, not a run_all verdict: there is no report to answer about.
        .code(2)
        .stderr(predicates::str::contains("not/here.json"));
}

#[test]
fn a_changed_file_that_is_not_a_valid_report_is_an_operational_failure() {
    use assert_cmd::prelude::*;

    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &[]);
    let repo = graph_repo();
    touch_target(&repo);

    // `{}` is valid JSON but not a report. It used to deserialise into an empty
    // one and walk to a confident `selected` verdict selecting nothing, which
    // is the one failure mode this command must never have.
    let path = tmp.path().join("empty.json");
    std::fs::write(&path, "{}").expect("write the file");

    gerenuk(repo.path(), &tyf)
        .args(["impacted-tests", "--changed", &path.display().to_string()])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("empty.json"))
        .stderr(predicates::str::contains("base"));
}

#[test]
fn a_report_from_a_different_gerenuk_is_rejected_rather_than_half_read() {
    use assert_cmd::prelude::*;

    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &[]);
    let repo = graph_repo();

    let path = tmp.path().join("stale.json");
    std::fs::write(&path, r#"{"base": "main", "merge_base": "abc", "changed_symbol": []}"#)
        .expect("write the file");

    gerenuk(repo.path(), &tyf)
        .args(["impacted-tests", "--changed", &path.display().to_string()])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("changed_symbol"));
}

#[test]
fn an_expired_wall_clock_budget_is_honoured_from_the_command_line() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    // 1 ms is gone by the time the first frontier is answered.
    let report = impacted(&repo, &tyf, &["--budget-ms", "1"]);
    assert_eq!(report["reason"], "budget", "got {report}");

    let unlimited = impacted(&repo, &tyf, &["--budget-ms", "0"]);
    assert_eq!(unlimited["verdict"], "selected", "0 disables the clock: {unlimited}");
}

#[test]
fn human_output_shows_the_chain_that_reached_each_test() {
    use assert_cmd::prelude::*;

    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    gerenuk(repo.path(), &tyf)
        .arg("impacted-tests")
        .assert()
        .success()
        .stdout(predicates::str::contains("verdict selected"))
        .stdout(predicates::str::contains("tests/test_service.py::test_middle"))
        .stdout(predicates::str::contains("← mypkg.service:middle ← mypkg.core:target"))
        .stdout(predicates::str::contains("tests/test_settings.py  (whole file)"));
}

#[test]
fn the_full_json_report_matches_its_snapshot() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
    let repo = graph_repo();
    touch_target(&repo);

    let mut report = impacted(&repo, &tyf, &[]);
    // The merge base is a fresh sha and the walk takes however long it takes.
    report["merge_base"] = Value::from("<sha>");
    report["stats"]["duration_ms"] = Value::from(0);

    insta::assert_snapshot!(
        serde_json::to_string_pretty(&report).expect("the report re-serialises")
    );
}
