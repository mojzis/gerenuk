# Tune gerenuk

Reference for the knobs. The defaults are conservative on purpose: tune
for speed only once a `run_all` reason keeps repeating.

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
- `pytest-command`: an argv, never a string, because the common value has
  arguments. Empty means `pytest` on `PATH`; `GERENUK_PYTEST` beats both.
  `-o addopts=` drops a coverage-carrying `addopts` for the hook run only.
- `fallback-command`: what `run` execs instead of the whole suite on
  `run_all`. It gets the reason in `GERENUK_FALLBACK_REASON`, the
  `changed-symbols` report as JSON on stdin, and its exit code is the hook's.
  `--fallback-command <JSON_ARRAY>` and `GERENUK_FALLBACK` override it, in
  that order. An empty array anywhere is an error at startup.
- `git-env`: `isolate` (the default) removes `GIT_DIR`, `GIT_INDEX_FILE` and
  the rest of what a hook exports from pytest's environment, so a test that
  creates a repository of its own never touches the one being committed;
  `inherit` keeps them. `--git-env <POLICY>` beats it. The fallback inherits.

**Binaries.** `GERENUK_TYF`, `GERENUK_GIT` and `GERENUK_PYTEST` each name one
executable and skip the `PATH` lookup.

**Audit.** `gerenuk audit src/app.py` reads the same graph backwards for the
files you name: symbols nothing references, and symbols only tests reach.
Exit `1` on findings.

next: run `gerenuk run --dry-run`
