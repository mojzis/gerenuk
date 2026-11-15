
## outline (outline)

session `23d986ca-8a45-4edf-8988-60bdc1b13792`, $0.5643

```json
{
  "phases": [
    {
      "title": "Unborn HEAD runs the full suite",
      "slug": "unborn-head-run-all",
      "deliverable": "On an unborn HEAD, `run` (default, `--scope last-commit`, `--scope branch`) and `impacted-tests` take `run_all` with a new reason naming the unborn HEAD / first commit; other base failures give a scope-matched error, and a missing explicit `--base` still exits 2.",
      "covers": [
        "R1",
        "R2",
        "R3",
        "R4"
      ],
      "depends_on": [],
      "size": "M",
      "scenario": "small",
      "reason": "Pitch sets a one-phase small appetite; changes are confined to git.rs base resolution, one Reason variant, its call sites and the guide page.",
      "approach": "Have `git::Git` detect unborn HEAD (via its existing run/try_run seam) and return a distinct outcome from base resolution that `cli::resolve_base` maps to a new `Reason::UnbornHead` (added to `Reason::ALL`, payload version stays 1, named in the guide triage ladder); make the no-base error list the candidates actually tried for the scope; explicit `--base` path unchanged; integration tests in tests/run.rs and tests/impacted_tests.rs build the issue's repro repo with a staged but uncommitted tree.",
      "set": []
    }
  ]
}
```

## outline_review (outline_review)

session `282716df-40ce-482f-bb1d-1c8f482af1e5`, $0.7298

```json
{
  "ok": true,
  "findings": [
    {
      "id": "changed-symbols-unspecified",
      "blocking": false,
      "title": "Effect on `changed-symbols` is not stated",
      "detail": "`cli::resolve_base` (src/cli.rs:467) is shared by `run_changed_symbols` (src/cli.rs:451) and the impacted-tests/run path (src/cli.rs:598). If base resolution starts returning an 'unborn' outcome, the outline should say what `changed-symbols` does with it. The safest choice is to keep today's exit 2: it is an inventory, so it has no run_all to fall back to, and its exit-code contract is 0 or 2 only. The brief should state this so dev doesn't invent a new behaviour."
    },
    {
      "id": "short-circuit-before-diff",
      "blocking": false,
      "title": "run_all must come before `git diff` and `untracked`",
      "detail": "On an unborn HEAD there is no merge-base, so `changed_report`'s `git.diff(&base.merge_base)` would fail. The unborn verdict has to come out at the `resolve_base` call site, before `changed_report` runs. It also has to come before `tyf` discovery, matching the existing invariant that up-front verdicts are checked before `tyf` is looked for. The outline implies this but doesn't say it. Worth one sentence in the brief, plus a test with an empty `PATH`, like the existing up-front-verdict tests."
    },
    {
      "id": "replay-untouched",
      "blocking": false,
      "title": "Replay no-go is respected",
      "detail": "Replay picks its base through `replay_checkout` and `replay_base` (src/cli.rs:653-700), not `resolve_base`, so the planned change doesn't reach it. The brief should still say that `Git::resolve_commit` is out of scope."
    },
    {
      "id": "scenario-ok",
      "blocking": false,
      "title": "Scenario, size and appetite fit",
      "detail": "One phase, size M, scenario `small` (the default, with a reason given). It covers R1-R4, matches the pitch's one-phase appetite, adds no flags or config keys, and keeps the git spawn inside `git::Git`. Adding a `Reason` variant without bumping the payload version follows the CLAUDE.md rule (adding a variant keeps the version). The guide test that names every `Reason::label` is updated in the same phase, so the gate stays green."
    }
  ]
}
```
