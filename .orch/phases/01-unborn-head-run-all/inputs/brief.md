# Inputs for the brief: phase 1, Unborn HEAD runs the full suite

Read these files; they are the contract.

- Pitch: `docs/pitch.md`
- Outline: `.orch/plan.md`

## This phase's outline row

- phase 1: **Unborn HEAD runs the full suite**
- deliverable: On an unborn HEAD, `run` (default, `--scope last-commit`, `--scope branch`) and `impacted-tests` take `run_all` with a new reason naming the unborn HEAD / first commit; other base failures give a scope-matched error, and a missing explicit `--base` still exits 2.
- covers: R1, R2, R3, R4
- size: M
- scenario: `small` (Pitch sets a one-phase small appetite; changes are confined to git.rs base resolution, one Reason variant, its call sites and the guide page.)
- approach: Have `git::Git` detect unborn HEAD (via its existing run/try_run seam) and return a distinct outcome from base resolution that `cli::resolve_base` maps to a new `Reason::UnbornHead` (added to `Reason::ALL`, payload version stays 1, named in the guide triage ladder); make the no-base error list the candidates actually tried for the scope; explicit `--base` path unchanged; integration tests in tests/run.rs and tests/impacted_tests.rs build the issue's repro repo with a staged but uncommitted tree.
