#!/usr/bin/env bash
# Run `gerenuk impacted-tests` against the fixture package with a *real* `tyf`.
#
# `cargo test` stubs `tyf`, which keeps the suite hermetic but means the closure
# loop's contract with real LSP output is never exercised. This script closes
# that gap, outside `cargo test` so a missing `ty` cannot break the build.
#
# It copies the fixture into a scratch repository — the fixture lives inside
# gerenuk's own checkout, so running in place would diff gerenuk, not it.
#
# The scratch repository lives at a *stable* path under target/ rather than in
# mktemp: `ty`'s daemon registers a workspace per directory and keeps it after
# the directory is gone, so a fresh temp path every run makes the second run
# fail with "Failed to create LSP client".
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$repo_root/tests/fixtures/sample_pkg"

if ! command -v tyf > /dev/null 2>&1; then
  echo "⚠️  tyf not on PATH; skipping. Install with: uv add --dev ty ty-find"
  exit 0
fi

gerenuk="$repo_root/target/debug/gerenuk"
[[ -x "$gerenuk" ]] || { echo "build gerenuk first: cargo build"; exit 1; }

work="$repo_root/target/impact-smoke"
rm -rf "$work"
mkdir -p "$work"

# .venv is a build artefact of `make test-fixture`, not part of the fixture.
tar -C "$fixture" --exclude=.venv --exclude=__pycache__ -cf - . | tar -C "$work" -xf -

git -C "$work" -c init.defaultBranch=main init -q
git -C "$work" -c user.email=smoke@example.com -c user.name=Smoke \
    -c commit.gpgsign=false add -A
git -C "$work" -c user.email=smoke@example.com -c user.name=Smoke \
    -c commit.gpgsign=false commit -qm base

# Change `describe`, which `Enricher.run` calls and `enrich_endpoint` calls in
# turn. `-i.bak` is the one in-place form both GNU and BSD sed accept.
sed -i.bak 's/ — senior/ — SENIOR/' "$work/sample_pkg/service.py"
rm -f "$work/sample_pkg/service.py.bak"

echo "🎯 impacted-tests against the fixture, with real tyf:"
output="$("$gerenuk" --workspace "$work" impacted-tests)"
echo "$output"

# Assert on the *chain*, not just the filename. `tests/test_service.py` and
# `tests/test_api.py` are both selected wholesale through the `sample_pkg.cli`
# module-level edge, which needs no `tyf` at all — so matching those names would
# still pass if the position query broke. The per-test entry below is the one
# that can only exist because a real `tyf refs file:line:col` resolved.
fail=0
expect() {
  grep -qF "$1" <<< "$output" || { echo "❌ missing: $1"; fail=1; }
}

expect "tests/test_pipelines.py::test_run_describes_animals_in_order"
expect "← sample_pkg.pipelines:Enricher.run ← sample_pkg.service:describe"
expect "tests/test_service.py"
expect "tests/test_api.py"
expect "verdict selected"

if [[ "$fail" -eq 0 ]]; then
  echo "✅ real tyf resolved the position query and the walk reached its tests"
fi
exit "$fail"
