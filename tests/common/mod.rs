//! Shared helpers for the integration tests.
//!
//! The tests avoid depending on a real `tyf` (which would drag in `ty`, a
//! Python environment and an LSP handshake). Instead [`fake_tyf`] writes a tiny
//! script that answers the three `tyf` sub-commands gerenuk uses with canned
//! JSON, and `GERENUK_TYF` points the binary at it.

#![allow(dead_code, reason = "each integration test file uses a different subset")]
#![allow(
    clippy::expect_used,
    reason = "these are test helpers; a failed setup step should abort loudly"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::CommandCargoExt;
use tempfile::TempDir;

/// Path to the checked-in Python fixture package.
pub fn sample_pkg() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_pkg")
}

/// A `gerenuk` command with `GERENUK_TYF` pointed at `tyf`, run from `cwd`.
pub fn gerenuk(cwd: &Path, tyf: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gerenuk").expect("gerenuk binary should be built for tests");
    cmd.current_dir(cwd).env("GERENUK_TYF", tyf);
    cmd
}

/// A `gerenuk` command with no `tyf` configured at all, run from `cwd`.
///
/// `changed-symbols` must never reach for `tyf`, so its tests deliberately
/// leave the environment variable unset.
pub fn gerenuk_bare(cwd: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gerenuk").expect("gerenuk binary should be built for tests");
    cmd.current_dir(cwd).env_remove("GERENUK_TYF");
    cmd
}

/// A throwaway git repository, isolated from the developer's git configuration.
///
/// Signing keys, hooks and a different `init.defaultBranch` would all make
/// these tests fail on someone else's machine, so the global and system config
/// files are pointed at `/dev/null`.
pub struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    /// An initialised repository on branch `main`, with no commits yet.
    pub fn new() -> Self {
        let repo = Self { dir: TempDir::new().expect("temp dir") };
        repo.git(&["init", "--initial-branch=main"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "user.name", "Test"]);
        repo
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(self.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .args(args)
            .output()
            .expect("git should be on PATH for the test suite");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub fn write(&self, rel: &str, body: &str) {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write file");
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path().join(rel)).expect("remove file");
    }

    pub fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message]);
    }

    /// Run `gerenuk changed-symbols --format json` and parse the result.
    pub fn changed_symbols(&self, extra: &[&str]) -> serde_json::Value {
        let output = gerenuk_bare(self.path())
            .args(["--format", "json", "changed-symbols"])
            .args(extra)
            .output()
            .expect("gerenuk should run");
        assert!(
            output.status.success(),
            "changed-symbols failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("the output should be valid JSON")
    }
}

impl Default for TestRepo {
    fn default() -> Self {
        Self::new()
    }
}

/// Write an executable stub that stands in for `tyf`.
///
/// The script ignores `--format json` (gerenuk always passes it) and dispatches
/// on the sub-command. `outline` answers `list`, and `refs_for` maps a symbol
/// name to a `tyf refs` payload; unknown symbols get an empty result.
///
/// Returns the script path; it lives inside `dir`, so keep the `TempDir` alive.
pub fn fake_tyf(dir: &TempDir, outline: &str, refs_for: &[(&str, &str)]) -> PathBuf {
    let mut cases = String::new();
    for (symbol, payload) in refs_for {
        let _ = write!(cases, "    {symbol}) cat <<'JSON'\n{payload}\nJSON\n      ;;\n");
    }

    let script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

# Drop the leading `--format json` that gerenuk always sends.
while [[ "${{1:-}}" == --* ]]; do
  shift 2
done

cmd="${{1:-}}"
shift || true

case "$cmd" in
  list)
    echo "Document outline for $1:"
    cat <<'JSON'
{outline}
JSON
    ;;
  refs)
    case "${{1:-}}" in
{cases}    *) printf '{{"symbol": "%s", "reference_count": 0, "references": [], "test_reference_count": 0, "test_references": []}}\n' "${{1:-}}" ;;
    esac
    ;;
  *)
    echo "fake tyf: unsupported command '$cmd'" >&2
    exit 1
    ;;
esac
"#
    );

    let path = dir.path().join("fake-tyf");
    std::fs::write(&path, script).expect("write fake tyf script");
    make_executable(&path);
    path
}

/// A stub that always fails, for exercising gerenuk's error path.
pub fn failing_tyf(dir: &TempDir, message: &str) -> PathBuf {
    let path = dir.path().join("failing-tyf");
    std::fs::write(&path, format!("#!/usr/bin/env bash\necho '{message}' >&2\nexit 3\n"))
        .expect("write failing tyf script");
    make_executable(&path);
    path
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).expect("stat stub").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod stub");
    }
}

/// Outline matching `sample_pkg/service.py`: one function, one class with three
/// methods, and a second function.
pub const SERVICE_OUTLINE: &str = r#"[
  {"name": "describe", "kind": 12,
   "range": {"start": {"line": 15, "character": 0}, "end": {"line": 18, "character": 48}},
   "selectionRange": {"start": {"line": 15, "character": 4}, "end": {"line": 15, "character": 12}},
   "children": []},
  {"name": "ShelterService", "kind": 5,
   "range": {"start": {"line": 21, "character": 0}, "end": {"line": 39, "character": 60}},
   "selectionRange": {"start": {"line": 21, "character": 6}, "end": {"line": 21, "character": 20}},
   "children": [
     {"name": "__init__", "kind": 6,
      "range": {"start": {"line": 24, "character": 4}, "end": {"line": 25, "character": 31}},
      "selectionRange": {"start": {"line": 24, "character": 8}, "end": {"line": 24, "character": 16}},
      "children": []},
     {"name": "summary", "kind": 6,
      "range": {"start": {"line": 27, "character": 4}, "end": {"line": 31, "character": 70}},
      "selectionRange": {"start": {"line": 27, "character": 8}, "end": {"line": 27, "character": 15}},
      "children": []},
     {"name": "seniors", "kind": 6,
      "range": {"start": {"line": 33, "character": 4}, "end": {"line": 35, "character": 60}},
      "selectionRange": {"start": {"line": 33, "character": 8}, "end": {"line": 33, "character": 15}},
      "children": []},
     {"name": "_sorted_by_age", "kind": 6,
      "range": {"start": {"line": 37, "character": 4}, "end": {"line": 39, "character": 60}},
      "selectionRange": {"start": {"line": 37, "character": 8}, "end": {"line": 37, "character": 22}},
      "children": []}
   ]},
  {"name": "legacy_export", "kind": 12,
   "range": {"start": {"line": 42, "character": 0}, "end": {"line": 45, "character": 25}},
   "selectionRange": {"start": {"line": 42, "character": 4}, "end": {"line": 42, "character": 17}},
   "children": []}
]"#;
