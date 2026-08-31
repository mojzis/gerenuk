# Commands

| Command | What it does |
|---|---|
| [`audit`](audit.md) | Report unreferenced and test-only symbols in one or more files |
| [`changed-symbols`](changed-symbols.md) | Map the working tree's diff to the Python symbols it changed |
| [`impacted-tests`](impacted-tests.md) | Walk from those changed symbols to the tests that reach them |
| [`run`](run.md) | Run pytest on exactly those tests |
| [`doctor`](doctor.md) | Show the resolved workspace and `tyf` binary, then exit |

## Global flags

| Flag | Default | Meaning |
|---|---|---|
| `--workspace PATH` | auto-detect | Project root to resolve symbols against |
| `--format human\|json` | `human` | Output shape |
| `-v`, `--verbose` | off | Debug logging to stderr (`RUST_LOG` wins if set) |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | The run completed and nothing was flagged |
| `1` | The run completed and findings were reported |
| `2` | The run could not complete — `tyf` missing, bad workspace, malformed output |

The split between `1` and `2` is what makes `gerenuk` usable in CI: a failing
check and a broken setup are different problems.

`changed-symbols` and `impacted-tests` never return `1`: their output is an
inventory, not a verdict on one. When `impacted-tests` cannot trust its own
answer it says so in the report (`verdict: run_all`) and still exits `0` —
"run everything" is a usable answer for a pre-commit hook, and failing the hook
because the analysis was inconclusive only teaches people to bypass it.

[`run`](run.md) is the exception to the table. Its own failures exit `2` as
usual, but once pytest starts the exit code is **pytest's**, verbatim — that is
the hook contract. A selection that turns out to impact no tests exits `0`
without spawning anything.

## Prerequisites per command

`audit` and `doctor` need `tyf` on `PATH`. `changed-symbols` needs only `git` —
it parses Python with `tree-sitter`, so it works in a checkout that has never
had `ty` installed. `impacted-tests` needs both, but looks for `tyf` only once
it knows the walk will actually run. `run` needs those two plus pytest, which it
resolves from `GERENUK_PYTEST`, then `pytest-command` in `pyproject.toml`, then
`PATH`.
