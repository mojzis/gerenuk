# Tune gerenuk

The defaults are conservative: tune only once a `run_all` reason repeats.

**Where the diff comes from.** `--base <REF>` names the ref to diff against;
by default `origin/main`, then `main`, then `master`. The working tree -
staged, unstaged and untracked - is diffed from the merge-base.
`--workspace <PATH>` names the project root instead of walking up to it.

**Budgets.** The walk says `run_all` at the first limit it hits. A flag
beats `[tool.gerenuk]` in `pyproject.toml`, which beats the default.

| Flag | Key | Default | Meaning |
|---|---|---|---|
| `--max-depth <N>` | `max-depth` | 10 | levels from a changed symbol out to a test |
| `--max-symbols <N>` | `max-symbols` | 500 | symbols visited before giving up |
| `--budget-ms <MS>` | `budget-ms` | 30000 | wall clock for the walk; `0` disables it |

```toml
[tool.gerenuk]
max-depth = 20
suite-ms = 800
ignore-decorators = ["transformation", "celery.task"]
ignore-paths = ["**/*.md", "docs/**"]
collect-ignore = ["tests/data/**"]
docstrings = "code"
pytest-command = ["uv", "run", "pytest", "-o", "addopts="]
fallback-command = ["scripts/pick-subprojects.sh", "--from-gerenuk"]
```

- `suite-ms`: how long the whole suite takes, from pytest's summary line. A
  selection costs 1.5-2 s of `tyf` round-trips, so a suite declared at
  2000 ms or under makes `run` skip the walk and say `run_all` (`fast_suite`)
  whenever the diff would have needed one. `impacted-tests` ignores it.
- `ignore-decorators`: dotted names, suffix-matched syntactically, of
  decorators that register a function with a runner. A changed symbol
  carrying one is reported as ignored; import aliases are not resolved.
- `ignore-paths`: non-Python files that never force `run_all`, matched on
  the repo-relative path (`*`, `?`, `**`) and listed as `ignored_paths`; a
  `.py` file never is. Leave out anything a test reads, like checked docs.
- `collect-ignore`: test-tree files pytest never collects, such as sample
  data named `test_*.py`. Same syntax; `run` never hands one to pytest, even
  via a conftest or fixture, and lists it under `dropped`.
- `docstrings`: `ignore` (default) selects nothing for a docstring-only edit;
  `code` counts it, for code that reads `__doc__` (docopt, CLI help text).
- `pytest-command`: an argv, never a string, because the common value has
  arguments. Empty means `pytest` on `PATH`; `GERENUK_PYTEST` beats both.
  `-o addopts=` drops a coverage-carrying `addopts` for the hook run only.
- `fallback-command`: what `run` execs instead of the whole suite on
  `run_all`, given the reason in `GERENUK_FALLBACK_REASON` and the
  `changed-symbols` JSON on stdin; its exit code is the hook's. Overridden by
  `--fallback-command <JSON_ARRAY>`, then `GERENUK_FALLBACK`. Empty is an error.
- `git-env`: `isolate` (default) keeps a hook's `GIT_DIR`, `GIT_INDEX_FILE`
  and the like out of pytest's environment; `inherit` hands them on.
  `--git-env <POLICY>` beats it. The fallback always inherits.

**Binaries.** `GERENUK_TYF`, `GERENUK_GIT` and `GERENUK_PYTEST` skip `PATH`.
**Audit.** `gerenuk audit src/app.py` lists unreferenced or test-only symbols.

next: run `gerenuk run --dry-run`
