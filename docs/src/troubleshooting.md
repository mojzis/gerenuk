# Troubleshooting

## `tyf not found on PATH`

`gerenuk` looks for `tyf` on `PATH` unless `GERENUK_TYF` is set.

```sh
uv add --dev ty-find
# or
GERENUK_TYF=/path/to/tyf gerenuk doctor
```

## `no Python project root above ...`

Nothing above the current directory holds a `pyproject.toml`, `setup.py`,
`setup.cfg`, or `.git`. Either run from inside the project, or pass
`--workspace /path/to/project`.

## `no JSON found in tyf output`

`tyf` answered with human text instead of a JSON payload — usually because the
symbol does not exist, or because `ty` failed to start and `tyf` reported that
in prose. Run the same query by hand to see it:

```sh
tyf --format json refs the_symbol
```

## `ty server did not start`

`tyf` needs `ty`. Install it, or make sure `uvx` is available for `tyf`'s
fallback:

```sh
uv add --dev ty
```

## The audit is slow

`audit` makes one `tyf refs` call per auditable symbol. A module with fifty
public functions means fifty LSP round-trips. Narrow the input:

```sh
gerenuk audit pkg/the_one_file.py     # not pkg/*.py
```

`tyf` keeps a daemon warm between calls, so the second run on the same project
is much faster than the first.

## A symbol is flagged but is definitely used

Expected, for a few known shapes:

- the caller reaches it through `getattr`, a registry, or a plugin entry point;
- it is re-exported via `__all__` and consumed outside the project;
- it is a framework hook (a pytest fixture, a Django signal receiver) invoked by
  name rather than by reference.

`gerenuk` reports static references only. Confirm with `tyf refs <symbol>`
before deleting.

## A symbol is used only by tests and that is fine

Some helpers legitimately exist for the test suite. The rule emits a `note`
rather than a `warn` for exactly that reason — it is a prompt to look, not a
failure.
