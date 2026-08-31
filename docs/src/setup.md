# Setup

## Install

```sh
uv add --dev "gerenuk[ty]"        # from PyPI (a maturin-built wheel)
uvx --from "gerenuk[ty]" gerenuk audit pkg/x.py   # one-off, no install
```

From a `pip` world, `pip install "gerenuk[ty]"`.

## What gets installed, and why

`gerenuk` shells out to `tyf` (from
[ty-find](https://github.com/mojzis/ty-find)), which in turn drives
[`ty`](https://github.com/astral-sh/ty)'s language server.

| Package | How it arrives |
|---|---|
| `ty-find` | a **dependency** — `tyf` is what `gerenuk` drives, so it is never optional |
| `ty` | the **`[ty]` extra** — recommended, but see below |

`ty` is an extra rather than a dependency deliberately. It is a pre-1.0 type
checker that projects tend to pin for their own use, and a hard requirement in
`gerenuk` would compete with that pin for no benefit: when `tyf` finds no `ty`
on `PATH` it falls back to `uvx ty`. So `uv add --dev gerenuk` works on its own
if `uv` is around — the first call just pays a download.

Install the extra when you want the version pinned in your lockfile, or when
the machine running `gerenuk` has no network.

`changed-symbols` needs none of this: it uses `git` and nothing else.

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
- `gerenuk changed-symbols` — which Python symbols the working tree changed
- `gerenuk impacted-tests` — which tests those changed symbols can reach
- `gerenuk run -- -q` — run pytest on exactly those tests
- `gerenuk doctor` — check that `tyf` and the workspace resolve

Exit codes: `0` clean, `1` findings reported, `2` the run could not complete.
`changed-symbols` and `impacted-tests` never return `1`. When `impacted-tests`
cannot trust its answer it reports `"verdict": "run_all"` with a `reason` and
still exits `0` — read the verdict, not the exit code.

`run` is the exception: once pytest starts, the exit code is pytest's own. Use
`gerenuk run --dry-run` to see the decision and the exact argv without running
anything.

Findings are signals, not verdicts: dynamic dispatch, plugin registries, and
`__all__` re-exports can hide a real usage. Confirm with `tyf refs <symbol>`
before deleting anything.
```
<!-- END SHARED:claude-snippet -->
