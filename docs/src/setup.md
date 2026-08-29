# Setup

## Prerequisites

`gerenuk` shells out to `tyf`, which in turn drives `ty`'s language server.

```sh
uv add --dev ty ty-find
```

If `ty` is not on `PATH`, `tyf` falls back to `uvx ty`, so having `uv`
available is enough.

## Install

```sh
uv add --dev gerenuk       # from PyPI (a maturin-built wheel)
uvx gerenuk audit pkg/x.py # one-off, no install
```

From source:

```sh
git clone https://github.com/mojzis/gerenuk
cd gerenuk
cargo install --path .
```

## Verify

```sh
$ gerenuk doctor
workspace: /home/you/proj
tyf:       /home/you/proj/.venv/bin/tyf
```

If `tyf` lives somewhere unusual, point `GERENUK_TYF` at it:

```sh
GERENUK_TYF=/opt/bin/tyf gerenuk doctor
```

## Workspace detection

By default `gerenuk` walks up from the current directory until it finds a
`pyproject.toml`, `setup.py`, `setup.cfg`, or `.git`. Override it with
`--workspace PATH` when you want to audit a project you are not standing in.

## Telling coding agents about it

Paste this into your project's `CLAUDE.md`:

<!-- BEGIN SHARED:claude-snippet -->
```markdown
### Dead-symbol checks — `gerenuk`

This project has `gerenuk` — a symbol-level auditor built on `tyf` (ty-find).
Run it before deleting Python code, and after a refactor.

- `gerenuk audit pkg/module.py` — flag symbols nothing references, and symbols
  only tests reach
- `gerenuk audit --format json pkg/*.py` — same, machine-readable
- `gerenuk doctor` — check that `tyf` and the workspace resolve

Exit codes: `0` clean, `1` findings reported, `2` the run could not complete.

Findings are signals, not verdicts: dynamic dispatch, plugin registries, and
`__all__` re-exports can hide a real usage. Confirm with `tyf refs <symbol>`
before deleting anything.
```
<!-- END SHARED:claude-snippet -->
