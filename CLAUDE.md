# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with
code in this repository.

## Project Overview

`gerenuk` is a CLI that audits Python symbols by driving `tyf` (from
[ty-find](https://github.com/mojzis/ty-find)). It is a hybrid Rust/Python
project: a Rust binary packaged as a Python wheel via maturin.

It reports two things per file: symbols nothing references, and symbols only
tests reach. `changed-symbols` maps the working tree's git diff to the Python
symbols it changed — the one part of the crate that needs `git` rather than
`tyf`. `impacted-tests` walks the reverse reference graph from those symbols out
to the tests that reach them. Together they are impact-based pytest selection,
minus the pytest invocation (phase 3).

Architecture details: `docs/dev/ARCHITECTURE.md`. Decisions and their costs:
`docs/adr/`.

## Prerequisites

- **`tyf`** on `PATH` for `audit`/`doctor` (`uv add --dev ty ty-find`). Not
  needed for the test suite — the tests stub it.
- **`git`** on `PATH` for `changed-symbols` and its tests, or `GERENUK_GIT`
  pointing at it. That is all `changed-symbols` needs; `impacted-tests` needs
  `tyf` as well.
- **`uv`** for the fixture package's pytest suite.

## Common Commands

```sh
# Pre-commit checks (always run before committing)
cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features

make review        # the above, plus the fixture pytest suite, audit and deny
make review-quick  # skip the network checks
make test-fixture  # just the Python fixture package's pytest suite
make test-impact   # impacted-tests against the fixture with a REAL tyf
make docs          # build the mdBook site + llms.txt
```

If formatting fails, fix it with `cargo fmt --all` and re-run.

## Development Workflow

All features and bug fixes follow TDD (red-green-refactor). No implementation
code without a failing test first. Bug fixes must include a regression test that
fails without the fix.

## Test Changes Require Deliberation

When a test fails during implementation:

1. **Stop and diagnose.** Understand WHY it fails before changing anything. Is
   the test wrong, or is the implementation wrong?
2. **Default assumption: the test is right.** Fix the implementation first.
3. **If the test genuinely needs updating** (requirements changed, API evolved),
   explain what changed and why the old assertion is no longer correct before
   modifying it.
4. **Never weaken an assertion just to make it pass.** Making a test more
   permissive without understanding the failure is not a fix.
5. **If uncertain, ask.** A 2-line question is cheaper than a silent wrong
   decision.

## Key Invariants

- **`tyf` and `git` are the only two modules that spawn a process.** The literal
  spawn sites are `tyf::Runner::run` and the private `git::Git::output`, which
  `git::Git::run` and `git::Git::try_run` are the only ways into. Everything
  downstream takes parsed data. Keep it that way — it is what makes `analyze`,
  `changed`, `closure` and `report` testable with no `tyf`, no `ty`, no `git`
  and no Python. Adding a third seam needs a very good reason (ADR 0001).
- **`changed-symbols` must never construct a `tyf::Runner`.** Discovery happens
  inside the `Audit` and `Doctor` arms of `Cli::run`, not before the match. A
  test in `tests/changed_symbols.rs` runs with an empty `PATH` to enforce this.
- **`changed::Sources` is the seam the classification is tested through.** New
  inputs go behind that trait, not into `changed::analyze` directly, or the unit
  tests stop being able to run without a repository.
- **`main.rs` stays thin.** Argument parsing, tracing setup, exit-code mapping.
  Command bodies belong in `cli.rs`.
- **Exit codes are part of the contract.** `0` clean, `1` findings, `2` the run
  could not complete. Do not collapse `1` and `2`. `changed-symbols` is an
  inventory rather than a verdict, so it returns `0` or `2` and never `1`.
- **The fixture package and the canned payloads must agree.** Changing the call
  graph in `tests/fixtures/sample_pkg` means updating both its pytest suite and
  the stub payloads in `tests/audit.rs`.
- **`closure.rs` reaches the world only through `Refs` and `Index`.** New inputs
  go behind one of those traits, not into `walk` directly, or the unit tests
  stop being able to run without `tyf`. `impact.rs` is glue and holds no rules.
- **`impacted-tests` never fails a run it could answer.** Anything that goes
  wrong past the first gate — `tyf` missing, `tyf` garbling its output, an
  unreadable file — degrades to `verdict: run_all` and exit `0`. The one
  exception is a run with no answer at all (not a repository, unreadable
  `--changed`), which is exit `2`. There is no exit `1` (ADR 0009).
- **The up-front verdicts are checked before `tyf` is looked for.** Reordering
  `run_impacted_tests` so discovery happens first would break
  `impacted-tests` on a `pyproject.toml`-only diff in a checkout with no `ty`.
  `tests/impacted_tests.rs` runs those cases with an empty `PATH` to enforce it.
- **A symbol id with no colon is a module.** That convention carries the
  module-level edge and the whole-file test selection (`"symbol": null`) through
  `via`, `origin` and `impacted_tests[].symbol` alike (ADR 0008).
- **`clippy::unwrap_used` / `expect_used` warn outside tests, and the documented
  `cargo clippy … -D warnings` makes them fatal.** Use
  `anyhow::Context` on every `?` that crosses an I/O or parsing boundary.
- **A symbol's `added`/`modified`/`deleted` verdict comes from whether it exists
  on each side of the diff, not from which side the hunk touched.** Deleting
  lines from a function that still exists is a modification; its callers were
  never orphaned. Renames are deliberately *not* paired — a moved module is a
  new module, so the old path's symbols are `deleted` and the new path's
  `added`.
- **Do not trust `tyf`'s production/test split.** Its heuristic reads the whole
  absolute path, so a project under a `tests/` directory has every reference
  filed as a test. `analyze::split_refs` re-derives the buckets from paths
  relative to the workspace root, and drops the symbol's own definition. The
  stub payloads in `tests/audit.rs` reproduce both quirks — keep them that way.

## Docs

The mdBook site under `docs/` deploys to GitHub Pages on push to `main`
(`.github/workflows/docs.yml`), along with `llms.txt` and `llms-full.txt`.

- Shared prose lives in `docs/shared/` and is injected into `README.md` and
  `docs/src/setup.md` by `docs/inject-shared.sh`. Edit the shared file, then run
  the script — never edit between the `<!-- BEGIN SHARED:... -->` markers.
- `docs/gen-version.sh` syncs the docs version badge with `Cargo.toml`.
- Adding a page means adding it to `docs/src/SUMMARY.md`; the `llms.txt`
  generator reads that file.
