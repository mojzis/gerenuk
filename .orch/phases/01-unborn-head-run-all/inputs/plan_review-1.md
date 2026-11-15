# Inputs for the plan review: phase 1, Unborn HEAD runs the full suite

Read these files; they are the contract.

- Phase brief: `.orch/phases/01-unborn-head-run-all/brief.md`
- Pitch: `docs/pitch.md`
- Outline: `.orch/plan.md`

## Outline row

Unborn HEAD runs the full suite: On an unborn HEAD, `run` (default, `--scope last-commit`, `--scope branch`) and `impacted-tests` take `run_all` with a new reason naming the unborn HEAD / first commit; other base failures give a scope-matched error, and a missing explicit `--base` still exits 2.; covers R1, R2, R3, R4
