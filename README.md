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

**Prerequisite:** [ty-find](https://github.com/mojzis/ty-find) and
[ty](https://github.com/astral-sh/ty).

```sh
uv add --dev ty ty-find gerenuk
```

If `ty` is not on `PATH`, `tyf` falls back to `uvx ty`, so having `uv` is enough.

Verify the setup:

```sh
gerenuk doctor
```

## Usage

```sh
gerenuk audit pkg/module.py            # human output
gerenuk audit --format json pkg/*.py   # machine-readable
gerenuk audit --workspace ../other pkg/module.py
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
  mypkg.pipelines.enrich
```

`--base` defaults to the first of `origin/main`, `main`, `master` that exists;
the diff runs from `merge-base(HEAD, base)` to the working tree, staged and
unstaged alike. `--format json` emits the same data for scripting. Registry
decorators can be filtered out via `[tool.gerenuk] ignore-decorators` in
`pyproject.toml`. See the
[documentation](https://mojzis.github.io/gerenuk/commands/changed-symbols.html).

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
- `gerenuk doctor` — check that `tyf` and the workspace resolve

Exit codes: `0` clean, `1` findings reported, `2` the run could not complete.

Findings are signals, not verdicts: dynamic dispatch, plugin registries, and
`__all__` re-exports can hide a real usage. Confirm with `tyf refs <symbol>`
before deleting anything.
```
<!-- END SHARED:claude-snippet -->

## Development

```sh
make review        # fmt + clippy + rust tests + fixture pytest + audit + deny
make review-quick  # skip the network checks
make docs          # build the mdBook site and llms.txt into docs/book/html
```

The test suite is hermetic: `tests/common/mod.rs` stubs `tyf` with a shell
script and points `GERENUK_TYF` at it, so neither `tyf` nor `ty` is needed to
run `cargo test`. See [`docs/dev/ARCHITECTURE.md`](docs/dev/ARCHITECTURE.md).

## License

MIT
