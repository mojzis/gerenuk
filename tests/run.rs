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

use std::path::{Path, PathBuf};

use assert_cmd::cargo::CommandCargoExt;
use assert_cmd::prelude::*;
use common::{fake_fallback, fake_pytest, fake_tyf, gerenuk, json_output, recorded_argv, TestRepo};
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

    fn command(&self) -> std::process::Command {
        let mut cmd = gerenuk(self.repo.path(), &self.tyf);
        cmd.env("GERENUK_PYTEST", &self.pytest);
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
