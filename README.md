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

Grep tells you whether a name *appears*. `gerenuk` asks `ty`'s type checker
whether it is actually *referenced* — so docstrings, comments and same-named
symbols in other modules do not count.

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
gerenuk audit pkg/module.py            # human output
gerenuk audit --format json pkg/*.py   # machine-readable
gerenuk audit --workspace ../other pkg/module.py
gerenuk changed-symbols                # what the working tree changed
gerenuk impacted-tests                 # and which tests that reaches
```

| Severity | Rule |
|---|---|
| `warn` | The symbol has no references anywhere |
| `note` | Every reference lives in a test file |

Only callable symbols are audited; `_private` and dunder names are skipped.

| Exit code | Meaning |
|---|---|
| `0` | Nothing flagged |
| `1` | Findings reported |
| `2` | The run could not complete (`tyf` missing, bad workspace, …) |

The split between `1` and `2` is what makes it usable in CI: a failing check and
a broken setup are different problems.

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
reach them — impact-based test selection, minus the `pytest` invocation.

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

### Caveat

`gerenuk` reports *static* references. Dynamic dispatch, plugin registries,
`getattr` lookups and `__all__` re-exports are invisible to it. Findings are
leads to confirm with `tyf refs <symbol>`, not a delete list.

## Usage with Claude Code

Add this to your project's `CLAUDE.md`:

<!-- BEGIN SHARED:claude-snippet -->
```markdown
### Dead-symbol checks — `gerenuk`

This project has `gerenuk` — a symbol-level auditor built on `tyf` (ty-find).
Run it before deleting Python code, and after a refactor.

- `gerenuk audit pkg/module.py` — flag symbols nothing references, and symbols
  only tests reach
- `gerenuk audit --format json pkg/*.py` — same, machine-readable
- `gerenuk changed-symbols` — which Python symbols the working tree changed
- `gerenuk impacted-tests` — which tests those changed symbols can reach
- `gerenuk doctor` — check that `tyf` and the workspace resolve

Exit codes: `0` clean, `1` findings reported, `2` the run could not complete.
`changed-symbols` and `impacted-tests` never return `1`. When `impacted-tests`
cannot trust its answer it reports `"verdict": "run_all"` with a `reason` and
still exits `0` — read the verdict, not the exit code.

Findings are signals, not verdicts: dynamic dispatch, plugin registries, and
`__all__` re-exports can hide a real usage. Confirm with `tyf refs <symbol>`
before deleting anything.
```
<!-- END SHARED:claude-snippet -->

## Development

```sh
make review        # fmt + clippy + rust tests + fixture pytest + audit + deny
make review-quick  # skip the network checks
make test-impact   # impacted-tests against the fixture, with a real tyf
make docs          # build the mdBook site and llms.txt into docs/book/html
```

The test suite is hermetic: `tests/common/mod.rs` stubs `tyf` with a shell
script and points `GERENUK_TYF` at it, so neither `tyf` nor `ty` is needed to
run `cargo test`. See [`docs/dev/ARCHITECTURE.md`](docs/dev/ARCHITECTURE.md)
for the shape and [`docs/adr/`](docs/adr/README.md) for why it is that shape.

## License

MIT
