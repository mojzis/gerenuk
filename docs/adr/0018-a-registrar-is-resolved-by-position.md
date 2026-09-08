# 0018 — A registrar is resolved by position before it is scanned by name

**Status:** accepted. Amends [0012](0012-a-decorator-is-a-reference.md).

## Decision

When a decorated dead end is chased to its registrar (`app` for
`@app.command()`), the walk first looks for where the decorated symbol's own
module *binds* that name — an assignment or an import at module scope — and
queries `tyf refs` at that position. The answer is absorbed as [0012] absorbs a
word scan. The scan itself runs only when the module does not bind the name, or
when the query answers with nothing at all.

The "resolved" test is unchanged: a registrar counts as resolved when some site
is not one of the definitions it registers. Sites now come from `ty`, so an
`app` mentioned nowhere but its own decorator lines is `run_all` with
`decorator_dispatch`, even if a same-named object elsewhere would have matched.

One query per `(file, registrar)` per walk: every command on one `app`
dead-ends on the same registrar.

## Why

The word scan [0012] chose over-matched exactly as it said it would, and on
real code the match was a different object: a Typer `app` in `cli/main.py`
chased into a FastAPI `app` in `web/app.py`, whose module-level
`include_router` made the module a node and pulled in every `tests/test_web_*`
file — nine files for a one-line docstring in a CLI helper. The chain showed an
edge nothing in the source has.

The registrar is an ordinary name with an ordinary binding, and `ty` resolves
references to a module-level variable across the workspace, tests included.
That is the "revisit when" [0012] wrote down.

An empty answer is not trusted. Every `@app.command` line references `app`, so
a bound registrar with zero references is `ty` not resolving it, and the scan
remains the floor: over-selection over a silent miss.

## Cost

- One extra `tyf refs` round-trip per distinct registrar a walk dead-ends on.
- `pysource` now collects module-scope bindings, a second small table next to
  the import aliases.
- A registrar bound inside `if TYPE_CHECKING:` or built dynamically has no
  binding the parse can see and still takes the scan.

## Revisit when

`tyf` can answer "what does this decorator expression resolve to" directly,
which removes the binding lookup as well as the scan.
