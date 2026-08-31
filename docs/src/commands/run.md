# `gerenuk run`

Run pytest on exactly the tests the working tree's changes impact.

```
gerenuk run [--base <REF> | --impact <FILE>]
            [--max-depth <N>] [--max-symbols <N>] [--budget-ms <MS>]
            [--dry-run] [-- <pytest args>…]
```

This is the third and last stage. [`changed-symbols`](changed-symbols.md)
answers *what changed*, [`impacted-tests`](impacted-tests.md) answers *what
could break*, and this runs it. The impact report is computed in-process — one
command, no intermediate files, never a subprocess of itself.

It needs `git`, `tyf` and pytest.

## Why this is a command and not a list of node ids

A selection has three possible answers, and an argument list can only express
two of them:

| Outcome | pytest argv | Trap |
|---|---|---|
| run these | `pytest a.py::t1 b.py` | — |
| run everything | `pytest` | — |
| run **nothing** | *(there is none)* | an empty argv **is** "run everything" |

So `pytest $(gerenuk print-node-ids)` would invert the best case: the one diff
that impacts no tests at all would run the entire suite. The empty selection has
to short-circuit before a shell ever interpolates it.

| Verdict | What happens | Exit |
|---|---|---|
| `selected`, non-empty | `pytest <node ids> <passthrough>` | pytest's |
| `selected`, empty | **nothing is spawned**, one line to stderr | `0` |
| `run_all` (any reason) | `pytest <passthrough>` — the full suite | pytest's |

The `run_all` row is [the safety
argument](impacted-tests.md#the-verdict) carried to its conclusion: gerenuk
degrading mid-walk still produces a correct hook, just not a fast one.

## Example

```
$ gerenuk run -- -q
gerenuk: 5 node id(s) from 1 origin(s) in 152 ms — details: gerenuk impacted-tests
.........                                                                [100%]
9 passed in 0.02s
```

gerenuk says one line for itself and then *becomes* pytest — Ctrl-C, colours and
the exit code are pytest's own, because the process is pytest. The other two
outcomes announce themselves the same way:

```
gerenuk: full suite — non-Python files changed
gerenuk: no tests impacted — nothing to run
```

## `--dry-run`

Prints the decision and the exact argv, and spawns nothing:

```
$ gerenuk run --dry-run
decision: selected
gerenuk: 5 node id(s) from 1 origin(s) in 152 ms — details: gerenuk impacted-tests

tests/test_api.py
  ← sample_pkg.cli ← sample_pkg.cli:main ← sample_pkg.service:describe
tests/test_fixtures.py::test_described_marks_the_senior
  ← tests.conftest:described ← sample_pkg.service:describe
tests/test_pipelines.py::test_run_describes_animals_in_order
  ← sample_pkg.pipelines:Enricher.run ← sample_pkg.service:describe

expanded tests.conftest:described (fixture) → 1 node id(s)

argv
  uv
  run
  pytest
  tests/test_api.py
  tests/test_fixtures.py::test_described_marks_the_senior
  tests/test_pipelines.py::test_run_describes_animals_in_order
```

One argv element per line, deliberately: it is for reading, not for `$(…)`.

`--format json` emits the selection as one object — the verdict and reason
carried through from the impact report, plus `node_ids`, `expanded`, `dropped`
and the assembled `argv`:

```json
{
  "verdict": "selected",
  "reason": null,
  "decision": "selected",
  "node_ids": [
    {
      "node_id": "tests/test_fixtures.py::test_described_marks_the_senior",
      "via": ["tests.conftest:described"],
      "origin": "sample_pkg.service:describe"
    }
  ],
  "expanded": [
    {
      "from": "tests.conftest:described",
      "kind": "fixture",
      "into": ["tests/test_fixtures.py::test_described_marks_the_senior"]
    }
  ],
  "dropped": [],
  "argv": ["uv", "run", "pytest", "tests/test_fixtures.py::test_described_marks_the_senior"]
}
```

`decision` is the three-way outcome (`selected` / `run_all` / `nothing`);
`verdict` is the impact report's, unchanged.

## From symbol to node id

`impacted_tests[].symbol` was designed to be node-id-shaped. Turning it into one
is mostly mechanical and entirely about pytest's collection rules.

- `symbol: null` → the file path *is* the node id.
- `tests.test_x:test_fn` → `tests/test_x.py::test_fn`. The file is taken as-is
  and the qualified name's dots become `::`, so
  `tests.test_x:TestFoo.test_bar` → `tests/test_x.py::TestFoo::test_bar`.
- Parametrised tests need nothing: `file::test_fn` selects every
  parametrisation, which is the right grain for impact selection.

### The collectibility gate

The walk records the *enclosing* symbol of a reference, which is frequently a
helper or a fixture rather than a test. Handing pytest a non-collectible node id
is a usage error (exit `4`) that fails the entire run, so each qualified name is
checked segment by segment against pytest's defaults — `test*` functions,
`Test*` classes with no `__init__`:

| The symbol is | Node id |
|---|---|
| Collectible throughout | `file::seg::seg` |
| Collectible up to a point (`TestFoo.helper`) | trimmed to `file::TestFoo` |
| Not collectible at all, and a fixture | [fixture expansion](#fixture-awareness) |
| Not collectible at all, anything else | the **whole file** |

Every degrade is towards *more* tests. A file pytest collects nothing from at
all — `__init__.py`, a helper module, a `conftest.py` on its own — is dropped
instead, because selecting it would mean exit code `5`.

pytest's ini-level overrides (`python_functions = check_*`) are not read; see
[Known gaps](#known-gaps).

## Fixture awareness

pytest injects fixtures by **name**, not by reference, so a fixture is invisible
to `tyf` from the consuming side — the walk dead-ends at the fixture's
definition. Two failure modes follow directly, and both under-select silently,
which is the one thing the design promises never to do:

1. A changed symbol referenced from a fixture body is recorded as
   `tests.conftest:shelter`. That is not collectible, and `conftest.py` collects
   zero tests, so the whole-file fallback would select nothing.
2. A directly edited `conftest.py` lands in `test_files_changed`, and "the file
   selects itself" again selects nothing.

So gerenuk resolves the name edge itself, with tree-sitter and no `tyf`:

**Recognising a fixture.** A def whose decorators suffix-match `pytest.fixture`
or `fixture` — the same syntactic matcher `ignore-decorators` uses, with import
aliases deliberately unresolved. The fixture's name is the def's name unless a
string-literal `name="…"` overrides it; `autouse=True` is recorded.

**Scope.** A fixture in a test module is visible in that module. A fixture in a
`conftest.py` is visible in every test file in that directory's subtree. A
module-level fixture shadows a `conftest.py` one of the same name — pytest's own
rule — and a nearer `conftest.py` shadows a further one. pytest's override
idiom, `def shelter(shelter)` in a nearer `conftest.py`, still runs the fixture
it shadows, so the shadowed one still reaches that subtree.

**Consumers**, within that scope:

- test functions and methods whose parameter list names the fixture;
- anything carrying `@pytest.mark.usefixtures("name")` with string-literal
  arguments, and the methods of a `Test*` class so decorated;
- other fixtures whose parameters name it — followed transitively, with cycles
  terminating on the visited set;
- every test in scope when the fixture is `autouse=True`.

**Coarse fallbacks**, where precision is not available:

| Situation | Selection |
|---|---|
| Fixture with a dynamic `name=` | every test file in its scope |
| A changed `conftest.py` | every test file in its subtree |
| A `conftest.py` that cannot be parsed | every test file in its subtree |

An expanded entry keeps its audit trail: the fixture's symbol id is prepended to
the why-chain, so the output reads

```
tests/test_service.py::test_summary
  ← tests.conftest:shelter ← sample_pkg.service:describe
```

and `tests.conftest:shelter` is a real symbol id you can hand to `tyf refs`.

## Changed test files, filtered

`test_files_changed` arrives from phase 1 unfiltered; this is where it is
resolved:

- entries the working tree no longer has → dropped;
- `conftest.py` entries → subtree expansion, never a node id;
- anything pytest collects no tests from → dropped;
- everything else → the file is its own node id.

After mapping and expansion, whole-file entries supersede their own per-test
node ids — the same collapse [the closure
applies](impacted-tests.md#reading-the-identifiers), re-run because expansion
can introduce new whole-file entries. Superseded ids appear under `dropped`.

## Finding pytest

First of: `GERENUK_PYTEST`, then `pytest-command` in `pyproject.toml`, then
`pytest` on `PATH`. The config key is an argv rather than a string, because the
common real-world value is a multi-word runner:

```toml
[tool.gerenuk]
pytest-command = ["uv", "run", "pytest"]
```

`GERENUK_PYTEST` names a *single* executable, matching `GERENUK_TYF` and
`GERENUK_GIT`; use `pytest-command` for anything with arguments.

pytest is invoked from the **repository root**, because the node ids gerenuk
hands it are repository-relative. Passthrough paths are interpreted from there
too, and so is pytest's own `rootdir` and ini discovery — which is a difference
from running `pytest` yourself in a subdirectory.

Everything after `--` is appended to the argv verbatim — `-x`, `-k`, `-n auto`,
whatever. gerenuk has no opinion about ordering, parallelism or `--failed-first`.

## Replaying a saved report

```sh
gerenuk impacted-tests --format json > impact.json
gerenuk run --impact impact.json
```

`--impact` maps a saved phase-2 report instead of walking the tree. It is parsed
strictly — every field required, unknown keys rejected — because a report that
half-parses becomes a confident selection of the wrong tests. It conflicts with
`--base` and the budget flags, which belong to the walk that already happened.

## Exit codes

Once pytest starts, the exit code is **pytest's**, verbatim: `0` green, `1`
failures, and `2`–`5` mean what pytest says they mean. That is the hook
contract.

Before any spawn, gerenuk's own operational failures exit `2` as everywhere
else, and there is no ambiguity because pytest never ran. The empty selection
exits `0` — deliberately indistinguishable from a green suite, because for a
hook that is exactly what it is.

## Known gaps

- **Collection-convention drift.** A repo overriding `python_functions` or
  `python_classes` makes the collectibility gate wrong in both directions. The
  gate mirrors pytest's defaults; a mismatch degrades to whole-file.
- **Plugin-provided fixtures.** A fixture living in an installed plugin, or
  reached through `pytest_plugins`, is invisible, and its consumers dead-end
  unexpanded. The walk still reaches the *definition* module when it is in the
  repo, so the miss is narrower than it sounds.
- **`usefixtures` beyond string literals** — variables, `pytestmark` lists —
  is not read, so those consumers are not found by name. A `usefixtures` mark
  on any *enclosing* class is read, nested classes included.
- **Deep fixture-override chains.** `def shelter(shelter)` in a nearer
  `conftest.py` is followed, so a change to the shadowed fixture still selects
  the overriding subtree. The chain is followed by name through successive
  `conftest.py` files; an override that shadows without requesting the name it
  shadows correctly ends it.
- **`indirect=` parametrisation** and dynamically generated fixtures are not
  modelled.
- **Exit-code aliasing.** After the exec, gerenuk cannot distinguish its own
  never-happened failures from pytest's `2`. Every pre-exec failure has already
  exited before pytest existed, so this only matters when reading a log after
  the fact.
