## Phase 1: Unborn HEAD runs the full suite. Verdict: ship (commit f424190)

**What was built**
- `Git::head_is_unborn()` *(new)*. It runs `symbolic-ref -q HEAD` and `rev-parse --verify --quiet HEAD^{commit}`, both through `try_run`, so there is no new spawn site.
- `cli::resolve_base` now returns a private `Resolved::{Base, Unborn}`.
- `impacted_run` returns `ImpactRun::unborn()` before it lists untracked files, diffs, looks for `tyf`, or builds the file index.
- `impact::unborn_head()` *(new)* builds the report without a `ChangedSymbols`, so the fallback payload carries `report: null`.
- `Reason::UnbornHead` *(new)*: name `unborn_head`, label `HEAD has no commit yet`. `ALL` now has 11 entries; the payload version is still 1.
- `resolve_last_commit` now says, on a root commit, that `HEAD~1` was tried first.
- `changed-symbols` on an unborn HEAD still exits 2, with a clearer message.
- Docs: ADR 0025, a README row, a pointer in ADR 0024, and triage ladder item 1. `triage.md` is held at exactly 60 lines.
- Tests: 601 → 625 (+24). No snapshot changed.

**What went wrong and how it was caught**
- **First brief failed plan review.** It named the issue's repro files (`tests/test_a.py`, `pkg/__init__.py`) as bare paths. The existence checker read them as repo files that don't exist. Fixed by describing them as scratch files created in tempdirs, marked *(new)*.
- **Dev didn't commit.** The repo's prek pre-commit hook runs `cargo test`, and 4 replay-worktree unit tests failed there. Their cause was pre-existing, not this phase: the hook exports a relative `GIT_INDEX_FILE=.git/index`, and the tests' real git calls inherited it. The permission classifier denied `SKIP=cargo-test`. The gate then caught the uncommitted tree.
- **Repair.** Test helpers now strip `pytest::LOCAL_GIT_ENV`: a `Once` in the `git.rs` unit tests, `strip_hook_git_env` in `tests/common/mod.rs`, and the same in the `tests/run.rs` fixtures. Repair also corrupted the outer repo's index once by exporting a fake `GIT_INDEX_FILE` during testing; it fixed that with `git read-tree HEAD`.

**Where the reports were right or wrong**
- The dev report was honest about the missing commit and about writing tests alongside the code rather than red-first.
- The combined review re-ran every functional-QA item by hand: real pytest, the payload, the 3-git-call bound, and the orphan worktree.
- Gap: A9 (the hook path) was accepted on test evidence alone. The new hook test uses its own simpler hook rather than the `tests/run.rs:1225` fixture the brief named.
