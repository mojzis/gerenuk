# 0024 — The default diff is the last commit, not the branch

**Status:** accepted

## Decision

With no `--base`, the working tree is diffed from `HEAD~1` — the last commit
plus anything not committed yet. `[tool.gerenuk] scope = "branch"` (or
`--scope branch`) restores the old default: the merge base with `origin/main`,
then `main`, then `master`. `--scope` beats the key, `--base` beats both and
conflicts with `--scope`. A root commit has no `HEAD~1`, so `last-commit` falls
back to the branch chain there.

## Why

A hook that diffs the whole branch re-checks every earlier commit on every new
one. On a long branch that is most of the suite, every time, and people start
skipping the hook. Each earlier commit was already checked by the hook that
made it, so the last commit is enough.

`HEAD~1` rather than `HEAD`. In a pre-commit hook `HEAD` would be the tighter
fit — only the commit being made — but anywhere else (pre-push, CI, by hand
after committing) a clean tree against `HEAD` is an empty diff, and an empty
diff selects nothing and exits `0`. That is a silent miss. `HEAD~1` is the last
commit in every context: in a hook it also re-checks the previous commit, which
costs a little and never misses anything. It also covers `commit --amend`,
whose hook would otherwise see only the amend's delta.

## Cost

A branch merged without its commits passing their own hooks (`--no-verify`, a
rebase that never ran hooks) is no longer checked as a whole. `scope =
"branch"` is the answer for a repository where that matters, and CI should run
the branch scope or the full suite regardless.
