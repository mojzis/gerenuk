# 0017 — A helper in a test file is a step, not an answer

**Status:** accepted

## Decision

A reference that lands inside a definition in a test file is recorded as an
impacted test only when pytest reaches that definition by name: a `test_*`
function, a `Test*` class, a `@pytest.fixture`, or a name in the framework-hook
table (`setup_method`, `teardown_class`, …). Anything else — a helper method on
a test class, a module-level factory, a closure inside a test — is expanded on
the next frontier exactly like a definition in production code.

A helper whose expansion finds nothing is then recorded after all, as it was
before this rule, so the selection degrades to its class or file.

## Why

`TestQueryLogging.logged_run_sql` was listed among 26 "impacted tests" in a
real rollout. It is not a test; pytest cannot collect it, and the mapping in
`run` widened it to its whole class. Its callers are the tests around it, and
`tyf` resolves `self.logged_run_sql(...)` like any other call — so stepping
through it names the tests precisely, and the chain says why.

The dead-end fallback is the safety argument. A helper reached only by a route
the walk cannot see — `getattr`, a hook the table does not know — has no
visible callers, and "no callers" is exactly the case that would otherwise
become a silent miss. Recording it there keeps the old, wider answer for the
one shape where the new rule could narrow.

## Cost

- One more `tyf` round-trip per level of helpers between a change and its
  tests. Helpers are shallow; the depth budget already bounds it.
- A helper called both by tests `tyf` sees and by one it does not
  under-selects the latter. That is the same gap every walked edge has.

## Revisit when

pytest's collection conventions are read from configuration rather than
assumed; then "reached by name" is exact instead of the defaults plus a table.
