# Tune gerenuk

Reference for the knobs. The defaults are conservative on purpose: every
degrade widens the selection and never narrows it, so tune for speed only
once a `run_all` reason keeps repeating.

**Where the diff comes from.** `--base <REF>` names the ref to diff against;
by default `origin/main`, then `main`, then `master`, whichever exists first.
The diff is taken from the merge-base, so commits already on the base do not
count, and it is the working tree that is diffed - staged, unstaged and
untracked alike. `--workspace <PATH>` names the project root instead of
walking up from the current directory to the nearest `pyproject.toml`,
`setup.py`, `setup.cfg` or `.git`.

**Budgets.** The walk stops and says `run_all` at whichever limit it hits
first. A flag beats `[tool.gerenuk]` in `pyproject.toml`, which beats the
built-in default.

| Flag | Key | Default | Meaning |
|---|---|---|---|
| `--max-depth <N>` | `max-depth` | 10 | levels from a changed symbol out to a test |
| `--max-symbols <N>` | `max-symbols` | 500 | symbols visited before giving up |
| `--budget-ms <MS>` | `budget-ms` | 30000 | wall clock for the walk; `0` disables it |

```toml
[tool.gerenuk]
max-depth = 20
ignore-decorators = ["transformation", "celery.task"]
pytest-command = ["uv", "run", "pytest"]
fallback-command = ["scripts/pick-subprojects.sh", "--from-gerenuk"]
```

- `ignore-decorators`: dotted names, suffix-matched syntactically, of
  decorators that register a function with a runner. A changed symbol
  carrying one is reported as ignored instead of walked; import aliases are
  not resolved.
- `pytest-command`: an argv, never a string, because the common value has
  arguments. Empty means `pytest` on `PATH`; `GERENUK_PYTEST` beats both.
- `fallback-command`: what `run` execs instead of the whole suite on
  `run_all`. It receives the reason in `GERENUK_FALLBACK_REASON` and the
  `changed-symbols` report as JSON on stdin, and its exit code becomes the
  hook's. `--fallback-command <JSON_ARRAY>` and `GERENUK_FALLBACK` override
  it, in that order. An empty array anywhere is an error at startup.

**Binaries.** `GERENUK_TYF`, `GERENUK_GIT` and `GERENUK_PYTEST` each name one
executable and skip the `PATH` lookup. `tyf` is looked for only once a walk
is actually needed, so `changed-symbols` and a `run_all` settled by the diff
alone work in a checkout with no `ty` at all.

**Reading a walk.** `gerenuk changed-symbols` is the first stage on its own:
the symbols the diff changed, from `git` alone. `gerenuk impacted-tests`
adds the walk, with `--changed <FILE>` to replay a saved first stage.
`--format json` on either is the schema the next stage reads.

**Audit.** `gerenuk audit src/app.py` reads the same reference graph
backwards for the files you name: symbols nothing references, and symbols
only tests reach. Exit `1` on findings. It is a verifier for a candidate
something else flagged, not a sweep.

next: run `gerenuk run --dry-run`
