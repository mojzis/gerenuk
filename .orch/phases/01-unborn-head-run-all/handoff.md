State of the code after phase 1 (commit f424190 on orch/20261115-121708):

**New and changed contracts**
- `git::Git::head_is_unborn()`: true only when HEAD is a symbolic ref to a branch with no commit. A detached HEAD is not unborn.
- `cli::resolve_base` returns a private `Resolved::{Base, Unborn}`. The unborn check is skipped when `--base` is given, so an explicit missing base is still exit 2.
- `impacted_run` short-circuits on unborn in the `Diff::WorkingTree` arm, before the untracked listing, the diff, the `upfront_reason`/`fast_suite` checks and the `tyf` lookup. Its git calls there are exactly 3: rev-parse `--show-toplevel`, symbolic-ref, rev-parse.
- `impact::unborn_head()` builds an `ImpactReport` with `verdict: run_all`, `reason: unborn_head`, `base: "HEAD"`, `merge_base: ""`, every list empty, and no `ChangedSymbols`. `ImpactRun.changed` is `None`, so the fallback payload's `report` is null.
- `closure::Reason::UnbornHead` has name `unborn_head` and label `HEAD has no commit yet`. `Reason::ALL` is `[Self; 11]`; `gerenuk_fallback_payload_version` is still 1.
- `changed-symbols` on an unborn HEAD exits 2 with "HEAD has no commit yet, so there is nothing to diff against…".
- `resolve_last_commit`'s root-commit error names `HEAD~1` and then the branch chain. The `--scope branch` and explicit `--base` messages are unchanged.
- Docs:
  - `docs/adr/0025-an-unborn-head-runs-everything.md`, with its row in `docs/adr/README.md` and a pointer in 0024.
  - `docs/src/guide/triage.md` ladder item 1 is the unborn reason. The page is at exactly the 60-line cap: any new reason needs prose cut first.

**Test infrastructure**
- `tests/common/mod.rs::strip_hook_git_env` removes `pytest::LOCAL_GIT_ENV` from `TestRepo::git` and from gerenuk commands.
- The `src/git.rs` unit tests drop the same env once, via `Once` and `remove_var`.
- In `tests/run.rs`, `without_tools`, the hook fixture's `git()` and the first-commit test strip it too.
- Any new test that spawns real git must use these, or it fails under the repo's prek `cargo test` hook.

**Known gaps and deferred items**
- **P2:** production `Git::output` still inherits the hook's `GIT_INDEX_FILE` and the rest. `replay` run inside a pre-commit hook could act on the wrong index.
- **P1:** `run --dry-run` prints a `pytest not found` line to stderr when pytest is not on PATH, then exits 0 with `argv: unknown`. Not specific to unborn HEAD.
- `unborn_head` is not yet mentioned in `docs/src/` run.md, impacted-tests.md, tune.md or `docs/dev/ARCHITECTURE.md`. Those pages list `fast_suite`; no test requires the new reason there.
- The unborn first-commit hook test uses its own simple hook, not the `tests/run.rs:1225` fixture.
- Running `git fsck` on the dev checkout may show a dangling object, left after repair restored the index with `git read-tree HEAD`.
