# 0002 — `tyf refs` is queried by position, not by name

**Status:** accepted

## Decision

The closure asks `tyf refs <file>:<line>:<col>`, using the definition's *name*
position — not `tyf refs <QualName>`, which is what `audit` uses.

## Why

Two reasons, one of them a hard blocker.

**`tyf refs` has no name form for a nested symbol.** It accepts at most one dot
(`Class.member`); `Outer.Inner.method` is a usage error. Since `pysource`
produces exactly those qualnames, name queries would degrade every nested
definition to `run_all`. (Lifting that restriction in `ty-find` is worth doing
on its own — the language server can resolve the name; only tyf's argument
parser refuses. It would not change this decision.)

**A name cannot tell two symbols apart.** `mypkg.a:run` and `mypkg.b:run` answer
as one query, so the walk follows the union of their callers. A position cannot
be ambiguous.

## Cost

The column has to be exact, and a *wrong* column comes back as an empty
reference list rather than as an error — silent under-selection, the one failure
mode this tool must not have. Two things guard it: `pysource::SymbolSpan`
records the identifier's own position (not the decorator's), and the columns are
byte offsets, which equal character offsets for every prefix Python allows
before a definition's name (indentation, `def `, `class `, `async def ` — all
ASCII).

It also costs a field in the phase-1 schema ([0004](0004-phase1-schema-carries-positions.md)).

## Revisit when

Never, unless `tyf` drops position queries.
