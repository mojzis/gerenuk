//! End-to-end tests for `gerenuk run`.
//!
//! Real `git`, a stubbed `tyf` and a stubbed pytest that records the argv it
//! was handed. The load-bearing assertions are about the *three-way* outcome:
//! a selection runs those node ids, a `run_all` verdict runs the bare suite,
//! and an empty selection spawns nothing at all — the recording file must not
//! exist, because an empty pytest argv means "run everything".

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests should abort loudly on a failed assumption"
)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::CommandCargoExt;
use assert_cmd::prelude::*;
use common::{
    fake_fallback, fake_pytest, fake_tyf, gerenuk, json_output, make_executable, recorded_argv,
    recorded_env, TestRepo,
};
use serde_json::Value;
use tempfile::TempDir;

const CORE: &str = r#""""The changed module."""


def target(value):
    return value + 1
"#;

/// A fixture that calls the changed symbol. `tyf` can see this reference; what
/// it cannot see is the edge from the fixture to the tests that consume it.
const CONFTEST: &str = r"import pytest

from mypkg.core import target


@pytest.fixture
def shelter():
    return target(1)
";

const TEST_CORE: &str = r"from mypkg.core import target


def test_target():
    assert target(1) == 2


def test_with_fixture(shelter):
    assert shelter == 2
";

/// A repository whose only interesting edge is the fixture one.
fn repo() -> TestRepo {
    let repo = TestRepo::new();
    repo.write("pyproject.toml", "[project]\nname = \"mypkg\"\n");
    repo.write("src/mypkg/__init__.py", "");
    repo.write("src/mypkg/core.py", CORE);
    repo.write("tests/conftest.py", CONFTEST);
    repo.write("tests/test_core.py", TEST_CORE);
    // A test path pytest collects nothing from: it must never become a node id.
    repo.write("tests/helpers.py", "VALUE = 1\n");
    repo.commit("base");
    repo
}

/// Change `target`'s body, and nothing else.
fn touch_target(repo: &TestRepo) {
    repo.write("src/mypkg/core.py", &CORE.replace("return value + 1", "return value + 2"));
}

/// `tyf refs` for `target`: its own declaration, the fixture body, and one test.
fn refs_fixtures() -> Vec<(&'static str, &'static str)> {
    vec![(
        "src/mypkg/core.py:4:5",
        r#"{"symbol": "target", "reference_count": 1, "references": [
              {"file": "src/mypkg/core.py", "line": 4, "column": 5, "context": "target"}
            ], "test_reference_count": 2, "test_references": [
              {"file": "tests/conftest.py", "line": 8, "column": 12, "context": "shelter"},
              {"file": "tests/test_core.py", "line": 5, "column": 12, "context": "test_target"}
            ]}"#,
    )]
}

/// A prepared `gerenuk run`, with both stubs wired up.
struct Fixture {
    tmp: TempDir,
    repo: TestRepo,
    tyf: PathBuf,
    pytest: PathBuf,
    record: PathBuf,
}

impl Fixture {
    fn new(exit_code: u8) -> Self {
        let tmp = TempDir::new().expect("temp dir");
        let tyf = fake_tyf(&tmp, "[]", &refs_fixtures());
        let record = tmp.path().join("argv.txt");
        let pytest = fake_pytest(&tmp, &record, exit_code);
        Self { tmp, repo: repo(), tyf, pytest, record }
    }

    /// A `gerenuk` with both stubs wired up, and none of git's repository-local
    /// variables: what a hook exports is injected by the tests that are about
    /// it, so the rest do not depend on how this test process was started.
    fn command(&self) -> std::process::Command {
        let mut cmd = gerenuk(self.repo.path(), &self.tyf);
        cmd.env("GERENUK_PYTEST", &self.pytest);
        for name in gerenuk::pytest::LOCAL_GIT_ENV {
            cmd.env_remove(name);
        }
        cmd
    }

    /// Run `gerenuk run <extra>` and return the finished output.
    fn run(&self, extra: &[&str]) -> std::process::Output {
        self.command().arg("run").args(extra).output().expect("gerenuk should run")
    }

    fn argv(&self) -> Option<Vec<String>> {
        recorded_argv(&self.record)
    }

    /// A `gerenuk` that can find neither `tyf` nor pytest.
    ///
    /// An empty `PATH` is what makes both lookups fail; `git` is then named
    /// explicitly, since it could not be found there either.
    fn without_tools(&self) -> std::process::Command {
        let git = which::which("git").expect("git should be on PATH for the test suite");
        let mut cmd = std::process::Command::cargo_bin("gerenuk").expect("gerenuk should be built");
        cmd.current_dir(self.repo.path())
            .env_remove("GERENUK_TYF")
            .env_remove("GERENUK_PYTEST")
            .env("GERENUK_GIT", git)
            .env("PATH", "");
        cmd
    }

    /// Save an `impacted-tests` report, for the cases that must replay one.
    fn save_impact(&self) -> String {
        let report = json_output(self.command().args(["--format", "json", "impacted-tests"]));
        let path = self.tmp.path().join("impact.json");
        std::fs::write(&path, report.to_string()).expect("write the saved report");
        path.display().to_string()
    }
}

#[test]
fn a_selection_becomes_the_node_ids_pytest_is_handed() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    let output = fixture.run(&[]);
    assert!(output.status.success(), "a green stub suite exits 0");
    assert_eq!(
        fixture.argv().expect("pytest should have been spawned"),
        vec!["tests/test_core.py::test_target", "tests/test_core.py::test_with_fixture"],
        "the direct test, and the one the fixture reaches"
    );
}

#[test]
fn a_fixture_carries_the_change_to_the_tests_that_consume_it() {
    // The failure this feature exists for: `tests.conftest:shelter` is not
    // collectible, and `conftest.py` collects zero tests, so without fixture
    // expansion the second test is silently missed.
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    let report = json_output(fixture.command().args(["--format", "json", "run", "--dry-run"]));

    let expanded = &report["expanded"][0];
    assert_eq!(expanded["from"], "tests.conftest:shelter", "the chain names a real symbol id");
    assert_eq!(expanded["kind"], "fixture");
    assert_eq!(expanded["into"][0], "tests/test_core.py::test_with_fixture");

    let reached = report["node_ids"]
        .as_array()
        .expect("an array")
        .iter()
        .find(|entry| entry["node_id"] == "tests/test_core.py::test_with_fixture")
        .expect("the fixture's consumer should be selected");
    assert_eq!(
        reached["via"][0], "tests.conftest:shelter",
        "the fixture is prepended to the why-chain: {report}"
    );
    assert_eq!(reached["origin"], "mypkg.core:target", "and the origin is still the change");
}

#[test]
fn the_passthrough_arrives_after_the_node_ids_verbatim() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    fixture.run(&["--", "-x", "-n", "auto"]);
    let argv = fixture.argv().expect("pytest should have been spawned");
    assert_eq!(
        &argv[argv.len() - 3..],
        ["-x", "-n", "auto"],
        "everything after `--` goes last, unaltered: {argv:?}"
    );
}

#[test]
fn pytests_exit_code_is_the_commands_exit_code() {
    let fixture = Fixture::new(1);
    touch_target(&fixture.repo);

    fixture
        .command()
        .arg("run")
        .assert()
        .code(1)
        // gerenuk's own operational failures are 2; after the exec the code is
        // pytest's, and 1 means "tests failed".
        .stderr(predicates::str::contains("node id(s)"));
}

#[test]
fn a_run_all_verdict_runs_the_bare_suite() {
    let fixture = Fixture::new(0);
    fixture.repo.write("requirements.txt", "requests==2.0\n");

    let output = fixture.run(&["--", "-q"]);
    assert!(output.status.success());
    assert_eq!(
        fixture.argv().expect("pytest should have been spawned"),
        vec!["-q"],
        "no node ids at all — that is what `the whole suite` looks like"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("full suite — non-Python files changed"),
        "and the hook's log says why it went wide: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn an_empty_selection_spawns_nothing_at_all() {
    // The trap the whole command exists to avoid: an empty pytest argv *is*
    // "run everything", so the empty case has to short-circuit before a spawn.
    let fixture = Fixture::new(0);

    let output = fixture.run(&[]);
    assert!(output.status.success(), "nothing impacted is a green result");
    assert_eq!(fixture.argv(), None, "the recording file must not even exist");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no tests impacted"),
        "and the reason is stated: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_dry_run_prints_the_argv_and_spawns_nothing() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    fixture
        .command()
        .args(["run", "--dry-run", "--", "-x"])
        .assert()
        .success()
        .stdout(predicates::str::contains("decision: selected"))
        .stdout(predicates::str::contains("tests/test_core.py::test_target"))
        .stdout(predicates::str::contains("← mypkg.core:target"))
        // One element per line: legible, and not something a shell can
        // interpolate back into `pytest $(...)`.
        .stdout(predicates::str::contains("argv\n  "))
        .stdout(predicates::str::contains("\n  -x\n"));

    assert_eq!(fixture.argv(), None, "a dry run spawns nothing");
}

#[test]
fn a_file_pytest_collects_nothing_from_never_becomes_a_node_id() {
    let fixture = Fixture::new(0);
    fixture.repo.write("tests/helpers.py", "VALUE = 2\n");

    let report = json_output(fixture.command().args(["--format", "json", "run", "--dry-run"]));

    assert!(
        report["node_ids"].as_array().is_some_and(Vec::is_empty),
        "`helpers.py` holds no tests, so handing it over would be exit code 5: {report}"
    );
    assert_eq!(report["decision"], "nothing");
    assert_eq!(report["dropped"][0]["entry"], "tests/helpers.py");
    assert_eq!(report["dropped"][0]["why"], "not_collectible");
}

#[test]
fn a_changed_conftest_selects_the_whole_subtree() {
    let fixture = Fixture::new(0);
    fixture.repo.write("tests/conftest.py", &CONFTEST.replace("target(1)", "target(2)"));

    let output = fixture.run(&[]);
    assert!(output.status.success());
    assert_eq!(
        fixture.argv().expect("pytest should have been spawned"),
        vec!["tests/test_core.py"],
        "the module executes at collection time for everything under it"
    );
}

#[test]
fn a_saved_impact_report_can_be_replayed() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    fixture.run(&["--impact", &fixture.save_impact()]);
    assert_eq!(
        fixture.argv().expect("pytest should have been spawned"),
        vec!["tests/test_core.py::test_target", "tests/test_core.py::test_with_fixture"],
        "a report written by one gerenuk has to be runnable by the next"
    );
}

#[test]
fn a_report_that_is_not_an_impact_report_is_an_operational_failure() {
    // `{}` is valid JSON. Reading it leniently would produce a confident
    // selection of nothing — the one failure this command must not have.
    let fixture = Fixture::new(0);
    let path = fixture.tmp.path().join("empty.json");
    std::fs::write(&path, "{}").expect("write the file");

    fixture
        .command()
        .args(["run", "--impact", &path.display().to_string()])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("empty.json"));
    assert_eq!(fixture.argv(), None, "and nothing was spawned on the way out");
}

#[test]
fn replaying_a_report_conflicts_with_a_base_and_with_every_budget_flag() {
    // The saved report already records the base it was taken against and the
    // budgets its walk ran under; accepting either would silently ignore one.
    let fixture = Fixture::new(0);
    for conflicting in [["--base", "main"], ["--max-depth", "3"]] {
        fixture
            .command()
            .args(["run", "--impact", "report.json"])
            .args(conflicting)
            .assert()
            .code(2)
            .stderr(predicates::str::contains("cannot be used with"));
    }
}

#[test]
fn a_dry_run_still_answers_when_pytest_cannot_be_found() {
    // The selection is the interesting half; reporting it is more use than an
    // error. Replaying a saved report is what lets the run happen with an empty
    // PATH — the `tyf` stub is a shell script and needs one of its own.
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);
    let saved = fixture.save_impact();

    fixture
        .without_tools()
        .args(["run", "--dry-run", "--impact", &saved])
        .assert()
        .success()
        .stdout(predicates::str::contains("decision: selected"))
        .stdout(predicates::str::contains("tests/test_core.py::test_target"))
        // Not "nothing would be run": the decision line above says `selected`,
        // and the two must not contradict each other.
        .stdout(predicates::str::contains("argv: unknown — pytest could not be resolved"))
        .stderr(predicates::str::contains("pytest"));
}

#[test]
fn a_real_run_with_no_pytest_anywhere_is_an_operational_failure() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);
    let saved = fixture.save_impact();

    fixture
        .without_tools()
        .args(["run", "--impact", &saved])
        // Exit 2, gerenuk's own: pytest never ran, so there is no code of its
        // own to propagate.
        .assert()
        .code(2)
        .stderr(predicates::str::contains("pytest"));
    assert_eq!(fixture.argv(), None, "and nothing was spawned on the way out");
}

#[test]
fn a_non_python_change_answers_before_tyf_or_pytest_is_looked_for() {
    // `run` inherits the up-front gate from `impacted-tests`; this pins that it
    // still holds now that a pytest lookup sits on the same path.
    let fixture = Fixture::new(0);
    fixture.repo.write("requirements.txt", "requests==2.0\n");

    let output = fixture
        .without_tools()
        .args(["--format", "json", "run", "--dry-run"])
        .output()
        .expect("gerenuk should run");

    assert!(output.status.success(), "a run_all verdict is an answer, not a failure");
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("the dry run should print JSON");
    assert_eq!(report["decision"], "run_all");
    assert_eq!(
        report["reason"], "non_python_changes",
        "settled up front, with no ty and no pytest installed at all: {report}"
    );
}

#[test]
fn a_suite_declared_faster_than_a_selection_skips_the_walk_before_tyf_is_looked_for() {
    let fixture = Fixture::new(0);
    fixture
        .repo
        .write("pyproject.toml", "[project]\nname = \"mypkg\"\n\n[tool.gerenuk]\nsuite-ms = 800\n");
    fixture.repo.commit("declare the suite");
    touch_target(&fixture.repo);

    let output = fixture
        .without_tools()
        .args(["--format", "json", "run", "--dry-run"])
        .output()
        .expect("gerenuk should run");

    assert!(output.status.success(), "run_all is an answer: {output:?}");
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("the dry run should print JSON");
    assert_eq!(report["decision"], "run_all");
    assert_eq!(
        report["reason"], "fast_suite",
        "settled from the diff alone, with no ty installed: {report}"
    );
    assert_eq!(report["argv"], serde_json::json!([]), "no pytest either, so no argv: {report}");
}

#[test]
fn a_fast_suite_still_runs_only_the_changed_test_files_when_nothing_needs_walking() {
    // The declared duration is not a switch: a diff that seeds no walk answers
    // for free, and that answer beats the full suite.
    let fixture = Fixture::new(0);
    fixture
        .repo
        .write("pyproject.toml", "[project]\nname = \"mypkg\"\n\n[tool.gerenuk]\nsuite-ms = 800\n");
    fixture.repo.commit("declare the suite");
    fixture
        .repo
        .write("tests/test_core.py", &format!("{TEST_CORE}\n\ndef test_more():\n    pass\n"));

    let output = fixture.run(&["--dry-run", "--format", "json"]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON");

    assert_eq!(report["decision"], "selected", "{report}");
    assert_eq!(report["node_ids"][0]["node_id"], "tests/test_core.py", "{report}");
    assert_eq!(report["node_ids"].as_array().map(Vec::len), Some(1), "{report}");
}

#[test]
fn a_fast_suite_slower_than_a_selection_is_walked_as_usual() {
    let fixture = Fixture::new(0);
    fixture.repo.write(
        "pyproject.toml",
        "[project]\nname = \"mypkg\"\n\n[tool.gerenuk]\nsuite-ms = 30000\n",
    );
    fixture.repo.commit("declare the suite");
    touch_target(&fixture.repo);

    let output = fixture.run(&["--dry-run", "--format", "json"]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON");

    assert_eq!(report["decision"], "selected", "a slow suite is worth selecting for: {report}");
}

#[test]
fn a_fast_suite_delegates_to_the_fallback_like_any_run_all() {
    let fixture = Fixture::new(0);
    fixture.repo.write(
        "pyproject.toml",
        "[project]\nname = \"mypkg\"\n\n[tool.gerenuk]\nsuite-ms = 1\nfallback-command = [\"scripts/pick.sh\"]\n",
    );
    fixture.repo.commit("declare the suite");
    touch_target(&fixture.repo);

    let output = fixture.run(&["--dry-run", "--format", "json"]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON");

    assert_eq!(report["fallback"]["reason"], "fast_suite", "{report}");
    assert_eq!(
        report["fallback"]["payload"]["report"]["changed_symbols"][0]["symbol"],
        "mypkg.core:target",
        "the fallback still gets the diff it can narrow on: {report}"
    );
}

#[test]
fn the_configured_pytest_command_is_used_when_no_override_is_set() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);
    fixture.repo.write(
        "pyproject.toml",
        &format!(
            "[project]\nname = \"mypkg\"\n\n[tool.gerenuk]\npytest-command = [\"{}\", \"--flag\"]\n",
            fixture.pytest.display()
        ),
    );

    let output = fixture
        .command()
        .env_remove("GERENUK_PYTEST")
        .args(["--format", "json", "run", "--dry-run"])
        .output()
        .expect("gerenuk should run");
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("the dry run should print JSON");

    assert_eq!(
        report["argv"][0],
        *fixture.pytest.display().to_string(),
        "a multi-word runner keeps its order: {report}"
    );
    assert_eq!(report["argv"][1], "--flag", "including its own arguments");
}

#[test]
fn the_dry_run_json_matches_its_snapshot() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    let mut report = json_output(fixture.command().args(["--format", "json", "run", "--dry-run"]));
    // The stub's path is a fresh temp directory on every run.
    report["argv"][0] = Value::from("<pytest>");

    insta::assert_snapshot!(
        serde_json::to_string_pretty(&report).expect("the report re-serialises")
    );
}

/// A stub fallback command, and the files it records into.
struct FallbackStub {
    script: PathBuf,
    prefix: PathBuf,
}

impl FallbackStub {
    /// The stub, written into `dir` as `name`, exiting with `code`.
    fn new(dir: &Path, name: &str, prefix: &Path, code: u8) -> Self {
        Self { script: fake_fallback(dir, name, prefix, code), prefix: prefix.to_path_buf() }
    }

    fn recorded(&self, what: &str) -> Option<String> {
        std::fs::read_to_string(self.prefix.with_extension(what)).ok()
    }

    /// The argv the stub was handed, or `None` when it never ran.
    fn argv(&self) -> Option<Vec<String>> {
        recorded_argv(&self.prefix.with_extension("argv"))
    }

    fn payload(&self) -> Value {
        let text = self.recorded("stdin").expect("the stub should have read its stdin");
        serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("stdin is not JSON ({err}): {text}"))
    }

    /// `fallback-command = [<script>, <args>…]`, as TOML.
    fn config(&self, args: &[&str]) -> String {
        let command: Vec<String> = std::iter::once(self.script.display().to_string())
            .chain(args.iter().map(ToString::to_string))
            .collect();
        format!(
            "fallback-command = {}\n",
            serde_json::to_string(&command).expect("strings serialise")
        )
    }
}

impl Fixture {
    /// A fallback stub living in the fixture's temp dir, exiting with `code`.
    fn fallback(&self, name: &str, code: u8) -> FallbackStub {
        let prefix = self.tmp.path().join(format!("{name}-record"));
        FallbackStub::new(self.tmp.path(), name, &prefix, code)
    }

    /// Rewrite the repo's `pyproject.toml` with `body` under `[tool.gerenuk]`,
    /// and commit it: a configuration change is a non-Python change, and left
    /// in the working tree it would force every outcome to `run_all`.
    fn configure(&self, body: &str) {
        self.repo.write(
            "pyproject.toml",
            &format!("[project]\nname = \"mypkg\"\n\n[tool.gerenuk]\n{body}"),
        );
        self.repo.commit("configure");
    }

    /// A change gerenuk cannot reason about at all: the `run_all` outcome.
    fn touch_non_python(&self) {
        self.repo.write("requirements.txt", "requests==2.0\n");
    }
}

#[test]
fn a_run_all_outcome_delegates_to_the_configured_fallback() {
    let fixture = Fixture::new(0);
    let stub = fixture.fallback("fallback", 0);
    fixture.configure(&stub.config(&["--from-gerenuk"]));
    fixture.touch_non_python();

    let output = fixture.run(&["--", "-q"]);
    assert!(
        output.status.success(),
        "the stub exited 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fixture.argv(), None, "pytest is not spawned when the fallback owns the run");
    assert_eq!(
        stub.argv().expect("the fallback should have been exec'd"),
        vec!["--from-gerenuk"],
        "the configured argv, verbatim — the pytest passthrough is not appended to it"
    );
    assert_eq!(
        stub.recorded("reason").as_deref(),
        Some("non_python_changes"),
        "GERENUK_FALLBACK_REASON lets a shell script branch without parsing JSON"
    );

    let payload = stub.payload();
    assert_eq!(
        payload["gerenuk_fallback_payload_version"], 1,
        "the payload is versioned: {payload}"
    );
    assert_eq!(payload["reason"], "non_python_changes");
    assert_eq!(
        payload["report"]["non_python_changes"],
        serde_json::json!(["requirements.txt"]),
        "the report is the changed-symbols report the run was computed from: {payload}"
    );
    assert_eq!(payload["report"]["base"], "main", "with the base it was taken against");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("fallback"), "the hook's log says the run was delegated: {stderr}");
    assert!(stderr.contains("non-Python files changed"), "and why: {stderr}");
}

#[test]
fn the_fallbacks_exit_code_is_the_commands_exit_code() {
    let fixture = Fixture::new(0);
    let stub = fixture.fallback("fallback", 3);
    fixture.configure(&stub.config(&[]));
    fixture.touch_non_python();

    fixture.command().arg("run").assert().code(3);
    assert!(stub.argv().is_some(), "the code is the stub's own, propagated verbatim");
}

#[test]
fn a_fallback_that_never_reads_stdin_still_exits_cleanly() {
    // The payload is delivered from a file, not a pipe: a script that ignores
    // it must neither block gerenuk nor fail.
    let fixture = Fixture::new(0);
    let stub = fixture.fallback("fallback", 0);
    fixture.configure(&stub.config(&["--skip-stdin"]));
    fixture.touch_non_python();

    fixture.command().arg("run").assert().success();
    assert_eq!(stub.argv().expect("the stub ran"), vec!["--skip-stdin"]);
    assert_eq!(stub.recorded("stdin"), None, "the stub never read its stdin");
    assert_eq!(stub.recorded("reason").as_deref(), Some("non_python_changes"));
}

#[test]
fn a_repo_relative_fallback_is_resolved_against_the_repo_root_not_the_cwd() {
    let fixture = Fixture::new(0);
    let prefix = fixture.tmp.path().join("relative-record");
    let stub = FallbackStub::new(fixture.repo.path(), "scripts/fallback.sh", &prefix, 0);
    fixture.configure("fallback-command = [\"scripts/fallback.sh\"]\n");
    fixture.touch_non_python();

    // Run from a subdirectory: `scripts/fallback.sh` does not exist relative
    // to it, so finding the script proves it was resolved against the root.
    fixture.command().current_dir(fixture.repo.path().join("src")).arg("run").assert().success();

    let root = fixture.repo.path().canonicalize().expect("the repo exists");
    assert_eq!(
        stub.recorded("cwd").map(PathBuf::from),
        Some(root),
        "and it runs from the repo root, like pytest does"
    );
    assert!(stub.script.exists(), "the stub is inside the repository");
}

#[test]
fn the_fallback_is_not_invoked_for_a_selected_outcome() {
    let fixture = Fixture::new(0);
    let stub = fixture.fallback("fallback", 0);
    fixture.configure(&stub.config(&[]));
    touch_target(&fixture.repo);

    fixture.command().arg("run").assert().success();
    assert_eq!(
        fixture.argv().expect("pytest should have been spawned"),
        vec!["tests/test_core.py::test_target", "tests/test_core.py::test_with_fixture"],
        "a selection is pytest's, exactly as without a fallback"
    );
    assert_eq!(stub.argv(), None, "the fallback's marker must not exist");
}

#[test]
fn the_fallback_is_not_invoked_for_an_empty_selection() {
    let fixture = Fixture::new(0);
    let stub = fixture.fallback("fallback", 0);
    fixture.configure(&stub.config(&[]));

    fixture.command().arg("run").assert().success();
    assert_eq!(fixture.argv(), None, "nothing impacted spawns no pytest");
    assert_eq!(stub.argv(), None, "and no fallback either");
}

#[test]
fn the_flag_beats_the_env_var_which_beats_the_config() {
    let fixture = Fixture::new(0);
    let from_config = fixture.fallback("from-config", 0);
    let from_env = fixture.fallback("from-env", 0);
    let from_flag = fixture.fallback("from-flag", 0);
    fixture.configure(&from_config.config(&[]));
    fixture.touch_non_python();

    let as_json = |stub: &FallbackStub| {
        serde_json::to_string(&[stub.script.display().to_string()]).expect("strings serialise")
    };

    fixture.command().env("GERENUK_FALLBACK", as_json(&from_env)).arg("run").assert().success();
    assert!(from_env.argv().is_some(), "the environment beats pyproject.toml");
    assert_eq!(from_config.argv(), None);

    fixture
        .command()
        .env("GERENUK_FALLBACK", as_json(&from_env))
        .args(["run", "--fallback-command", &as_json(&from_flag)])
        .assert()
        .success();
    assert!(from_flag.argv().is_some(), "and the flag beats the environment");
}

#[test]
fn an_empty_fallback_command_fails_at_startup_whatever_the_outcome() {
    // The outcome here would be `selected`, so the fallback would never have
    // been needed. It fails anyway: a config error is found when the config is
    // read, not on the day the bail-out first happens.
    let fixture = Fixture::new(0);
    fixture.configure("fallback-command = []\n");
    touch_target(&fixture.repo);

    fixture
        .command()
        .arg("run")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("fallback-command"))
        .stderr(predicates::str::contains("empty"));
    assert_eq!(fixture.argv(), None, "nothing was spawned on the way out");

    fixture.configure("");
    fixture
        .command()
        .env("GERENUK_FALLBACK", "[]")
        .arg("run")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("GERENUK_FALLBACK"));
    assert_eq!(fixture.argv(), None, "the environment is checked the same way");
}

#[test]
fn a_fallback_that_is_not_a_json_array_is_a_config_error() {
    let fixture = Fixture::new(0);
    fixture.touch_non_python();

    fixture
        .command()
        .args(["run", "--fallback-command", "scripts/pick.sh --from-gerenuk"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--fallback-command"))
        .stderr(predicates::str::contains("JSON array"));
    assert_eq!(fixture.argv(), None, "a shell string is not accepted, so nothing ran");
}

#[test]
fn a_dry_run_never_executes_the_fallback() {
    let fixture = Fixture::new(0);
    let stub = fixture.fallback("fallback", 0);
    fixture.configure(&stub.config(&["--from-gerenuk"]));
    fixture.touch_non_python();

    fixture
        .command()
        .args(["run", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains("decision: run_all"))
        .stdout(predicates::str::contains("would exec fallback: "))
        .stdout(predicates::str::contains("--from-gerenuk"))
        .stdout(predicates::str::contains("(reason: non_python_changes)"));
    assert_eq!(stub.argv(), None, "a dry run spawns nothing");
    assert_eq!(fixture.argv(), None);
}

#[test]
fn a_missing_fallback_binary_is_an_operational_failure_that_names_it() {
    let fixture = Fixture::new(0);
    fixture.configure("fallback-command = [\"scripts/missing.sh\", \"--from-gerenuk\"]\n");
    fixture.touch_non_python();

    let resolved = fixture.repo.path().join("scripts/missing.sh");
    fixture
        .command()
        .arg("run")
        .assert()
        .code(2)
        .stderr(predicates::str::contains(resolved.display().to_string()))
        .stderr(predicates::str::contains("pyproject.toml"));
    assert_eq!(fixture.argv(), None, "pytest did not run in its place");
}

#[test]
fn a_replayed_run_all_report_has_no_changed_symbols_to_hand_over() {
    // `--impact` replays a phase-2 report and never diffs the tree, so there is
    // no phase-1 report to put in the payload. The field is null rather than a
    // fabricated empty report, which would read as "nothing changed".
    let fixture = Fixture::new(0);
    let stub = fixture.fallback("fallback", 0);
    fixture.configure(&stub.config(&[]));
    fixture.touch_non_python();
    let saved = fixture.save_impact();

    fixture.command().args(["run", "--impact", &saved]).assert().success();
    let payload = stub.payload();
    assert_eq!(payload["reason"], "non_python_changes", "the reason is the report's");
    assert!(payload["report"].is_null(), "and there is no report to hand over: {payload}");
}

/// The dry-run JSON with every per-run value pinned, so it can be snapshotted.
fn pinned_dry_run(fixture: &Fixture) -> String {
    let mut report = json_output(fixture.command().args(["--format", "json", "run", "--dry-run"]));
    let root = fixture.repo.path().display().to_string();
    let mut text = serde_json::to_string_pretty(&report).expect("the report re-serialises");
    if let Some(sha) = report["fallback"]["payload"]["report"]["merge_base"].as_str() {
        text = text.replace(sha, "<sha>");
    }
    report = serde_json::from_str(&text.replace(&root, "<repo>")).expect("still JSON");
    serde_json::to_string_pretty(&report).expect("the report re-serialises")
}

#[test]
fn the_dry_run_json_with_a_fallback_matches_its_snapshot() {
    let fixture = Fixture::new(0);
    fixture.configure("fallback-command = [\"scripts/pick.sh\", \"--from-gerenuk\"]\n");
    fixture.touch_non_python();

    insta::assert_snapshot!(pinned_dry_run(&fixture));
}

#[test]
fn the_dry_run_json_without_a_fallback_matches_its_snapshot() {
    let fixture = Fixture::new(0);
    fixture.touch_non_python();

    let mut report = json_output(fixture.command().args(["--format", "json", "run", "--dry-run"]));
    report["argv"][0] = Value::from("<pytest>");
    insta::assert_snapshot!(
        serde_json::to_string_pretty(&report).expect("the report re-serialises")
    );
}

// --- The git environment ---
//
// A pre-commit hook runs with git's repository-local variables exported —
// `GIT_DIR`, `GIT_INDEX_FILE`, `GIT_PREFIX` and the rest of `git rev-parse
// --local-env-vars` — so that every git the hook spawns targets the repository
// being committed. A test that creates a repository of its own and inherits
// them operates on the outer repository instead. pytest must not inherit them;
// gerenuk's own diff, and the fallback, must.

/// The variables a hook exports, pointed at the fixture repository itself so
/// gerenuk's own git calls keep working while the child would see them.
fn hook_env(fixture: &Fixture) -> Vec<(&'static str, String)> {
    let git_dir = fixture.repo.path().join(".git");
    vec![
        ("GIT_DIR", git_dir.display().to_string()),
        ("GIT_INDEX_FILE", git_dir.join("index").display().to_string()),
        ("GIT_WORK_TREE", fixture.repo.path().display().to_string()),
        // Set but empty, which is how a hook at the root exports it.
        ("GIT_PREFIX", String::new()),
    ]
}

impl Fixture {
    /// The environment the pytest stub recorded, or `None` when it never ran.
    fn env(&self) -> Option<BTreeMap<String, String>> {
        recorded_env(&self.record.with_extension("env"))
    }

    /// `gerenuk run <extra>` under the environment a hook provides, plus one
    /// unrelated variable that has to come through untouched.
    fn run_as_hook(&self, extra: &[&str]) -> std::process::Output {
        self.command()
            .envs(hook_env(self))
            .env("KEEP_ME", "yes")
            .arg("run")
            .args(extra)
            .output()
            .expect("gerenuk should run")
    }
}

impl FallbackStub {
    fn env(&self) -> Option<BTreeMap<String, String>> {
        recorded_env(&self.prefix.with_extension("env"))
    }
}

#[test]
fn pytest_does_not_inherit_the_hooks_repository_local_git_variables() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    let output = fixture.run_as_hook(&[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        fixture.argv().expect("pytest should have been spawned"),
        vec!["tests/test_core.py::test_target", "tests/test_core.py::test_with_fixture"],
        "the selection is still computed in the hook's own context"
    );

    let env = fixture.env().expect("the stub records its environment");
    for (name, _) in hook_env(&fixture) {
        assert!(!env.contains_key(name), "`{name}` must not reach pytest: {env:?}");
    }
    assert_eq!(env.get("KEEP_ME").map(String::as_str), Some("yes"), "unrelated variables stay");
    assert!(env.contains_key("PATH"), "and so does PATH: {env:?}");
}

#[test]
fn the_git_environment_is_inherited_when_the_config_says_so() {
    let fixture = Fixture::new(0);
    fixture.configure("git-env = \"inherit\"\n");
    touch_target(&fixture.repo);

    let output = fixture.run_as_hook(&[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let env = fixture.env().expect("pytest should have been spawned");
    for (name, value) in hook_env(&fixture) {
        assert_eq!(env.get(name), Some(&value), "`{name}` is handed on verbatim: {env:?}");
    }

    // The flag beats the config, in the direction that matters for a one-off.
    fixture.run_as_hook(&["--git-env", "isolate"]);
    let env = fixture.env().expect("pytest should have been spawned");
    assert!(!env.contains_key("GIT_DIR"), "`--git-env isolate` beats the config: {env:?}");
}

#[test]
fn the_git_environment_is_inherited_when_the_flag_says_so() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    let output = fixture.run_as_hook(&["--git-env", "inherit"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let env = fixture.env().expect("pytest should have been spawned");
    assert!(env.contains_key("GIT_INDEX_FILE"), "inherited on request: {env:?}");
    assert_eq!(env.get("GIT_PREFIX").map(String::as_str), Some(""), "set-but-empty stays set");
}

#[test]
fn an_unknown_git_env_policy_is_a_usage_error() {
    let fixture = Fixture::new(0);
    fixture
        .command()
        .args(["run", "--git-env", "strip"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("isolate"))
        .stderr(predicates::str::contains("inherit"));
    assert_eq!(fixture.argv(), None, "nothing ran");
}

#[test]
fn an_unknown_git_env_policy_in_the_config_names_the_key() {
    let fixture = Fixture::new(0);
    fixture.configure("git-env = \"strip\"\n");
    touch_target(&fixture.repo);
    fixture
        .command()
        .arg("run")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("git-env"))
        .stderr(predicates::str::contains("pyproject.toml"));
    assert_eq!(fixture.argv(), None, "nothing ran");
}

#[test]
fn the_fallback_inherits_the_hooks_git_variables() {
    // The fallback is the repository's own script, run from the hook it was
    // configured for: it may need the very index git handed the hook. It
    // inherits everything, and the policy for pytest does not apply to it.
    let fixture = Fixture::new(0);
    let stub = fixture.fallback("fallback", 0);
    fixture.configure(&stub.config(&[]));
    fixture.touch_non_python();

    let output = fixture.run_as_hook(&[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let env = stub.env().expect("the fallback should have been exec'd");
    for (name, value) in hook_env(&fixture) {
        assert_eq!(env.get(name), Some(&value), "`{name}` reaches the fallback: {env:?}");
    }
    assert_eq!(env.get("GERENUK_FALLBACK_REASON").map(String::as_str), Some("non_python_changes"));
    assert_eq!(env.get("KEEP_ME").map(String::as_str), Some("yes"));
}

#[test]
fn the_dry_run_reports_the_git_environment_policy() {
    let fixture = Fixture::new(0);
    touch_target(&fixture.repo);

    let report = json_output(fixture.command().envs(hook_env(&fixture)).args([
        "--format",
        "json",
        "run",
        "--dry-run",
    ]));
    assert_eq!(report["git_env"]["policy"], "isolate");
    let removed: Vec<&str> = report["git_env"]["removed"]
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        removed,
        vec!["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_PREFIX"],
        "the variables that are set right now, in git's own order: {report}"
    );

    let text = String::from_utf8(
        fixture
            .command()
            .envs(hook_env(&fixture))
            .args(["run", "--dry-run"])
            .output()
            .expect("runs")
            .stdout,
    )
    .expect("utf-8");
    assert!(
        text.contains(
            "git env: isolate — removes GIT_DIR, GIT_WORK_TREE, GIT_INDEX_FILE, GIT_PREFIX"
        ),
        "got:\n{text}"
    );

    let report = json_output(fixture.command().envs(hook_env(&fixture)).args([
        "--format",
        "json",
        "run",
        "--dry-run",
        "--git-env",
        "inherit",
    ]));
    assert_eq!(report["git_env"]["policy"], "inherit");
    assert_eq!(report["git_env"]["removed"], Value::Array(Vec::new()), "nothing is removed");
}

#[test]
fn the_removed_variables_cover_gits_own_list() {
    // Git publishes the list; gerenuk's copy is pure so the rules stay
    // testable without git. This is the one place the two are compared, so
    // a newer git that adds a variable fails here rather than in a hook.
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--local-env-vars"])
        .output()
        .expect("git should be on PATH for the test suite");
    assert!(output.status.success());
    let listed: Vec<&str> = std::str::from_utf8(&output.stdout).expect("utf-8").lines().collect();
    assert!(!listed.is_empty(), "git names at least GIT_DIR");
    for name in listed {
        assert!(
            gerenuk::pytest::LOCAL_GIT_ENV.contains(&name),
            "git lists `{name}` as repository-local and gerenuk does not remove it"
        );
    }
}

// --- Through a real hook ---
//
// The deterministic tests above inject the variables. These let git export
// them: a pre-commit hook runs `gerenuk run`, and the "pytest" it execs does
// what the test in the report did — initialises a second repository,
// configures an identity there, stages a file and commits it with `git -C`.
// The outer repository must come out of the commit with exactly the commit
// the user asked for, and nothing of the fixture's.

/// A pytest that commits to a repository of its own, and records what it did.
struct GitUsingPytest {
    bin: PathBuf,
    inner: PathBuf,
    record: PathBuf,
}

impl GitUsingPytest {
    fn new(dir: &Path) -> Self {
        let inner = dir.join("inner");
        let record = dir.join("git-pytest-record");
        let bin = dir.join("git-pytest");
        std::fs::write(
            &bin,
            format!(
                r#"#!/usr/bin/env bash
set -euo pipefail
inner='{inner}'
printf '%s\n' "$@" > '{record}.argv'
mkdir -p "$inner"
git -C "$inner" init -q --initial-branch=main
git -C "$inner" config user.email fixture@example.com
git -C "$inner" config user.name Fixture
printf 'data\n' > "$inner/fixture.txt"
git -C "$inner" add fixture.txt
git -C "$inner" commit -qm fixture
git -C "$inner" rev-parse HEAD > '{record}.head'
"#,
                inner = inner.display(),
                record = record.display(),
            ),
        )
        .expect("write the git-using pytest");
        make_executable(&bin);
        Self { bin, inner, record }
    }

    fn argv(&self) -> Option<Vec<String>> {
        recorded_argv(&self.record.with_extension("argv"))
    }

    fn inner_head(&self) -> Option<String> {
        std::fs::read_to_string(self.record.with_extension("head"))
            .ok()
            .map(|s| s.trim().to_string())
    }
}

/// A fixture repository with a pre-commit hook that runs `gerenuk run`.
struct Hooked {
    fixture: Fixture,
    pytest: GitUsingPytest,
}

impl Hooked {
    fn new() -> Self {
        let fixture = Fixture::new(0);
        let pytest = GitUsingPytest::new(fixture.tmp.path());
        let hook = fixture.repo.path().join(".git/hooks/pre-commit");
        std::fs::write(
            &hook,
            format!("#!/usr/bin/env bash\nexec '{}' run\n", env!("CARGO_BIN_EXE_gerenuk")),
        )
        .expect("write the hook");
        make_executable(&hook);
        Self { fixture, pytest }
    }

    /// `git <args>` in `dir`, with what a hooked `gerenuk run` needs: the
    /// `tyf` stub and the git-using pytest, and nothing of this machine's
    /// git config.
    fn git(&self, dir: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GERENUK_TYF", &self.fixture.tyf)
            .env("GERENUK_PYTEST", &self.pytest.bin)
            .env_remove("GERENUK_FALLBACK")
            .args(args)
            .output()
            .expect("git should be on PATH for the test suite")
    }

    fn git_ok(&self, dir: &Path, args: &[&str]) -> String {
        let output = self.git(dir, args);
        assert!(
            output.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// The commit went through, pytest saw the selection, the fixture's commit
    /// landed in the fixture's repository, and the outer one carries nothing
    /// of it.
    fn assert_only_the_outer_commit_happened(&self, dir: &Path, head_before: &str) {
        assert_eq!(
            self.pytest.argv().expect("the hook should have reached pytest"),
            vec!["tests/test_core.py::test_target", "tests/test_core.py::test_with_fixture"],
            "the selection was computed in the hook's context"
        );

        let head = self.git_ok(dir, &["rev-parse", "HEAD"]);
        assert_ne!(head, head_before, "the outer commit was created");
        assert_eq!(
            self.git_ok(dir, &["rev-parse", "HEAD~1"]),
            head_before,
            "exactly one commit, the outer one"
        );
        let tree = self.git_ok(dir, &["ls-tree", "-r", "--name-only", "HEAD"]);
        assert!(
            !tree.contains("fixture.txt"),
            "the fixture's file is not in the outer commit:\n{tree}"
        );
        assert_eq!(self.git_ok(dir, &["status", "--porcelain"]), "", "the outer index is clean");
        assert_eq!(
            self.git_ok(dir, &["config", "user.email"]),
            "test@example.com",
            "the outer identity is untouched"
        );
        assert_eq!(
            self.git_ok(dir, &["log", "--format=%ae", "-1"]),
            "test@example.com",
            "and the outer commit was made with it"
        );

        let inner_head = self.pytest.inner_head().expect("the fixture recorded its commit");
        assert_ne!(inner_head, head, "the fixture committed somewhere else");
        assert_eq!(
            self.git_ok(&self.pytest.inner, &["log", "--format=%s"]),
            "fixture",
            "one commit, its own"
        );
        assert_eq!(
            self.git_ok(&self.pytest.inner, &["rev-parse", "HEAD"]),
            inner_head,
            "and it is the fixture repository that carries it"
        );
    }
}

#[test]
fn a_hooked_run_lets_a_test_commit_to_its_own_repository_and_not_the_outer_one() {
    let hooked = Hooked::new();
    let root = hooked.fixture.repo.path().to_path_buf();
    touch_target(&hooked.fixture.repo);
    hooked.git_ok(&root, &["add", "-A"]);
    let before = hooked.git_ok(&root, &["rev-parse", "HEAD"]);

    // A partial commit: git hands the hook an absolute path to a temporary
    // index, which is the variable that turned the fixture's `git add` into
    // a write to the outer index.
    let output = hooked.git(&root, &["commit", "-qm", "change", "--", "src/mypkg/core.py"]);
    assert!(
        output.status.success(),
        "the hook should pass:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    hooked.assert_only_the_outer_commit_happened(&root, &before);
}

#[test]
fn a_hooked_run_in_a_linked_worktree_leaves_the_worktree_and_its_branch_alone() {
    let hooked = Hooked::new();
    let worktree = hooked.fixture.tmp.path().join("worktree");
    hooked.fixture.repo.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "task",
        &worktree.display().to_string(),
    ]);
    std::fs::write(
        worktree.join("src/mypkg/core.py"),
        CORE.replace("return value + 1", "return value + 2"),
    )
    .expect("write the change in the worktree");
    hooked.git_ok(&worktree, &["add", "-A"]);
    let before = hooked.git_ok(&worktree, &["rev-parse", "HEAD"]);
    let main_before = hooked.git_ok(&worktree, &["rev-parse", "main"]);

    // In a linked worktree git exports an absolute `GIT_DIR`, which is the
    // variable that turned the fixture's `git config` and `git commit` into
    // writes to the outer repository and its branch.
    let output = hooked.git(&worktree, &["commit", "-qm", "change"]);
    assert!(
        output.status.success(),
        "the hook should pass:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    hooked.assert_only_the_outer_commit_happened(&worktree, &before);
    assert_eq!(hooked.git_ok(&worktree, &["rev-parse", "main"]), main_before, "main is untouched");
    assert_eq!(hooked.git_ok(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]), "task");
}

#[test]
fn a_hooked_run_still_propagates_pytests_failure() {
    let hooked = Hooked::new();
    let root = hooked.fixture.repo.path().to_path_buf();
    // A pytest that fails, so the hook has to block the commit.
    std::fs::write(&hooked.pytest.bin, "#!/usr/bin/env bash\nexit 1\n").expect("rewrite the stub");
    touch_target(&hooked.fixture.repo);
    hooked.git_ok(&root, &["add", "-A"]);
    let before = hooked.git_ok(&root, &["rev-parse", "HEAD"]);

    let output = hooked.git(&root, &["commit", "-qm", "change"]);
    assert!(!output.status.success(), "a red suite blocks the commit");
    assert_eq!(hooked.git_ok(&root, &["rev-parse", "HEAD"]), before, "no commit was created");
}
