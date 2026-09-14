# 0020 — pytest does not inherit the hook's git environment

**Status:** accepted. Amends [0011](0011-a-third-seam-that-only-execs.md)
and [0014](0014-run-all-delegates-to-a-fallback.md), which stay in force.

## Decision

By default, the pytest `gerenuk run` execs does not inherit git's
repository-local environment: `GIT_DIR`, `GIT_INDEX_FILE`, `GIT_WORK_TREE`,
`GIT_PREFIX` and the rest of `git rev-parse --local-env-vars`. The list is a
copy in `pytest::LOCAL_GIT_ENV`, checked against the installed git by an
integration test. `[tool.gerenuk] git-env = "inherit"` or `run --git-env
inherit` turns it off. The fallback command is not governed by the policy and
inherits everything, always.

The removal happens in the `Handoff`, which gains a `remove` list next to its
`env` and `stdin`. The seam is otherwise unchanged: still exec-and-replace,
still nothing captured, and still the one place a process is spawned.

## Why

A pre-commit hook runs with those variables exported so that every git the
hook spawns targets the repository being committed — including, on a partial
commit, a temporary index, and in a linked worktree an absolute `GIT_DIR`.
Git documents this and recommends clearing them before operating on another
repository. A test that creates a repository of its own under `tmp_path` and
inherits them does not: its `git init`, `git config`, `git add` and `git
commit` land in the repository being committed. Two downstream projects saw
exactly that — an outer index changed, an identity rewritten, fixture commits
on the real branch.

gerenuk is the process between the hook and pytest, so it is where the
boundary can be drawn without asking every test to draw it. Its own diff has
already run by then, in the hook's context, which is the context it needs.

The fallback is different: it is the repository's own script, run from the
hook it was configured for, and it may need the very index git handed that
hook. Stripping context from an arbitrary git-aware command would be a silent
change to what it sees. So its contract is stated rather than guessed: it
inherits everything, and if it runs pytest, isolating that is its job.

Configurable, because a repository can have a test that reads the hook's
context on purpose, and because a policy that cannot be turned off is one
that cannot be diagnosed. The default is the safe direction.

## Cost

- The list is a copy. A newer git that adds a variable is caught by the test
  that compares the two, not by a hook, and only once that git is installed
  where the tests run.
- This is a safeguard, not a sandbox. A test that reads `GIT_DIR` from its
  own environment and passes it on, or that finds the outer repository by
  walking up from its cwd, still reaches it. Project helpers that target
  another repository should sanitise their own subprocess environment; the
  runner's removal is additional protection.
- `--dry-run` gained a `git_env` field and a line, so the JSON shape grew by
  one always-present key.

## Revisit when

A fallback command wants the same treatment, at which point the policy grows a
value or the fallback its own key — a per-child contract, not a widening of
this one.
