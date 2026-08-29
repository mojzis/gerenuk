# `gerenuk doctor`

Check that the workspace and the `tyf` binary resolve, without running an
analysis.

```
$ gerenuk doctor
workspace: /home/you/proj
tyf:       /home/you/proj/.venv/bin/tyf
```

Exits `0` when both resolve and `2` when either does not — so it works as a
cheap preflight step in CI before the real audit runs.

## What it checks

1. **The workspace root.** Either the `--workspace` path (which must exist), or
   the nearest ancestor of the current directory holding a `pyproject.toml`,
   `setup.py`, `setup.cfg`, or `.git`.
2. **The `tyf` binary.** `GERENUK_TYF` if set, otherwise the first `tyf` on
   `PATH`.

It does **not** start `ty` or make an LSP request. A green `doctor` means
`gerenuk` knows where to look; it does not prove the language server will come
up. If `doctor` passes but `audit` fails, the problem is downstream — see
[Troubleshooting](../troubleshooting.md).
