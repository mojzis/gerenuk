---
title: Run the full suite on an unborn HEAD instead of erroring
pitch: In a fresh repo (no commits yet), `gerenuk run` falls back to running every test instead of exiting 2.
scenario: small
---

# Run the full suite on an unborn HEAD

Source: https://github.com/mojzis/gerenuk/issues/31

## Problem

In a repo with no commits yet (unborn `HEAD`), there is nothing to diff against.
`gerenuk run` exits 2 with
`no usable base ref (tried origin/main, main, master). Pass --base with a ref that exists.`
As a madoqua pre-commit check this blocks the first commit of every project made
from python-template; the only way past is `MADOQUA_SKIP=gerenuk`.

The base is picked in `src/git.rs` (`resolve_base`, `resolve_last_commit`).
`resolve_last_commit` already falls back to the `DEFAULT_BASES` chain for a root
commit, and that chain is what fails when `HEAD` itself does not exist.

## Requirements

- **R1** When `HEAD` is unborn (no commit exists), `gerenuk run` does not error: it
  takes the `run_all` outcome (the full suite, or `fallback-command` if configured),
  exits with pytest's own status, and the report gives a reason that names the
  unborn `HEAD` / first commit. Holds for the default scope and for
  `--scope last-commit` and `--scope branch`.
- **R2** `gerenuk impacted-tests` on an unborn `HEAD` likewise reports `run_all` with
  the same reason instead of erroring.
- **R3** When the base cannot be resolved for another reason, the error message
  matches the scope in use: with `--scope last-commit` it does not list only the
  `branch`-scope fallbacks (`origin/main, main, master`) as if those were the cause.
- **R4** An explicit `--base <ref>` that does not exist is still an error (no silent
  `run_all`), as today.

## Acceptance check (the issue's repro, gerenuk 0.5.5)

```sh
git init -b main && mkdir pkg tests && touch pkg/__init__.py
printf 'def test_a():\n    assert True\n' > tests/test_a.py
printf '[project]\nname="pkg"\nversion="0"\n' > pyproject.toml
git add -A
gerenuk run -- -q                        # expect: exit 0, test_a ran, verdict run_all
gerenuk run --scope last-commit -- -q    # expect: same
```

Cover this with an integration test in `tests/` that builds such a repo (the
existing tests that commit through a real pre-commit hook show how).

## No-gos

- Do not change behaviour for repos that have at least one commit.
- Do not change how `replay` picks its base.
- No new CLI flags or config keys.
- Keep `git` as the only module that spawns git (see CLAUDE.md).

## Appetite

Small: one phase. Touch `src/git.rs`, the call sites that turn a base error into
an exit, the reason text, tests, and the guide/docs page that lists `run_all`
reasons (`src/guide.rs` test requires every reason to be named there).
