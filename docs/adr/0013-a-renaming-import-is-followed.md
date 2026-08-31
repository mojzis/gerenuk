# 0013 — A renaming import is followed; a plain one is still dropped

**Status:** accepted
**Amends:** the import-dropping rule described in
[0005](0005-why-chain-excludes-endpoints.md)'s neighbourhood and documented as a
known gap in `impacted-tests`.

## Decision

A reference that lands on an import line is dropped — **unless** that import
renames the symbol. `from x import y as z` and `import a.b as c` become a new
node: the alias, queried at its own position in the importing module.

The node's id is `<importing module>:<alias>`, so a `via` chain shows exactly
where the rename happened:

```
tests/routes/test_session_detail.py
  ← introspect.api.main
  ← introspect.api.routes:session_detail
  ← introspect.api.routes:_session_detail
```

## Why

Dropping import lines rests on one assumption: *the importing module's own uses
of the symbol show up as their own references, so the import line is redundant.*
That is true only while the name survives the import.

With a rename it is false, and quietly so. `tyf refs` for the original name
returns the import line and nothing else, because the calls below spell the
alias — a different binding, which `tyf` will not return for a query about the
original. Dropping that one line severs the graph, and everything past it
becomes invisible.

Measured on `introspect`, whose `api/` layer delegates through renamed imports
as a matter of convention: a broken handler reached **no tests at all** (1 symbol
visited, 0 impacted) while the change really broke 52. Following the alias
reaches the route function, whose decorator then carries the walk out to the
tests. 41 imports in that project are renaming ones; in a large repo, a
re-exporting package layer can put one on every path out of the leaves.

## Cost

One extra index lookup per import site — only for sites that were about to be
dropped.

A renamed import that is genuinely unused now adds a node where it previously
added none. It has no references, so it dead-ends immediately.

## Revisit when

`tyf` resolves an alias binding back to the symbol it names, which would make
the import line an ordinary reference and this rule unnecessary.
