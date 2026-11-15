# 0025 — An unborn HEAD runs everything

**Status:** accepted, amends [0024](0024-the-default-diff-is-the-last-commit.md)

## Decision

When `HEAD` names a branch with no commit yet, `gerenuk run` and
`gerenuk impacted-tests` answer `run_all` with the reason `unborn_head` (label
`HEAD has no commit yet`), in every scope. `run` then execs the bare suite, or
the `fallback-command`, and exits with its code.

The check runs only when `--base` is absent, before the scope is looked at and
before anything is diffed, listed or looked up: no `tyf`, no `git diff`, no
`ls-files`. The report carries `base: "HEAD"`, an empty `merge_base` and no
lists; there is no `ChangedSymbols`, so the fallback payload's `report` is
`null`. The payload version stays `1`: a new reason is a new variant.

An explicit `--base` takes the path it always took. A ref that does not exist
is an error, unborn or not. `changed-symbols` is an inventory with no base to
diff against, so it stays exit `2`, with a message that says why.

Unborn means `symbolic-ref -q HEAD` succeeds and `HEAD^{commit}` does not. A
detached or corrupt `HEAD` is not unborn and behaves as before. Both questions
go through `Git::try_run`; no spawn site is added.

For a root commit with no branch chain, the `last-commit` error now says that
`HEAD~1` was tried first and does not exist because `HEAD` is a root commit.
`branch` and `--base` errors are unchanged.

## Why

Exit `2` on an unborn `HEAD` blocks the first commit of every new project made
with the hook installed, and the only way past is to skip the hook. There is no
diff to bound, and ADR 0009 already says "run everything" is an answer, not a
failure.

Not an error, and not "nothing": an empty selection would let the first commit
through unchecked, which is the silent miss `run` must not have.

## Cost

One full run on the first commit. After it, `HEAD` exists and the next commit
selects normally.
