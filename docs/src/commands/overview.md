# Commands

| Command | What it does |
|---|---|
| [`audit`](audit.md) | Report unreferenced and test-only symbols in one or more files |
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
