# Plan

Run `20261115-121708` on branch `orch/20261115-121708`. Written by `orch plan`; each phase's brief is written just before it runs.

| # | phase | covers | depends on | size | scenario | approach | reason |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | Unborn HEAD runs the full suite | R1, R2, R3, R4 | - | M | small | Have `git::Git` detect unborn HEAD (via its existing run/try_run seam) and return a distinct outcome from base resolution that `cli::resolve_base` maps to a new `Reason::UnbornHead` (added to `Reason::ALL`, payload version stays 1, named in the guide triage ladder); make the no-base error list the candidates actually tried for the scope; explicit `--base` path unchanged; integration tests in tests/run.rs and tests/impacted_tests.rs build the issue's repro repo with a staged but uncommitted tree. | Pitch sets a one-phase small appetite; changes are confined to git.rs base resolution, one Reason variant, its call sites and the guide page. |

## Deliverables

1. **Unborn HEAD runs the full suite**: On an unborn HEAD, `run` (default, `--scope last-commit`, `--scope branch`) and `impacted-tests` take `run_all` with a new reason naming the unborn HEAD / first commit; other base failures give a scope-matched error, and a missing explicit `--base` still exits 2.

## Requirement coverage

- R1: phase 1
- R2: phase 1
- R3: phase 1
- R4: phase 1
