# 0014 — `run_all` delegates to a configured fallback, through the exec seam

Status: accepted. Amends [0011](0011-a-third-seam-that-only-execs.md), which
stays in force; extends [0009](0009-run-all-is-a-success.md).

## Decision

`gerenuk run` accepts a **fallback command** — `--fallback-command`, then
`GERENUK_FALLBACK`, then `fallback-command` under `[tool.gerenuk]` — and, when
and only when the outcome is `run_all`, execs it instead of the full suite
under pytest. `selected` and `nothing` never see it.

Three things about the shape are decided here rather than left to drift:

- **It goes through `pytest::Runner::exec`, and nowhere else.** The seam gained
  a `Handoff` — bytes for the child's stdin and variables for its environment
  — and nothing else. It is still exec-and-replace: gerenuk's process becomes
  the fallback, its exit code is the hook's, nothing is captured or parsed. The
  stdin bytes are written to an unlinked temporary file that becomes fd 0, not
  a pipe, because after the exec there is no writer, and a child that never
  reads must not hang. `fallback.rs` — resolution, program lookup, the payload
  — is pure, like everything around the seam.
- **The payload is a versioned external contract.**
  `{"gerenuk_fallback_payload_version": 1, "reason": …, "report": …}`, where
  `report` is the phase-1 `changed-symbols` report, reused rather than forked,
  and `reason` is `closure::Reason`'s existing `snake_case` name. Adding a
  field or a variant keeps the version; renaming or removing one bumps it.
  `GERENUK_FALLBACK_REASON` carries the reason alone for scripts that will not
  parse JSON.
- **Delegation is one-way.** The fallback receives context and ownership of the
  invocation and has no way to hand a selection back. A two-way protocol would
  be a new, separately versioned mechanism with its own record.

An empty argv in any layer of the chain is a configuration error found at
startup, before the diff is taken — not on the day the bail-out first happens.

## Why

The consumer is still a pre-commit hook, and [0009](0009-run-all-is-a-success.md)
still holds: `run_all` is a correct answer. In a large monorepo it is also an
expensive one, and repositories of that size usually already own a coarser
narrowing tool — a script from changed files to sub-projects. Letting the
repository substitute that for "everything" keeps the hook honest without
teaching gerenuk about sub-projects, which it has no business knowing.

Reusing the pytest seam is the whole point of the design. A second spawn site
would be a fourth seam ([0001](0001-two-impure-seams.md) says how much that
costs), and spawn-and-wait would be a different contract from the one the hook
is written against — signals, TTY, the exit code — so the fallback inherits
pytest's exactly.

## What this diverges from

- The seam was recorded in 0011 as "exec pytest". It now execs whatever the
  repository configured, and hands it stdin and an environment variable. The
  shape that made the seam acceptable — terminal, nothing consumed, pure on
  every side — is unchanged; the reason it was allowed still applies.
- `run` has one more source of configuration, and one more thing that can fail
  before pytest exists. Both fail with the existing operational code, `2`.

## Cost

- One more contract to keep: the payload schema and the reason names. The
  reason names were already pinned by `impacted-tests --format json`; this
  record makes them load-bearing twice.
- On the `--impact` replay path there is no phase-1 report, so `report` is
  `null`. A fabricated empty report would read as "nothing changed", which is
  worse than an honest null.
- The passthrough after `--` is pytest's and is not forwarded to the fallback.
  A fallback that wants pytest's flags has to own them.
- `tempfile` becomes a runtime dependency, for the unlinked file.

## What would make us revisit

Wanting the fallback to *answer* — node ids back to gerenuk to run, or a
per-reason command table. Either is a new mechanism with a new record, not a
widening of the payload.
