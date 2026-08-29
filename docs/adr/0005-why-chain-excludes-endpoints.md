# 0005 — `via` excludes both endpoints

**Status:** accepted

## Decision

`impacted_tests[].via` lists the symbols strictly between the test and
`origin`, nearest-to-the-test first. An empty `via` means the test references
the changed symbol directly.

## Why

The pitch contradicts itself: its prose says "excluding both endpoints", its
JSON example includes `origin` in `via`. The prose definition was taken, because
"empty `via` means a direct reference" is a crisp invariant a test can assert,
and because repeating `origin` in two fields invites them to drift.

## Cost

Reconstructing the full chain for display means `[test] + via + [origin]` rather
than `[test] + via`. The human renderer does that.

## Revisit when

Never on its own; only if a consumer proves the concatenation is a nuisance.
