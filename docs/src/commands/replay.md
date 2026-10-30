# `gerenuk replay`

Rebuild the test selection a commit's diff makes — to see what it ran and why,
and with `--run`, how long each of those tests took.

```
gerenuk replay <COMMIT> [--base <REF>]
               [--max-depth <N>] [--max-symbols <N>] [--budget-ms <MS>]
               [--run [--durations <N>]] [-- <pytest args>…]
```

This is for the day a hook took far longer than it should have. Two questions
come up then: *why were these tests selected*, and *which of them is slow*.
`replay` answers the first by default and the second with `--run`.

It needs what [`run`](run.md) needs: `git`, `tyf`, and pytest for `--run`.

## Where it runs

`ty` and pytest both read the working tree, so a selection rebuilt against any
other tree would be about other code. A replay therefore needs a checkout of
`COMMIT`, and gerenuk makes one when yours is not:

```
$ gerenuk replay 1a2b3c4
gerenuk: the checkout is not 1a2b3c4d5e6f, so it is replayed in /repo/.git/gerenuk/replay — kept for the next replay; `git worktree remove --force /repo/.git/gerenuk/replay` drops it
gerenuk: replaying 1a2b3c4d5e6f against 1a2b3c4^ (0f9e8d7c6b5a)
…
```

Your own checkout — branch, index, uncommitted edits — is not touched. The
worktree is detached and kept: the first replay pays for a full checkout, and
each later one only rewrites the files that differ, so checking a handful of
commits in a row is cheap. Untracked files left in it are removed between
replays; ignored ones (a `.venv`, caches) stay.

If your checkout already matches `COMMIT` — `HEAD` need not be `COMMIT`, only
its tracked files have to be — it is used as it is and no worktree is made.
Untracked files are ignored either way; they were never part of the commit.
`COMMIT` and `--base` are resolved in your checkout, so `HEAD~1` means what it
means where you typed it.

Under `--run`, pytest starts in the worktree with whatever pytest gerenuk
resolves (see [`run`](run.md)). A virtualenv with the project installed
editable points at your main checkout, so a `src/` layout may import that code
instead of the commit's; a `pytest-command` like `uv run pytest` builds an
environment for the worktree instead, once.

## Which diff

| Invocation | Diff | Use it for |
|---|---|---|
| `replay <COMMIT>` | `COMMIT^` → `COMMIT` | that commit's own change |
| `replay <COMMIT> --base main` | `merge-base(COMMIT, main)` → `COMMIT` | what a pre-commit hook on a branch forked from `main` saw when it made `COMMIT` |

The hook diffs the working tree against the merge base with `origin/main` (see
[`changed-symbols`](changed-symbols.md)), so a commit late in a branch was
judged on the whole branch so far. `--base` reproduces that; the default looks
at the one commit only. A root commit has no parent and needs `--base`.

## Listing (the default)

Prints exactly what [`run --dry-run`](run.md#--dry-run) prints — the decision,
every selected node id with the chain that reached it, and the argv — and
spawns nothing:

```
$ gerenuk replay 1a2b3c4
gerenuk: replaying 1a2b3c4d5e6f against 1a2b3c4^ (0f9e8d7c6b5a)
decision: selected
gerenuk: 5 node id(s) from 1 origin(s) in 41 ms — details: gerenuk impacted-tests

tests/test_api.py
  ← sample_pkg.cli ← sample_pkg.cli:main ← sample_pkg.service:describe
tests/test_fixtures.py::test_described_marks_the_senior
  ← tests.conftest:described ← sample_pkg.service:describe
…
```

`--format json` is the dry run's JSON. A `run_all` decision names its reason,
which is often the whole answer to "why did that take so long".

## `--run`: with timings

Runs the selection under pytest with `--durations=N` added, so pytest ends with
its table of the slowest test phases — setup, call and teardown separately,
which is how a slow fixture shows up:

```
$ gerenuk replay 1a2b3c4 --run --durations 10
gerenuk: replaying 1a2b3c4d5e6f against 1a2b3c4^ (0f9e8d7c6b5a)
gerenuk: 5 node id(s) from 1 origin(s) in 41 ms — details: gerenuk impacted-tests
...
============================= slowest 10 durations =============================
12.41s call     tests/test_api.py::test_full_export
 3.02s setup    tests/test_service.py::test_lookup
…
```

`--durations` defaults to `0`, every phase; pytest hides those under 5 ms
unless you pass `-- -vv`. The flag goes in before the passthrough, so a
`--durations` of your own after `--` wins.

Everything else is [`run`](run.md)'s: gerenuk becomes pytest and the exit code
is pytest's, an empty selection spawns nothing, and a `run_all` decision runs
the full suite — or the configured [fallback
command](run.md#the-fallback-command), which gets no pytest arguments and so no
timings. `suite-ms`, `git-env` and the other `[tool.gerenuk]` keys apply as
they do to `run`.

## Not to be confused with

[`run --impact`](run.md#replaying-a-saved-report) and [`impacted-tests
--changed`](impacted-tests.md#replaying-a-saved-report) replay a saved *report*.
`replay` recomputes everything from a *commit*, against today's gerenuk and
`ty` — so a selection that differs from the one the hook made at the time is
itself a finding.
