# 0008 — A symbol id without a colon is a module

**Status:** accepted

## Decision

`mypkg.enrich:Enricher.run` is a symbol. `mypkg.enrich` — no colon — is the
module itself. The convention holds in `via`, in `origin` and in
`impacted_tests[].symbol`.

`impacted_tests[].symbol` is `null` when a whole test *file* was selected rather
than one test function, which is what a module-level edge produces.

## Why

Two things in phase 2 are not symbols but have to appear in the same fields: the
module reached by a module-level reference, and a test file selected wholesale.
Inventing a second id namespace for them would mean every consumer parsing two
shapes; a colon is already the separator, so its absence is free to carry
meaning.

`null` rather than a made-up symbol name keeps phase 3 honest: a null symbol
means the pytest node id is the file path.

## Cost

Consumers have to split on `:` to tell the two apart. Documented in the command
page.

## Revisit when

Phase 3 finds the null case awkward to map onto node ids.
