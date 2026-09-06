//! End-to-end tests of the binary's argument handling and failure modes.

mod common;

use assert_cmd::prelude::*;
use predicates::str::contains;
use std::process::Command;
use tempfile::TempDir;

use common::{failing_tyf, gerenuk, sample_pkg, SERVICE_OUTLINE};

#[test]
fn help_lists_the_available_commands() {
    Command::cargo_bin("gerenuk")
        .expect("binary should build")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("audit"))
        .stdout(contains("doctor"))
        .stdout(contains("guide"));
}

/// `guide` is what an agent runs before anything is installed: no `tyf` on
/// PATH, no git repository, no pyproject. It must still print.
#[test]
fn guide_prints_setup_with_nothing_installed_and_no_repository() {
    let tmp = TempDir::new().expect("temp dir");
    let output = common::gerenuk_no_tyf(tmp.path()).arg("guide").output().expect("runs");
    assert_eq!(output.status.code(), Some(0), "an inventory never fails");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("# gerenuk guide: not configured here -> setup\n\n# Setup gerenuk"),
        "got: {stdout}"
    );
    assert!(stdout.contains("uv add --dev gerenuk"), "setup says how to install");
    assert!(output.stderr.is_empty(), "the guide is output, not logging");
}

#[test]
fn guide_picks_triage_once_a_madoqua_step_names_gerenuk() {
    let tmp = TempDir::new().expect("temp dir");
    std::fs::write(
        tmp.path().join("pyproject.toml"),
        "[tool.madoqua]\nextend_check = [{ name = \"gerenuk\", cmd = \"gerenuk run -- -q\", pass_files = false }]\n",
    )
    .expect("write pyproject");
    common::gerenuk_no_tyf(tmp.path())
        .arg("guide")
        .assert()
        .success()
        .stdout(contains("# gerenuk guide: configured via pyproject.toml [tool.madoqua] -> triage"))
        .stdout(contains("`run_all` ladder"));
}

#[test]
fn guide_prints_the_named_topic_whatever_the_directory_holds() {
    let tmp = TempDir::new().expect("temp dir");
    std::fs::write(
        tmp.path().join("pyproject.toml"),
        "[tool.madoqua]\ncheck = [\"gerenuk run\"]\n",
    )
    .expect("write pyproject");
    common::gerenuk_no_tyf(tmp.path())
        .args(["guide", "setup"])
        .assert()
        .success()
        .stdout(contains("# gerenuk guide: setup\n"));
    common::gerenuk_no_tyf(tmp.path())
        .args(["guide", "tune"])
        .assert()
        .success()
        .stdout(contains("# gerenuk guide: tune\n"))
        .stdout(contains("GERENUK_TYF"));
}

#[test]
fn an_unknown_guide_topic_is_a_usage_error() {
    let tmp = TempDir::new().expect("temp dir");
    common::gerenuk_no_tyf(tmp.path())
        .args(["guide", "how"])
        .assert()
        .code(2)
        .stderr(contains("invalid value"));
}

#[test]
fn version_matches_the_crate_version() {
    Command::cargo_bin("gerenuk")
        .expect("binary should build")
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn an_unknown_subcommand_fails_with_usage() {
    Command::cargo_bin("gerenuk")
        .expect("binary should build")
        .arg("nonsense")
        .assert()
        .failure()
        .stderr(contains("Usage"));
}

#[test]
fn doctor_reports_the_resolved_workspace_and_binary() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = common::fake_tyf(&tmp, SERVICE_OUTLINE, &[]);
    let pkg = sample_pkg();

    gerenuk(&pkg, &tyf)
        .arg("doctor")
        .assert()
        .success()
        .stdout(contains("sample_pkg"))
        .stdout(contains("fake-tyf"));
}

#[test]
fn an_explicit_workspace_overrides_detection() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = common::fake_tyf(&tmp, SERVICE_OUTLINE, &[]);
    let pkg = sample_pkg();

    // Run from a directory that is not the fixture, but point --workspace at it.
    gerenuk(tmp.path(), &tyf)
        .args(["--workspace", &pkg.display().to_string(), "doctor"])
        .assert()
        .success()
        .stdout(contains("sample_pkg"));
}

#[test]
fn a_missing_workspace_path_exits_with_code_two() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = common::fake_tyf(&tmp, SERVICE_OUTLINE, &[]);

    gerenuk(tmp.path(), &tyf)
        .args(["--workspace", "/definitely/not/here", "doctor"])
        .assert()
        .code(2)
        .stderr(contains("/definitely/not/here"));
}

#[test]
fn a_missing_tyf_binary_is_reported_not_ignored() {
    let pkg = sample_pkg();

    gerenuk(&pkg, std::path::Path::new("/nonexistent/tyf"))
        .args(["audit", "sample_pkg/service.py"])
        .assert()
        .code(2)
        .stderr(contains("tyf"));
}

#[test]
fn a_failing_tyf_surfaces_its_stderr() {
    let tmp = TempDir::new().expect("temp dir");
    let tyf = failing_tyf(&tmp, "ty server did not start");
    let pkg = sample_pkg();

    gerenuk(&pkg, &tyf)
        .args(["audit", "sample_pkg/service.py"])
        .assert()
        .code(2)
        .stderr(contains("ty server did not start"));
}

#[test]
fn audit_requires_at_least_one_file() {
    Command::cargo_bin("gerenuk")
        .expect("binary should build")
        .arg("audit")
        .assert()
        .failure()
        .stderr(contains("FILE"));
}
