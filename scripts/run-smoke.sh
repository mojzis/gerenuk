#!/usr/bin/env bash
# Run `gerenuk run` against the fixture package with a *real* `tyf` and a *real*
# pytest.
#
# `cargo test` stubs both, which keeps the suite hermetic but means the full
# pipeline — diff, walk, node-id mapping, fixture expansion, pytest — is never
# exercised end to end. This closes that gap, outside `cargo test` so a missing
# `ty` cannot break the build.
#
# Like `impact-smoke.sh` it copies the fixture into a scratch repository at a
# *stable* path under target/: `ty`'s daemon registers a workspace per directory
# and keeps it after the directory is gone, so a fresh temp path every run makes
# the second run fail with "Failed to create LSP client".
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$repo_root/tests/fixtures/sample_pkg"

if ! command -v tyf > /dev/null 2>&1; then
  echo "⚠️  tyf not on PATH; skipping. Install with: uv add --dev ty ty-find"
  exit 0
fi
if ! command -v uv > /dev/null 2>&1; then
  echo "⚠️  uv not on PATH; skipping. The fixture's pytest runs under uv."
  exit 0
fi

gerenuk="$repo_root/target/debug/gerenuk"
[[ -x "$gerenuk" ]] || { echo "build gerenuk first: cargo build"; exit 1; }

# The stable scratch path is what keeps `ty`'s daemon happy across runs, but it
# also means the daemon holds an index of the *previous* run's copy. Editing the
# fixture then makes it answer with stale line numbers, which is indistinguishable
# from gerenuk mapping the wrong symbol. Restart it rather than debug that twice.
tyf daemon restart > /dev/null 2>&1 || true

work="$repo_root/target/run-smoke"
rm -rf "$work"
mkdir -p "$work"

# .venv is a build artefact of `make test-fixture`, not part of the fixture.
tar -C "$fixture" --exclude=.venv --exclude=__pycache__ -cf - . | tar -C "$work" -xf -

# `pytest-command` has to name a runner that can import the package. `uv run`
# with the fixture's own dependencies is the one that works from a scratch copy.
cat >> "$work/pyproject.toml" << 'TOML'
pytest-command = ["uv", "run", "--with", "pytest", "--with", "hatchling", "python", "-m", "pytest"]
TOML

git -C "$work" -c init.defaultBranch=main init -q
git -C "$work" -c user.email=smoke@example.com -c user.name=Smoke \
    -c commit.gpgsign=false add -A
git -C "$work" -c user.email=smoke@example.com -c user.name=Smoke \
    -c commit.gpgsign=false commit -qm base

# Change `describe`. Two routes lead out of it: references `tyf` can resolve,
# and the `described` fixture in tests/conftest.py, which it cannot.
#
# The edit is deliberately behaviour-preserving, because this script goes on to
# run the selected tests for real — a red suite here has to mean the selection
# is wrong, not that the fixture's assertions were sabotaged.
sed -i.bak 's/^    suffix = /    suffix: str = /' "$work/sample_pkg/service.py"
rm -f "$work/sample_pkg/service.py.bak"
grep -q 'suffix: str = ' "$work/sample_pkg/service.py" ||
  { echo "❌ the fixture edit did not apply; service.py has changed shape"; exit 1; }

echo "🎯 gerenuk run --dry-run against the fixture, with real tyf:"
output="$("$gerenuk" --workspace "$work" run --dry-run)"
echo "$output"

fail=0
expect() {
  grep -qF "$1" <<< "$output" || { echo "❌ missing: $1"; fail=1; }
}

expect "decision: selected"
# Resolved by `tyf refs`, then trimmed to a node id pytest will accept.
expect "tests/test_pipelines.py::test_run_describes_animals_in_order"
# Reachable *only* through the fixture: nothing in test_fixtures.py mentions
# `describe`, so this line is the whole point of phase 3.
expect "tests/test_fixtures.py::test_described_marks_the_senior"
expect "← tests.conftest:described ← sample_pkg.service:describe"
# The configured multi-word runner, one element per line.
expect "  uv"

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo ""
echo "🎯 the same selection, actually run under pytest:"
# The change is cosmetic in a way the fixture's assertions do not check, so a
# green suite is the expected outcome; a red one means the selection is wrong.
if "$gerenuk" --workspace "$work" run -- -q; then
  echo "✅ the full pipeline selected its tests and pytest ran them green"
else
  echo "❌ pytest exited non-zero; the selection or the fixture is wrong"
  exit 1
fi
