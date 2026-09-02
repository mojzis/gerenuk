# 0012 — A registering decorator is a reference to what it decorates

**Status:** accepted

## Decision

When the walk dead-ends at a symbol carrying a decorator, gerenuk chases the
decorator's *registrar* — the object the decorator hangs the function on —
and absorbs its word-boundary occurrences as if they were references to the
decorated symbol.

- `@app.command()` → registrar `app`; `@router.get(...)` → registrar `router`.
- The first dotted segment is the registrar: it is the local name a test
  imports and drives.
- A **bare** decorator (`@register`) has none. That line already references
  `register` by name, so `tyf` draws the edge itself.
- Decorators that wrap rather than register — `@property`, `@staticmethod`,
  `@functools.wraps`, `@dataclass`, … — are inert and never chased.
- A decorator matched by `ignore-decorators` is not chased: that is a
  deliberate dead end, and phase 1 made the same call.

The scan runs **only on a dead end**. A decorated symbol with real callers is
already on a path to its tests and never pays for it.

When no registrar can be resolved, the verdict degrades to `run_all` with
`reason: "decorator_dispatch"`, and `errors` names the symbol and the decorator.

A registrar counts as resolved when it is mentioned somewhere that is *not* one
of the definitions it registers — `app.add_typer(journal_app)`, a module-level
`include_router`, or a test driving the app. Every `@app.command` line mentions
`app`, so "the scan found something" cannot mean resolved.

## Why

Nothing references a Typer command or a FastAPI route. The framework holds the
only handle, so `tyf refs` returns the definition and nothing else, the walk
stops, and `impacted-tests` answered `selected` with an empty list — indis­tin­gui­shable,
byte for byte, from the correct answer for a genuinely unused symbol.

Measured against two real projects before this change: **248 tests** that a
one-line edit actually broke were not selected, and every one of them was
reached through a decorator. `gerenuk run` spawned no pytest and exited `0`.
That is precisely the silent under-selection [0009](0009-run-all-is-a-success.md)
calls the one outcome that makes the tool worse than not having it.

The word-boundary scan is the same instrument the deleted-symbol path already
uses, chosen for the same reason: no type checker can resolve this edge, and
over-selection is the safe direction.

The escalation exists because resolving the registrar is a heuristic and
heuristics miss. A miss must cost a full suite run, never a wrong green.

## Cost

- Over-selection when a registrar's name is common. `app` also matches an
  unrelated `app` in another module, and those tests get selected too.
- A registrar with more than `REGISTRAR_SITE_CAP` (400) hits is not absorbed:
  a name like `pytest` says nothing about who drives the function, and
  absorbing thousands of sites would be slower and no more correct.
- One extra `Index::classify` per hit on dead-ended symbols only.

## Revisit when

`tyf` can answer "what does this decorator expression resolve to", which would
replace the word scan with a real edge and remove both the cap and the
over-selection.
