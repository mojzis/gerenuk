# 0009 — `run_all` is a successful answer, not an error

**Status:** accepted

## Decision

`impacted-tests` always emits a verdict. `run_all` — with a machine-readable
`reason` — exits `0`, exactly like `selected`. Exit `2` is reserved for a run
that could not produce a verdict at all (not a repository, unreadable
`--changed` file). There is no exit `1`.

Anything that goes wrong *during* the walk — `tyf` missing, `tyf` returning
garbage, a file that vanished — degrades to `run_all` with the message recorded
in `errors`, never to a crash and never to a silent under-selection.

## Why

The consumer is a pre-commit hook. "Run everything" is a correct, useful,
actionable answer; failing the hook because the analysis was inconclusive
trains people to bypass it. Under-selecting silently is the one outcome that
makes the tool worse than not having it.

`audit`'s `0/1/2` is untouched: there, `1` means "findings", and impacted tests
are an inventory rather than a verdict.

## Cost

A caller that wants to know whether the walk succeeded has to read `verdict`
rather than `$?`.

## Revisit when

Never; this is the whole safety argument for the feature.
