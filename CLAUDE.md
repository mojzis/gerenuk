# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with
code in this repository.

## Project Overview

`gerenuk` is a CLI that audits Python symbols by driving `tyf` (from
[ty-find](https://github.com/mojzis/ty-find)). It is a hybrid Rust/Python
project: a Rust binary packaged as a Python wheel via maturin.

It reports two things per file: symbols nothing references, and symbols only
tests reach. Architecture details: `docs/dev/ARCHITECTURE.md`.

## Prerequisites

- **`tyf`** on `PATH` for real runs (`uv add --dev ty ty-find`). Not needed for
  the test suite — the tests stub it.
- **`uv`** for the fixture package's pytest suite.

## Common Commands

```sh
# Pre-commit checks (always run before committing)
cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features

make review        # the above, plus the fixture pytest suite, audit and deny
make review-quick  # skip the network checks
make test-fixture  # just the Python fixture package's pytest suite
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

- **`tyf::Runner::run` is the only function that spawns a process.** Everything
  downstream takes parsed data. Keep it that way — it is what makes `analyze`
  and `report` testable without `tyf` or `ty` installed.
- **`main.rs` stays thin.** Argument parsing, tracing setup, exit-code mapping.
  Command bodies belong in `cli.rs`.
- **Exit codes are part of the contract.** `0` clean, `1` findings, `2` the run
  could not complete. Do not collapse `1` and `2`.
- **The fixture package and the canned payloads must agree.** Changing the call
  graph in `tests/fixtures/sample_pkg` means updating both its pytest suite and
  the stub payloads in `tests/audit.rs`.
- **`clippy::unwrap_used` / `expect_used` are denied outside tests.** Use
  `anyhow::Context` on every `?` that crosses an I/O or parsing boundary.
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
