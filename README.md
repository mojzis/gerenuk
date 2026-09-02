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

It gets there by asking `ty`'s type checker, through `tyf`, which symbols the
working tree changed and which tests can reach them — so the selection follows
Python's actual name resolution, not a name match. Anything gerenuk cannot see
through becomes "run the whole suite" rather than a confident short list.

📖 **[Documentation](https://mojzis.github.io/gerenuk)** ·
🤖 [llms.txt](https://mojzis.github.io/gerenuk/llms.txt)

## Install

```sh
uv add --dev "gerenuk[ty]"
```

[ty-find](https://github.com/mojzis/ty-find) comes with `gerenuk` — `tyf` is
what it drives. The `[ty]` extra adds [ty](https://github.com/astral-sh/ty)
itself; without it `tyf` falls back to `uvx ty`, which works whenever `uv` is
around but fetches the checker on first use.

Verify the setup:

```sh
gerenuk doctor
```

## Usage

```sh
gerenuk changed-symbols                # what the working tree changed
gerenuk impacted-tests                 # and which tests that reaches
gerenuk run -- -q                      # and run exactly those, under pytest
gerenuk run --dry-run                  # the decision and the argv, no pytest
gerenuk audit pkg/module.py            # separately: what nothing references
```

The three selection stages are one pipeline, and `run` computes the whole thing
in-process — the first two are there to be read when a selection surprises you.

### `changed-symbols`

Maps the working tree's diff to the Python symbols it changed — the first stage
of impact-based test selection. It needs only `git`; no `tyf`, no `ty`, no
Python environment.

```
$ gerenuk changed-symbols
base main (merge-base 5ddda1f)

changed symbols (2)
  modified  method    mypkg.pipelines.enrich:Enricher.run  src/mypkg/pipelines/enrich.py
  added     function  mypkg.utils:parse_date               src/mypkg/utils.py

module-level changes (1)
  mypkg.pipelines.enrich  src/mypkg/pipelines/enrich.py
```

`--base` defaults to the first of `origin/main`, `main`, `master` that exists;
the diff runs from `merge-base(HEAD, base)` to the working tree, staged and
unstaged alike. `--format json` emits the same data for scripting. Registry
decorators can be filtered out via `[tool.gerenuk] ignore-decorators` in
`pyproject.toml`. See the
[documentation](https://mojzis.github.io/gerenuk/commands/changed-symbols.html).

### `impacted-tests`

Walks the reference graph from those changed symbols out to the tests that
reach them — the selection, minus the `pytest` invocation, which is
[`run`](#run)'s job.

```
$ gerenuk impacted-tests
base main (merge-base 5ddda1f)
verdict selected

impacted tests (2)
  tests/test_enrich.py::test_run_enriches
    ← mypkg.pipelines.enrich:Enricher.run
  tests/test_api.py::test_endpoint_returns_lines
    ← mypkg.api:enrich_endpoint ← mypkg.pipelines.enrich:Enricher.run

41 symbol(s) visited, 5 tyf call(s), 640 ms
```

The `←` chain is the point: when a selection looks wrong, it names the edge to
blame, and `tyf refs <symbol>` confirms it by hand.

Every run emits a **verdict**. `selected` means the list is the answer;
`run_all` means run the whole suite, with a machine-readable `reason` — a
non-Python file changed, a file did not parse, `tyf` was unavailable or failed,
or a budget tripped. Both exit `0`. Under-selecting silently is the one outcome
that would make the tool worse than not having it, so anything gerenuk cannot
see through becomes "run everything" rather than a short list.

See the
[documentation](https://mojzis.github.io/gerenuk/commands/impacted-tests.html).

### `run`

Turns that selection into a pytest invocation and runs it. gerenuk says one line
for itself and then *becomes* pytest, so Ctrl-C, colours and the exit code are
pytest's own.

```
$ gerenuk run -- -q
gerenuk: 5 node id(s) from 1 origin(s) in 152 ms — details: gerenuk impacted-tests
.........                                                                [100%]
9 passed in 0.02s
```

gerenuk owns the invocation rather than printing node ids for a shell to
interpolate, because a selection has three answers and an argument list has only
two: an empty `pytest` argv **is** "run everything". So `pytest $(gerenuk …)`
would run the entire suite in precisely the best case — the diff that impacts no
tests. Here that case spawns nothing and exits `0`.

`run` is also where pytest's own rules get applied. A recorded symbol is often a
helper or a fixture rather than a test, so each name is checked against pytest's
collection conventions and trimmed — or widened to the whole file — until it is
one pytest will accept. And because pytest injects fixtures by *name*, which no
type checker can follow, `conftest.py` is parsed for its fixtures and their
consumers; the expansion keeps its audit trail:

```
tests/test_service.py::test_summary
  ← tests.conftest:shelter ← sample_pkg.service:describe
```

`--dry-run` prints the decision and the exact argv without spawning anything.
`pytest-command = ["uv", "run", "pytest"]` under `[tool.gerenuk]` names the
runner. See the
[documentation](https://mojzis.github.io/gerenuk/commands/run.html).

## Dead code: `audit`

The same reference graph answers the opposite question — *what does nothing
reach?* — so `gerenuk audit` asks it for every symbol in a file:

```
$ gerenuk audit sample_pkg/service.py
warn  sample_pkg/service.py:43  func `legacy_export` has no references
note  sample_pkg/service.py:34  method `ShelterService.seniors` is referenced only from tests (1)

1 file(s), 7 symbol(s) checked — 1 warn, 1 note
```

| Severity | Rule |
|---|---|
| `warn` | The symbol has no references anywhere |
| `note` | Every reference lives in a test file |

Only callable symbols are audited; `_private` and dunder names are skipped.
`--format json` emits the findings for scripting.

### It is a verifier, not a sweep

[vulture](https://github.com/jendrikseipp/vulture) is the cheap repo-wide sweep
for dead code: one pass over a whole project, no type checker involved. It works
on *names*, so it cannot tell two same-named symbols apart, it flags dynamic and
framework code that is very much alive, and it cannot tell you *who* references
a symbol.

`gerenuk audit` asks `ty`'s type checker instead, through `tyf`. References
resolve the way Python resolves them — docstrings, comments and same-named
symbols in other modules do not count — and each one comes back as a file and a
line you can open. That costs one `tyf refs` call per symbol and needs `tyf`
installed, which is why it takes the files you name rather than a whole
repository. It is shaped for confirming a specific suspicion.

The two fit together in that order:

```sh
vulture src/                    # sweep: what might be dead
gerenuk audit src/suspect.py    # confirm the file against resolved references
tyf refs one_symbol             # or confirm a single name
# then delete
```

Exit codes carry the verdict: `0` nothing flagged, `1` findings reported, `2`
the run could not complete. The split between `1` and `2` is what makes it
usable in CI — a failing check and a broken setup are different problems.

### Referenced only from tests

The `note` is the finding a name-based scanner cannot produce at all: every
reference to the symbol exists, and every one of them is in a test file.
Production stopped calling it and its own tests are what keep it alive —
usually the residue of an unfinished refactor, and exactly the code that
survives a dead-code sweep forever. That finding is why `audit` earns a place
next to a repo-wide scanner rather than deferring to one entirely.

### What it cannot see

`gerenuk` reports *static* references, plus the three edges a type checker
cannot draw and gerenuk models explicitly: `conftest.py` fixtures, registering
decorators, and renaming imports (`from x import y as z`, which `tyf` answers
for under `y` while the code says `z`). Everything else dynamic — registries
populated at runtime, `getattr` dispatch, `__all__` star re-exports — stays
invisible.

So findings are leads, not a delete list. Confirm with `tyf refs <symbol>`
before deleting anything.

## Registered functions

Nothing *references* a `@app.command()` or a `@router.get()` — the framework
holds the only handle. So gerenuk follows the decorator to its registrar (`app`,
`router`) and treats what reaches the registrar as reaching the function. A
decorator that only wraps (`@property`, `@functools.wraps`) is left alone, and a
registrar that cannot be resolved gives `run_all` with
`reason: "decorator_dispatch"` rather than a confident empty answer.
See [ADR 0012](docs/adr/0012-a-decorator-is-a-reference.md).

## Usage with Claude Code

Add this to your project's `CLAUDE.md`. Two lines is the whole of it — the exit
codes, the `run_all` verdict and the caveat above are in `gerenuk --help` and on
the [documentation site](https://mojzis.github.io/gerenuk), which is where an
agent that needs them should go:

<!-- BEGIN SHARED:claude-snippet -->
```markdown
### `gerenuk` — test selection and dead code

- `gerenuk run -- -q` runs only the tests the working tree's diff impacts
  (`--dry-run` to inspect; `gerenuk impacted-tests` explains why).
- `gerenuk audit <file>` confirms a symbol vulture flagged is really unused;
  its `only tests reach it` findings are ones vulture cannot produce.
```
<!-- END SHARED:claude-snippet -->

## Development

```sh
make review        # fmt + clippy + rust tests + fixture pytest + audit + deny
make review-quick  # skip the network checks
make test-impact   # impacted-tests against the fixture, with a real tyf
make test-run      # the whole pipeline, with a real tyf and a real pytest
make docs          # build the mdBook site and llms.txt into docs/book/html
```

The test suite is hermetic: `tests/common/mod.rs` stubs `tyf` and pytest with
shell scripts and points `GERENUK_TYF` and `GERENUK_PYTEST` at them, so none of
`tyf`, `ty` or pytest is needed to run `cargo test`. See [`docs/dev/ARCHITECTURE.md`](docs/dev/ARCHITECTURE.md)
for the shape and [`docs/adr/`](docs/adr/README.md) for why it is that shape.

## License

MIT
