### Dead-symbol checks — `gerenuk`

This project has `gerenuk` — a symbol-level auditor built on `tyf` (ty-find).
Run it before deleting Python code, and after a refactor.

- `gerenuk audit pkg/module.py` — flag symbols nothing references, and symbols
  only tests reach
- `gerenuk audit --format json pkg/*.py` — same, machine-readable
- `gerenuk changed-symbols` — which Python symbols the working tree changed
- `gerenuk impacted-tests` — which tests those changed symbols can reach
- `gerenuk doctor` — check that `tyf` and the workspace resolve

Exit codes: `0` clean, `1` findings reported, `2` the run could not complete.
`changed-symbols` and `impacted-tests` never return `1`. When `impacted-tests`
cannot trust its answer it reports `"verdict": "run_all"` with a `reason` and
still exits `0` — read the verdict, not the exit code.

Findings are signals, not verdicts: dynamic dispatch, plugin registries, and
`__all__` re-exports can hide a real usage. Confirm with `tyf refs <symbol>`
before deleting anything.
