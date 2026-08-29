//! End-to-end tests for `gerenuk changed-symbols` against real git repositories.
//!
//! None of these set `GERENUK_TYF`: the whole point of phase 1 is that mapping a
//! diff to symbols needs `git` and nothing else.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests should abort loudly on a failed assumption"
)]

mod common;

use common::{gerenuk_bare, TestRepo};
use serde_json::Value;

/// A src-layout package with a handful of symbols in known positions.
const SERVICE: &str = r#""""Service module."""

import os

LIMIT = 10


def describe(name):
    return f"{name}{os.sep}"


def legacy():
    return None


class Shelter:
    capacity: int = 0

    def summary(self):
        return LIMIT

    def seniors(self):
        return []
"#;

/// A repository with `src/mypkg/service.py` committed on `main`.
fn src_layout_repo() -> TestRepo {
    let repo = TestRepo::new();
    repo.write("pyproject.toml", "[project]\nname = \"mypkg\"\n");
    repo.write("src/mypkg/__init__.py", "");
    repo.write("src/mypkg/service.py", SERVICE);
    repo.write("tests/test_service.py", "def test_describe():\n    assert True\n");
    repo.commit("base");
    repo
}

/// `(symbol, change)` pairs from one of the report's symbol arrays.
fn pairs(report: &Value, key: &str) -> Vec<(String, String)> {
    report[key]
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` should be an array, got {report}"))
        .iter()
        .map(|entry| {
            (
                entry["symbol"].as_str().unwrap_or_default().to_string(),
                entry["change"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn strings(report: &Value, key: &str) -> Vec<String> {
    report[key]
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` should be an array, got {report}"))
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn an_unchanged_tree_reports_empty_arrays_and_exits_zero() {
    let repo = src_layout_repo();
    let report = repo.changed_symbols(&[]);

    assert_eq!(report["base"], "main", "the default chain reaches `main`");
    assert!(pairs(&report, "changed_symbols").is_empty(), "nothing changed");
    assert!(strings(&report, "non_python_changes").is_empty(), "not even a config file");
    assert!(strings(&report, "errors").is_empty(), "and no errors");
}

#[test]
fn editing_a_function_body_reports_that_function_as_modified() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", &SERVICE.replace("return LIMIT", "return LIMIT * 2"));

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.service:Shelter.summary".to_string(), "modified".to_string())],
        "only the method whose body moved"
    );
    assert_eq!(report["changed_symbols"][0]["kind"], "method", "kind comes from the parse");
    assert_eq!(
        report["changed_symbols"][0]["file"], "src/mypkg/service.py",
        "paths are repository-relative"
    );
}

#[test]
fn adding_a_function_reports_it_as_added() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", &format!("{SERVICE}\n\ndef fresh():\n    return 1\n"));

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.service:fresh".to_string(), "added".to_string())],
        "the untouched symbols stay out of the report"
    );
}

#[test]
fn deleting_a_function_reports_it_as_deleted() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", &SERVICE.replace("def legacy():\n    return None\n", ""));

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.service:legacy".to_string(), "deleted".to_string())],
        "phase 2 has to chase the callers of a deleted symbol, so it must be reported"
    );
}

#[test]
fn deleting_a_whole_file_deletes_every_symbol_in_it() {
    let repo = src_layout_repo();
    repo.remove("src/mypkg/service.py");

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![
            ("mypkg.service:Shelter".to_string(), "deleted".to_string()),
            ("mypkg.service:Shelter.seniors".to_string(), "deleted".to_string()),
            ("mypkg.service:Shelter.summary".to_string(), "deleted".to_string()),
            ("mypkg.service:describe".to_string(), "deleted".to_string()),
            ("mypkg.service:legacy".to_string(), "deleted".to_string()),
        ],
        "the old blob is parsed so the departed symbols can still be named"
    );
    assert_eq!(
        strings(&report, "module_level_changes"),
        vec!["mypkg.service".to_string()],
        "its module-level code went with it"
    );
}

#[test]
fn renaming_a_file_produces_a_deleted_and_an_added_set() {
    let repo = src_layout_repo();
    repo.git(&["mv", "src/mypkg/service.py", "src/mypkg/shelter.py"]);

    let report = repo.changed_symbols(&[]);
    let symbols = pairs(&report, "changed_symbols");

    assert!(
        symbols.contains(&("mypkg.service:describe".to_string(), "deleted".to_string())),
        "the old module's symbols are gone: {symbols:?}"
    );
    assert!(
        symbols.contains(&("mypkg.shelter:describe".to_string(), "added".to_string())),
        "and the new module's are new: {symbols:?}"
    );
    assert_eq!(symbols.len(), 10, "five symbols on each side, unpaired: {symbols:?}");
}

#[test]
fn a_module_level_edit_reports_the_module_rather_than_a_symbol() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", &SERVICE.replace("LIMIT = 10", "LIMIT = 25"));

    let report = repo.changed_symbols(&[]);
    assert!(pairs(&report, "changed_symbols").is_empty(), "a constant belongs to no symbol");
    assert_eq!(
        strings(&report, "module_level_changes"),
        vec!["mypkg.service".to_string()],
        "phase 2 treats this as `the whole module changed`"
    );
}

#[test]
fn a_changed_test_file_is_listed_but_not_analysed() {
    let repo = src_layout_repo();
    repo.write("tests/test_service.py", "def test_describe():\n    assert 1 == 1\n");

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        strings(&report, "test_files_changed"),
        vec!["tests/test_service.py".to_string()],
        "a changed test simply selects itself later"
    );
    assert!(pairs(&report, "changed_symbols").is_empty(), "no symbol analysis for tests");
}

#[test]
fn a_non_python_change_is_partitioned_off() {
    let repo = src_layout_repo();
    repo.write("data/schema.sql", "CREATE TABLE t (id INT);\n");
    repo.write("pyproject.toml", "[project]\nname = \"mypkg\"\nversion = \"0.2.0\"\n");

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        strings(&report, "non_python_changes"),
        vec!["data/schema.sql".to_string(), "pyproject.toml".to_string()],
        "these later trigger a full-run bail-out"
    );
}

#[test]
fn a_syntax_error_is_reported_instead_of_crashing() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", "def broken(:\n    pass\n");

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        strings(&report, "errors"),
        vec!["src/mypkg/service.py".to_string()],
        "the file is named so the user can see why it degraded"
    );
    assert_eq!(
        strings(&report, "module_level_changes"),
        vec!["mypkg.service".to_string()],
        "an unparseable file is treated as a whole-module change"
    );
}

#[test]
fn a_flat_layout_package_resolves_the_same_way() {
    let repo = TestRepo::new();
    repo.write("pyproject.toml", "[project]\nname = \"flat\"\n");
    repo.write("flat/__init__.py", "");
    repo.write("flat/utils.py", "def parse_ts(x):\n    return x\n");
    repo.commit("base");

    repo.write("flat/utils.py", "def parse_ts(x):\n    return int(x)\n");

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("flat.utils:parse_ts".to_string(), "modified".to_string())],
        "no `src` to strip, but the __init__.py chain gives the same answer"
    );
}

#[test]
fn an_untracked_file_is_reported_as_wholly_added() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/fresh.py", "def brand_new():\n    return 1\n");

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.fresh:brand_new".to_string(), "added".to_string())],
        "git diff cannot see untracked files, so gerenuk asks ls-files as well"
    );
}

#[test]
fn an_ignored_untracked_file_stays_invisible() {
    let repo = src_layout_repo();
    repo.write(".gitignore", "build/\n");
    repo.commit("ignore build output");
    repo.write("build/generated.py", "def noise():\n    return 1\n");

    let report = repo.changed_symbols(&[]);
    assert!(
        pairs(&report, "changed_symbols").is_empty(),
        "generated code the repo ignores must not select tests: {report}"
    );
}

#[test]
fn staged_and_unstaged_edits_are_reported_together() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", &SERVICE.replace("return LIMIT", "return LIMIT * 2"));
    repo.git(&["add", "src/mypkg/service.py"]);
    repo.write("src/mypkg/other.py", "def unstaged():\n    return 1\n");
    repo.git(&["add", "src/mypkg/other.py"]);
    repo.write("src/mypkg/other.py", "def unstaged():\n    return 2\n");

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![
            ("mypkg.other:unstaged".to_string(), "added".to_string()),
            ("mypkg.service:Shelter.summary".to_string(), "modified".to_string()),
        ],
        "phase 1 takes the whole working tree, staged or not"
    );
}

#[test]
fn the_merge_base_is_used_so_the_base_branch_moving_on_is_invisible() {
    let repo = src_layout_repo();
    repo.git(&["checkout", "-b", "feature"]);
    repo.write("src/mypkg/service.py", &SERVICE.replace("return LIMIT", "return LIMIT * 2"));
    repo.commit("feature work");

    repo.git(&["checkout", "main"]);
    repo.write("src/mypkg/unrelated.py", "def elsewhere():\n    return 1\n");
    repo.commit("main moves on");
    repo.git(&["checkout", "feature"]);

    let report = repo.changed_symbols(&["--base", "main"]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.service:Shelter.summary".to_string(), "modified".to_string())],
        "work done on main after the fork point is not ours to run tests for"
    );
}

#[test]
fn ignore_decorators_moves_registry_functions_out_of_the_way() {
    let repo = TestRepo::new();
    repo.write(
        "pyproject.toml",
        "[project]\nname = \"mypkg\"\n\n\
         [tool.gerenuk]\nignore-decorators = [\"transformation\"]\n",
    );
    repo.write("src/mypkg/__init__.py", "");
    repo.write(
        "src/mypkg/daily.py",
        "import registry\n\n\n\
         @registry.transformation(name=\"prices\")\n\
         def normalize_prices(df):\n    return df\n\n\n\
         def helper(df):\n    return df\n",
    );
    repo.commit("base");

    repo.write(
        "src/mypkg/daily.py",
        "import registry\n\n\n\
         @registry.transformation(name=\"prices\")\n\
         def normalize_prices(df):\n    return df.copy()\n\n\n\
         def helper(df):\n    return df.copy()\n",
    );

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.daily:helper".to_string(), "modified".to_string())],
        "the registry function is not worth closure work"
    );
    assert_eq!(
        pairs(&report, "ignored_symbols"),
        vec![("mypkg.daily:normalize_prices".to_string(), "modified".to_string())],
        "but it is reported, not silently dropped"
    );
    assert_eq!(
        report["ignored_symbols"][0]["ignored_by"], "transformation",
        "the report says which config entry matched"
    );
}

#[test]
fn without_the_config_the_same_decorator_changes_nothing() {
    let repo = TestRepo::new();
    repo.write("pyproject.toml", "[project]\nname = \"mypkg\"\n");
    repo.write("src/mypkg/__init__.py", "");
    repo.write("src/mypkg/daily.py", "@transformation\ndef normalize(df):\n    return df\n");
    repo.commit("base");
    repo.write("src/mypkg/daily.py", "@transformation\ndef normalize(df):\n    return df.copy()\n");

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.daily:normalize".to_string(), "modified".to_string())],
        "ignoring is opt-in"
    );
    assert!(pairs(&report, "ignored_symbols").is_empty(), "nothing is ignored by default");
}

#[test]
fn an_external_diff_driver_cannot_blank_the_report() {
    let repo = src_layout_repo();
    // difftastic's own README recommends this setting. It replaces git's diff
    // generator wholesale, and without `--no-ext-diff` every run would report
    // nothing changed — with exit code 0, so nobody would notice.
    repo.git(&["config", "diff.external", "/bin/true"]);
    repo.write("src/mypkg/service.py", &SERVICE.replace("return LIMIT", "return LIMIT * 2"));

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.service:Shelter.summary".to_string(), "modified".to_string())],
        "a configured external differ must not silently empty the diff"
    );
}

#[test]
fn a_textconv_filter_cannot_shift_the_line_numbers() {
    let repo = src_layout_repo();
    // A textconv filter rewrites the blob before diffing, so hunk line numbers
    // would refer to the filtered text rather than to the real file.
    // Dropping blank lines shifts every subsequent line number, so a hunk
    // reported against the filtered text lands on the wrong symbol.
    repo.write(".gitattributes", "*.py diff=stripblanks\n");
    repo.git(&["config", "diff.stripblanks.textconv", "grep -v '^$'"]);
    repo.commit("add a textconv filter");
    repo.write("src/mypkg/service.py", &SERVICE.replace("return LIMIT", "return LIMIT * 2"));

    let report = repo.changed_symbols(&[]);
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.service:Shelter.summary".to_string(), "modified".to_string())],
        "line numbers must come from the real file, not from a filtered view of it"
    );
}

#[test]
fn a_missing_base_ref_exits_with_code_two() {
    let repo = src_layout_repo();
    let output = gerenuk_bare(repo.path())
        .args(["changed-symbols", "--base", "origin/nope"])
        .output()
        .expect("gerenuk should run");

    assert_eq!(output.status.code(), Some(2), "an unusable base is an operational failure");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("origin/nope"), "the error names the ref that was asked for: {stderr}");
}

#[test]
fn running_outside_a_repository_exits_with_code_two() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    std::fs::write(tmp.path().join("pyproject.toml"), "[project]\nname = \"p\"\n")
        .expect("write marker");

    let output =
        gerenuk_bare(tmp.path()).arg("changed-symbols").output().expect("gerenuk should run");

    if output.status.success() {
        // Some images have a git-tracked /tmp. Say so rather than passing
        // silently, or this test quietly stops covering anything.
        eprintln!("SKIPPED: {} is inside a repository", tmp.path().display());
        return;
    }
    assert_eq!(output.status.code(), Some(2), "not a repository is an operational failure");
}

#[test]
fn the_command_never_reaches_for_tyf() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", &SERVICE.replace("return LIMIT", "return LIMIT * 2"));

    // An empty PATH makes `which::which("tyf")` fail, so a run that succeeds
    // proves the tyf discovery was never attempted. `git` is handed over
    // explicitly, since it cannot be found on an empty PATH either.
    let git = which::which("git").expect("git should be on PATH for the test suite");
    let output = gerenuk_bare(repo.path())
        .env("PATH", "")
        .env("GERENUK_GIT", &git)
        .args(["--format", "json", "changed-symbols"])
        .output()
        .expect("gerenuk should run");

    assert!(
        output.status.success(),
        "changed-symbols must work with no tyf and no ty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Exiting zero is not enough — a run that produced nothing would too.
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("the output should be valid JSON");
    assert_eq!(
        pairs(&report, "changed_symbols"),
        vec![("mypkg.service:Shelter.summary".to_string(), "modified".to_string())],
        "and it must produce the real answer, not merely exit zero"
    );
}

#[test]
fn human_output_names_the_base_and_the_changed_symbols() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", &SERVICE.replace("return LIMIT", "return LIMIT * 2"));

    let output =
        gerenuk_bare(repo.path()).arg("changed-symbols").output().expect("gerenuk should run");
    let text = String::from_utf8_lossy(&output.stdout);

    assert!(text.contains("base main"), "the resolved base is stated up front: {text}");
    assert!(text.contains("mypkg.service:Shelter.summary"), "the symbol is listed: {text}");
    assert!(text.contains("modified"), "and what happened to it: {text}");
}

#[test]
fn human_output_says_so_when_nothing_changed() {
    let repo = src_layout_repo();
    let output =
        gerenuk_bare(repo.path()).arg("changed-symbols").output().expect("gerenuk should run");
    let text = String::from_utf8_lossy(&output.stdout);

    assert!(text.contains("No changes against main."), "an empty run must say so: {text}");
}

#[test]
fn the_full_json_report_matches_its_snapshot() {
    let repo = src_layout_repo();
    repo.write("src/mypkg/service.py", &SERVICE.replace("return LIMIT", "return LIMIT * 2"));
    repo.write("src/mypkg/fresh.py", "def brand_new():\n    return 1\n");
    repo.write("tests/test_service.py", "def test_describe():\n    assert 1 == 1\n");
    repo.write("data/schema.sql", "CREATE TABLE t (id INT);\n");

    let mut report = repo.changed_symbols(&[]);
    // The merge-base sha changes every run; everything else must not.
    report["merge_base"] = Value::String("<sha>".to_string());

    insta::assert_snapshot!(
        serde_json::to_string_pretty(&report).expect("the report re-serialises")
    );
}
