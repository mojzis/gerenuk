# 0026 — A replay drops the hook's git environment

**Status:** accepted, amends [0020](0020-pytest-does-not-inherit-the-hooks-git.md)
and [0023](0023-a-replay-checks-itself-out.md)

## Decision

`git::Git` carries an environment policy per instance. `Git::discover` and
`Git::rebind` inherit git's repository-local variables (`pytest::LOCAL_GIT_ENV`)
exactly as before; `Git::isolated` binds a `Git` that removes them from every
command it spawns. The spawn site is still the private `Git::output`.

Three things are isolated:

- inside `Git::replay_worktree`, everything after locating the git common
  directory — `worktree prune`, `worktree list`, `worktree add`, and the
  `checkout` and `clean` in the kept worktree;
- every git call `run_pytest` makes for a replay, from the first `rev-parse
  --show-toplevel` on, whether the replay uses the worktree or the user's
  checkout;
- both children of a replay: pytest and the fallback are handed an `Isolate`
  plan whatever `git-env` says.

Everything else — `changed-symbols`, `impacted-tests`, `run`, and the part of
`replay` that resolves `COMMIT` and `--base` in the user's checkout — inherits,
as 0020 decided.

## Why

Inside a hook those variables describe the checkout being committed. A
relative `GIT_INDEX_FILE=.git/index`, which prek exports, cannot be opened from
the replay worktree, where `.git` is a file; `worktree add` hands its own
checkout child that same index, which from the user's checkout is the user's
real one. An absolute `GIT_DIR` and `GIT_WORK_TREE` aim the worktree's
`checkout --force` back at the user's checkout. So a replay from a hook either
failed or wrote to the wrong place.

The hook's own diff still inherits, on purpose: on a partial commit the hook's
`GIT_INDEX_FILE` is a temporary index, and what it tracks is what is being
committed. `the_hooks_own_diff_reads_the_hooks_index` pins that, with a file
force-staged into an index only the hook can see.

A replay is about a commit, not about what the hook is committing. Its
children run on that commit's tree, so the hook's index describes nothing they
see; 0020's reason for letting the fallback inherit does not hold there.

## Cost

- `Git` has two constructors that differ only in what the child inherits, and
  picking the wrong one is silent. The tests in `tests/run.rs` replay under a
  relative `GIT_INDEX_FILE` and under absolute `GIT_DIR`/`GIT_WORK_TREE`, and
  check that the user's HEAD, index and working tree are unchanged.
- Isolated calls rely on discovering the repository from their cwd. A setup
  that only works with an exported `GIT_DIR` — a work tree with no `.git` at
  all — cannot replay from inside a hook.
- `git-env = "inherit"` is ignored by a replay. Nothing a replay runs can use
  the hook's index.

## Revisit when

Something other than a replay has gerenuk's git aim at a checkout that is not
the hook's. It takes `Git::isolated`, not a new flag.
