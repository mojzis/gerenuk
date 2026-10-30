# 0023 — A replay checks its commit out in a worktree it keeps

**Status:** accepted

## Decision

When the checkout does not match the commit `replay` was asked about, gerenuk
checks that commit out into a linked worktree at
`<git-common-dir>/gerenuk/replay` and replays from there, instead of refusing.
The user's own checkout — `HEAD`, index, uncommitted edits — is not touched. A
checkout that already matches is used as it is, and no worktree is made.

The worktree is kept. The next replay moves it with `git checkout --detach
--force` and removes untracked files with `git clean -d --force`; ignored files
(a `.venv`, caches) stay. A directory at that path that is not a registered
worktree of the repository is an error and is never overwritten; a
registration whose directory was deleted is pruned and made again.

The commit and `--base` are resolved to shas in the user's checkout, before the
switch: `HEAD~1` inside the worktree would name a different commit.

## Why

The rule that a replay reads a commit only through a checkout that matches it
stays: `tyf`, the file index and pytest still read a working tree, and no seam
learns about commits. What changes is who makes that checkout. Being handed
five shas and told to `git switch` or `git worktree add` for each is busywork
that gerenuk can do with the git seam it already has.

Keeping one worktree is what makes a run of replays cheap. The first pays for a
full checkout; each later one rewrites only the files that differ between the
two commits, which for commits from the same week is usually a handful.

Inside the git directory the worktree is per repository with no naming scheme,
is invisible to `git status`, and goes away with the clone.

## Cost

- pytest under `--run` starts in the worktree, with whatever environment
  resolves there. A virtualenv that has the project installed editable points
  at the main checkout, so a `src/` layout can import that checkout's code
  instead of the commit's. A `pytest-command` such as `uv run pytest` builds an
  environment for the worktree the first time, which is correct but slow once.
- `ty` looks for a `.venv` at the project root, and the worktree has none, so
  third-party imports may not resolve. Project-internal references, which are
  what the walk follows, do.
- Two replays at once in the same repository share the worktree and can race.
- The worktree holds disk space until `git worktree remove --force` drops it;
  gerenuk names the command every time it uses the worktree.

## Revisit when

- Replays need to run concurrently, which wants one worktree per commit.
- Users replay against long-lived environments often enough that the editable
  install problem bites, which wants an option to point pytest at an
  environment built for the worktree.
