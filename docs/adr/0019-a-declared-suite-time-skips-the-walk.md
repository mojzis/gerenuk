# 0019 — A declared suite time skips the walk

**Status:** accepted

## Decision

`[tool.gerenuk] suite-ms` declares how long the whole suite takes. When it is
set, at or under a built-in selection cost of 2000 ms, and the diff would have
needed a walk, `gerenuk run` skips the walk and says `run_all` with reason
`fast_suite`. The check sits with the other up-front verdicts, before `tyf` is
looked for. A diff that seeds no walk is unaffected. `impacted-tests` ignores
the key. A configured fallback receives `fast_suite` like any other `run_all`.

## Why

Across a four-repository rollout, every suite ran in 0.5–3 s and every selected
run was slower than the full run it replaced: selection is 1.5–2 s of `tyf`
round-trips, and on a one-second suite that is the whole budget twice over. The
hook is still correct there, just not useful, and the fix is to not analyse.

gerenuk cannot measure the suite: `run` execs pytest and never sees it finish.
The repository can, from pytest's own summary line, and a declared number is a
fact about the repository that survives gerenuk getting faster — the
comparison constant moves, the declaration does not. A boolean "selection off"
would say less and age worse.

The gate is `run`'s and not the walk's because it is about economics, and only
`run` pays and then runs. What could break is the same question however fast
the suite is, which is what `impacted-tests` answers.

## Cost

- A stale declaration on a suite that has grown quietly disables selection.
  The reason is in every hook log line, and the triage guide says to remove
  the key.
- The 2000 ms constant is a rollout measurement, not this machine's. A
  repository on the boundary should measure rather than guess.
- One more `run_all` reason in the fallback payload's table; the payload
  version is unchanged, as adding a variant is defined to be.

## Revisit when

gerenuk keeps a record of its own selection cost per repository, at which
point the constant becomes a measurement and the declaration a comparison.
