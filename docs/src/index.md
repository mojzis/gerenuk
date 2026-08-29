# gerenuk

**Symbol-level Python code intelligence, powered by [ty-find](https://github.com/mojzis/ty-find).**

`tyf` answers questions about one symbol at a time. `gerenuk` asks those
questions for every symbol in a file and reports what stands out:

```
$ gerenuk audit sample_pkg/service.py
warn  sample_pkg/service.py:43  func `legacy_export` has no references
note  sample_pkg/service.py:34  method `ShelterService.seniors` is referenced only from tests (1)

1 file(s), 7 symbol(s) checked — 1 warn, 1 note
```

## Why

Grep tells you whether a name appears somewhere. It cannot tell you whether that
appearance is a real reference, a docstring, or a same-named symbol in another
module. `gerenuk` asks `ty`'s type checker instead, through `tyf`, so the answer
follows Python's actual name resolution.

Two things fall out of that:

- **Unreferenced symbols** — nothing in the project uses them.
- **Test-only symbols** — production code stopped using them, but the tests
  kept them alive. Usually the residue of an unfinished refactor.

Turned around, the same reference graph answers the opposite question — *which
tests could this change break?* — which is what
[`changed-symbols`](commands/changed-symbols.md) and
[`impacted-tests`](commands/impacted-tests.md) are for.

## What it is not

`gerenuk` reports static references. Dynamic dispatch, plugin registries,
`getattr` lookups and `__all__` re-exports are invisible to it. Treat findings
as leads to confirm, not as a delete list.

## Next

- [Setup](setup.md) — install `gerenuk` and its `tyf` prerequisite
- [Commands](commands/overview.md) — the full CLI surface
- [How it works](how-it-works.md) — the pipeline, end to end
