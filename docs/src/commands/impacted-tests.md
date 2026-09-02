# `gerenuk impacted-tests`

Walk the reference graph from the symbols the working tree changed out to the
tests that reach them.

```
gerenuk impacted-tests [--base <REF>] [--changed <FILE>]
                       [--max-depth <N>] [--max-symbols <N>] [--budget-ms <MS>]
```

This is the second stage of impact-based test selection.
[`changed-symbols`](changed-symbols.md) answers *what changed*; this answers
*what could break*. It does not run `pytest` — the output is deliberately
node-ID-shaped so something else can.

It needs both `git` and `tyf`.

## The verdict

Every run emits a **verdict**, and both kinds exit `0`:

| Verdict | Meaning |
|---|---|
| `selected` | The walk completed. `impacted_tests` is the answer. |
| `run_all` | Run the whole suite. `reason` says why the selection is not trustworthy. |

That is the whole safety argument. A pre-commit hook that quietly under-selects
is worse than no hook, so anything gerenuk cannot see through becomes "run
everything" rather than a short list or a crash.

| `reason` | Why |
|---|---|
| `non_python_changes` | The diff touched files symbol analysis cannot read |
| `parse_errors` | A changed file did not parse, so its symbols are unknown |
| `tyf_unavailable` | `tyf` is not installed, so no reference can be resolved |
| `refs_failed` | `tyf` failed or answered unparseably part-way through |
| `index_failed` | The working tree could not be read part-way through |
| `max_depth` / `max_symbols` / `budget` | A limit tripped before the frontier emptied |
| `decorator_dispatch` | A changed symbol is registered by a decorator whose registrar could not be resolved, so the framework's route to its tests is invisible. `errors` names the symbol and the decorator. |

`non_python_changes` and `parse_errors` are settled *before* `tyf` is looked
for, so a diff of `pyproject.toml` alone answers in a checkout with no `ty`
installed.

## Example

```
$ gerenuk impacted-tests
base main (merge-base 57510ca)
verdict selected

impacted tests (4)
  tests/test_api.py  (whole file)
    ← sample_pkg.cli ← sample_pkg.cli:main ← sample_pkg.service:describe
  tests/test_pipelines.py::test_run_describes_animals_in_order
    ← sample_pkg.pipelines:Enricher.run ← sample_pkg.service:describe
  tests/test_pipelines.py::test_run_on_an_empty_shelter_returns_nothing
    ← sample_pkg.pipelines:Enricher.run ← sample_pkg.service:describe
  tests/test_service.py  (whole file)
    ← sample_pkg.cli ← sample_pkg.cli:main ← sample_pkg.service:describe

5 symbol(s) visited, 3 tyf call(s), 14 ms
```

That is a real run of `make test-impact`, which edits `describe` in the test
fixture. Read the `←` line right to left as "the change, which reaches this,
which the test calls" — it is the **why-chain**. When a selection looks wrong,
the chain names the edge to blame, and `tyf refs <symbol>` confirms it by hand.

Both whole-file entries here are the module-level rule at work:
`cli.py` ends in `if __name__ == "__main__": main()`, so `main` is referenced at
module scope, which makes `sample_pkg.cli` itself a node — and every test file
that imports it is selected wholesale. That is the intended conservatism, and
the chain makes it legible.

`--format json` emits the same run as one object:

```json
{
  "verdict": "selected",
  "reason": null,
  "base": "main",
  "merge_base": "57510ca2b98d1f9e469ed80748a73adca01f3dcc",
  "impacted_tests": [
    {
      "file": "tests/test_api.py",
      "symbol": null,
      "via": ["sample_pkg.cli", "sample_pkg.cli:main"],
      "origin": "sample_pkg.service:describe"
    },
    {
      "file": "tests/test_pipelines.py",
      "symbol": "tests.test_pipelines:test_run_describes_animals_in_order",
      "via": ["sample_pkg.pipelines:Enricher.run"],
      "origin": "sample_pkg.service:describe"
    },
    {
      "file": "tests/test_pipelines.py",
      "symbol": "tests.test_pipelines:test_run_on_an_empty_shelter_returns_nothing",
      "via": ["sample_pkg.pipelines:Enricher.run"],
      "origin": "sample_pkg.service:describe"
    },
    {
      "file": "tests/test_service.py",
      "symbol": null,
      "via": ["sample_pkg.cli", "sample_pkg.cli:main"],
      "origin": "sample_pkg.service:describe"
    }
  ],
  "test_files_changed": [],
  "ignored_symbols": [],
  "stats": {
    "seeds": 1,
    "visited": 5,
    "max_depth_reached": 2,
    "tyf_calls": 3,
    "duration_ms": 14
  },
  "errors": []
}
```

### Reading the identifiers

A symbol id is `module.path:QualName`. **An id with no colon is a module** — it
is what a reference at module scope reaches.

`symbol` is `null` when the *whole file* is selected rather than one test
function, which is what a module-level edge produces. In pytest terms, a
non-null symbol is a `file::function` node id and a null one is just the file.

`via` holds the symbols strictly between the test and `origin`, nearest the test
first. **An empty `via` means the test references the changed symbol directly.**
`origin` is never repeated in `via`; the human renderer joins them back up.

When a whole file is selected, its individually-reached tests are dropped from
the list: the file entry already covers them, and emitting both would hand the
same file to pytest twice.

`test_files_changed` passes straight through from
[`changed-symbols`](changed-symbols.md): a changed test selects itself, and
needs no walking.

## How the walk works

Breadth-first over the reverse reference graph, one `tyf refs` frontier at a
time, on the working tree only.

**Seeds** are every entry in `changed_symbols`. `ignored_symbols` are never
seeded — that is what phase 1's filter is for. Each `module_level_changes` entry
seeds both the module's own top-level definitions and the module itself.

**Each reference found** is classified by what owns its line:

| The reference is | What happens |
|---|---|
| In a test file, inside a test function | Recorded as an impacted test. The walk stops there. |
| In a test file, at module scope | The whole file is recorded (`symbol: null`). |
| A plain `import` / `from … import` line | **Dropped.** |
| Inside a definition | Mapped to that definition and expanded next round. |
| Inside a symbol carrying an ignored decorator | Recorded under `ignored_symbols`. Not expanded. |
| At module scope, not an import | The module is the node; test files importing it are selected. |

Dropping import lines is the single biggest precision win in the design. The
usages inside an importing module show up as their own references, so the import
line adds nothing — and keeping it would select every test that so much as
imports the module.

**Deleted symbols** are the one case a type checker cannot help with: there is
no definition left to resolve references to. Those fall back to a word-boundary
textual scan of the workspace's Python files for the bare name, and the walk
continues normally from whatever encloses each hit. This over-matches —
comments, docstrings, same-named locals — deliberately: deletions are rare per
commit, over-selection is safe, and the `via` chain shows what happened.

Because [renames are not paired](changed-symbols.md#added-modified-deleted), a
moved module walks precisely on the `added` half and coarsely on the `deleted`
half.

## Budgets

A pre-commit hook must never hang and never silently under-select. Three limits
back that up, each settable on the command line or in `pyproject.toml`:

```toml
[tool.gerenuk]
max-depth = 10      # BFS levels past the seeds
max-symbols = 500   # nodes visited
budget-ms = 30000   # wall clock; 0 disables it
```

A flag beats the config file, which beats the built-in default. Tripping any of
them produces `run_all`, with whatever was found so far still listed.

`max-depth` counts levels *beyond* the seed frontier, which is always expanded:
`--max-depth 0` still resolves the changed symbols' own references, and the
default of `10` walks ten hops out from them.

## Replaying a saved report

```sh
gerenuk changed-symbols --format json > changed.json
gerenuk impacted-tests --changed changed.json
```

`--changed` walks a saved phase-1 report instead of diffing the working tree —
useful for debugging a selection, and for replaying a diff the tree no longer
has. It conflicts with `--base`, since the saved report already records the base
it was taken against.

## Performance

One `tyf refs` call per BFS level — the whole frontier goes in one invocation —
against a warm `ty` daemon. `stats.tyf_calls` counts those invocations, so it is
the depth of the walk rather than its width.

The first call in a session pays `ty`'s cold start (roughly one to two seconds);
that cost is `tyf`'s, not gerenuk's. The fixture walk above takes about 14 ms
warm.

Symbols are addressed by `file:line:col` position rather than by name, which is
both faster to resolve and unambiguous between same-named symbols in different
modules.

## Known gaps

These are deliberate for this phase, not bugs to rediscover:

- **Re-exports.** A plain import line is dropped: the importing module's own
  uses of the symbol answer for themselves, and following the import too would
  select every module that so much as mentions it. A **renaming** import
  (`from x import y as z`) is followed, because there the uses below say `z` and
  a query about `y` never returns them — see
  [ADR 0013](https://github.com/mojzis/gerenuk/blob/main/docs/adr/0013-a-renaming-import-is-followed.md).
  `__all__`-driven star imports and module-object attribute access are still
  invisible.
- **Module-level execution.** A changed symbol called at import time of module
  `M` selects only the tests that import `M` directly. A non-test module that
  imports `M` and is tested elsewhere is missed.
- **Fixtures.** `conftest.py` is not treated specially, and pytest injects
  fixtures by name rather than by reference, so a fixture is a dead end for the
  walk. Fixture-aware expansion is phase 3.
- **Runtime registries.** Decorator registration is followed
  ([ADR 0012](https://github.com/mojzis/gerenuk/blob/main/docs/adr/0012-a-decorator-is-a-reference.md)),
  but a registry populated by a plain call at runtime, `getattr` dispatch and
  `globals()` lookups are not.
- **Non-ASCII before a definition's name.** Positions are byte columns. Every
  prefix Python allows before a `def`/`class` name is ASCII, so this is
  theoretical — but a wrong column resolves to *no* references rather than to an
  error.

## Exit codes

`0` for either verdict — `run_all` is an answer, not a failure. `2` only when no
verdict could be produced at all: not a git repository, or a `--changed` file
that cannot be read or parsed. Never `1`.
