# gerenuk

**Impact-based pytest selection for Python, powered by
[ty-find](https://github.com/mojzis/ty-find).**

A diff comes in; `gerenuk run` runs exactly the tests that diff impacts, and
nothing else:

```
$ gerenuk run -- -q
gerenuk: 5 node id(s) from 1 origin(s) in 152 ms — details: gerenuk impacted-tests
.........                                                                [100%]
9 passed in 0.02s
```

## Why

A test selector is only as good as its notion of "reaches". Matching names is
not one: it cannot tell two same-named symbols apart, and it cannot follow a
call through a re-export. `gerenuk` asks `ty`'s type checker instead, through
`tyf`, so the reference graph it walks follows Python's actual name resolution.

Three commands, one pipeline:

- [`changed-symbols`](commands/changed-symbols.md) — what the working tree
  changed, from `git` alone.
- [`impacted-tests`](commands/impacted-tests.md) — which tests reach those
  symbols, with the `←` chain that says why.
- [`run`](commands/run.md) — the same walk in-process, mapped to pytest node
  ids, and then it *becomes* pytest.

The one failure mode that would make a selector worse than no selector is a
short list that quietly misses a test. So every degrade widens: anything
gerenuk cannot see through becomes `verdict: run_all` — run the whole suite —
rather than a confident answer.

## The companion: `audit`

Turned around, the same reference graph answers the opposite question — *what
does nothing reach?* [`audit`](commands/audit.md) asks it for every symbol in a
file and reports two findings: symbols nothing references, and symbols only
tests reach.

```
$ gerenuk audit sample_pkg/service.py
warn  sample_pkg/service.py:43  func `legacy_export` has no references
note  sample_pkg/service.py:34  method `ShelterService.seniors` is referenced only from tests (1)

1 file(s), 7 symbol(s) checked — 1 warn, 1 note
```

It is a verifier rather than a repo-wide scanner: precise, per-file, and it
names the referencing sites. Run it on what a sweep like
[vulture](https://github.com/jendrikseipp/vulture) already flagged — see
[audit](commands/audit.md) for the pipeline.

## What it is not

`gerenuk` reports static references. Dynamic dispatch, plugin registries,
`getattr` lookups and `__all__` re-exports are invisible to it. Selections
handle that by widening; audit findings are leads to confirm, not a delete
list.

## Next

- [Setup](setup.md) — install `gerenuk` and its `tyf` prerequisite
- [Commands](commands/overview.md) — the full CLI surface
- [How it works](how-it-works.md) — the pipeline, end to end
