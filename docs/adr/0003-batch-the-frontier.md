# 0003 — One `tyf refs` call per BFS level

**Status:** accepted

## Decision

`closure::Refs::refs` takes the whole BFS frontier (`&[SymbolQuery]`), and the
`tyf`-backed implementation sends all of it to a single `tyf refs` invocation.
`stats.tyf_calls` therefore counts *levels*, not symbols.

## Why

`tyf refs` accepts several queries per invocation and resolves them in parallel
against one warm daemon, so a frontier of twenty symbols costs one process spawn
and one LSP round-trip rather than twenty of each. A depth-4 closure is a
handful of subprocess calls; the fixture walk runs in about 14 ms warm.

## Cost

The multi-query answer is a JSON *array* while a single query answers with a
bare object. `tyf::parse_refs_batch` accepts either, which is a small amount of
tolerance in exchange for not depending on which shape a given `tyf` picks.

Answers are matched to queries by index. `closure::RefAnswer` carries the symbol
id anyway, so the closure itself does not depend on order — only the adapter
does, and it checks that the answer count matches the query count.

## Revisit when

A frontier grows large enough that one invocation's argument list or response
becomes unwieldy. Chunking is a change inside `TyfRefs`; the closure does not
move.
