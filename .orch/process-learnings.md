
## Phase 1: Unborn HEAD runs the full suite

- In a brief, describe files that a test creates inside a tempdir as scratch files marked *(new)*, never as bare repo paths, or the plan-review existence check blocks the brief.
- Before dev starts, check whether the repo's own pre-commit hook (prek/pre-commit running `cargo test`) passes under the hook's environment, and list that check under Baseline. A hook that fails on the base commit blocks dev's commit, and dev is not allowed to skip it.
- Tell dev to commit through the real hook as part of the phase. The gate fails an uncommitted tree even when every check passes.
- Tests that spawn real git must strip git's repository-local env vars (GIT_INDEX_FILE, GIT_DIR, …), because they run inside hooks too. Use the helpers in tests/common rather than writing new ones.
- Never export a fake GIT_INDEX_FILE (or similar) in the agent's own shell to reproduce a hook issue. It rewrites the real checkout's index. Reproduce inside a tempdir repo instead.
