# Setup

## Install

```sh
uv add --dev "gerenuk[ty]"        # from PyPI (a maturin-built wheel)
uvx --from "gerenuk[ty]" gerenuk impacted-tests   # one-off, no install
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
`--workspace PATH` when you want to run against a project you are not standing
in.

## Telling coding agents about it

Paste this into your project's `CLAUDE.md`. It is deliberately two lines — the
exit codes, the `run_all` verdict and the static-reference caveat are all in
`gerenuk --help` and on this site, and an agent that needs them can read them
there:

<!-- BEGIN SHARED:claude-snippet -->
```markdown
### `gerenuk` — test selection and dead code

- `gerenuk run -- -q` runs only the tests the working tree's diff impacts
  (`--dry-run` to inspect; `gerenuk impacted-tests` explains why).
- `gerenuk audit <file>` confirms a symbol vulture flagged is really unused;
  its `only tests reach it` findings are ones vulture cannot produce.
```
<!-- END SHARED:claude-snippet -->
