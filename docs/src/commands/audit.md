# `gerenuk audit`

Report symbols that nothing references, and symbols only tests reach.

```
gerenuk audit [OPTIONS] <FILE>...
```

`audit` is the companion to the selection pipeline, not the product: the walk
that [`impacted-tests`](impacted-tests.md) does already holds a resolved
reference graph, and asking it *what does nothing reach?* is one more query
against the same graph.

## A verifier, not a repo-wide sweep

[vulture](https://github.com/jendrikseipp/vulture) is the cheap sweep for dead
code: one pass over a whole project, no type checker involved, nothing to
install beyond vulture itself. It works on **names**, though, which sets its
limits — it cannot tell two same-named symbols apart, it flags dynamic and
framework code that is very much alive, and it cannot tell you *who* references
a symbol.

`gerenuk audit` asks `ty`'s type checker instead, through `tyf`. References
resolve the way Python resolves them, and each one comes back as a file and a
line. That costs one `tyf refs` call per symbol and needs `tyf` installed, which
is why `audit` takes the files you name rather than a repository — it is shaped
for confirming a specific suspicion, not for finding one.

So run them in that order:

```sh
vulture src/                    # sweep: what might be dead
gerenuk audit src/suspect.py    # confirm the file against resolved references
tyf refs one_symbol             # or confirm a single name
# then delete
```

### Referenced only from tests

The `note` severity is the finding a name-based sweep cannot produce at all:
every reference to the symbol exists, and every one of them is in a test file.
Production stopped calling it and its own tests are what keep it alive —
usually the residue of an unfinished refactor, and exactly the code that
survives a dead-code scan forever. That finding is why `audit` earns a place
next to a repo-wide scanner rather than deferring to one entirely.

### What it cannot see

`gerenuk` reports **static** references, plus the three edges a type checker
cannot draw and gerenuk models explicitly: `conftest.py` fixtures, registering
decorators, and renaming imports (`from x import y as z`, which `tyf` answers
for under `y` while the code says `z`). Everything else dynamic — registries
populated at runtime, `getattr` dispatch, `__all__` star re-exports — stays
invisible.

So findings are leads, not a delete list. Confirm with `tyf refs <symbol>`
before deleting anything, and see
[Troubleshooting](../troubleshooting.md#a-symbol-is-flagged-but-is-definitely-used)
for the shapes that are flagged but alive.

## Example

```
$ gerenuk audit sample_pkg/service.py
warn  sample_pkg/service.py:43  func `legacy_export` has no references
note  sample_pkg/service.py:34  method `ShelterService.seniors` is referenced only from tests (1)

1 file(s), 7 symbol(s) checked — 1 warn, 1 note
```

Locations are relative to the workspace root and one-based, so they paste
straight into an editor.

## Severities

| Severity | Rule |
|---|---|
| `warn` | The symbol has no references anywhere — production or test |
| `note` | Every reference lives in a test file |

A file counts as a test when its path contains a `tests/` or `test/`
directory, or its name matches `test_*.py` / `*_test.py`.

## What gets audited

Only **callable** symbols — functions and methods. Within those, three groups
are skipped deliberately:

- names starting with `_`, including dunder methods: private by convention, or
  invoked implicitly by the interpreter;
- classes, variables and constants: their reference patterns are noisy enough
  that flagging them produces more false positives than signal;
- functions carrying a **registering decorator** — `@app.command()`,
  `@router.get(...)`, `@mcp.tool()`. A framework holds the reference and calls
  them, so "no references" says nothing about whether they are dead. Flagging
  them means flagging every CLI command and every route in the project, which
  buries the findings that are real: on one project this rule alone cut 48
  warnings to 5 while keeping both genuine ones. Decorators that merely wrap
  (`@property`, `@staticmethod`, `@functools.wraps`) do not count, and those
  functions are still audited. See
  [ADR 0012](https://github.com/mojzis/gerenuk/blob/main/docs/adr/0012-a-decorator-is-a-reference.md).

The `N symbol(s) checked` count in the summary reports every symbol in the
outline, including the skipped ones, so you can see how much of the file the
rules actually looked at.

## JSON output

```
$ gerenuk audit --format json sample_pkg/service.py
{
  "files": ["sample_pkg/service.py"],
  "symbols_checked": 7,
  "findings": [
    {
      "symbol": "legacy_export",
      "kind": "Function",
      "file": "/home/you/proj/sample_pkg/service.py",
      "line": 43,
      "severity": "warn",
      "message": "`legacy_export` has no references"
    }
  ]
}
```

JSON keeps **absolute** paths, so downstream tools do not have to know the
workspace root. Human output uses relative ones.

## What counts as a reference

The symbol's own definition does not. Neither does a mention in a docstring,
comment or string literal — `ty` resolves names, so those never appear.

References are classified as production or test by **path, relative to the
workspace root**. That relative part matters if your project itself lives under
a directory named `tests/`: absolute-path heuristics (including `tyf`'s own)
would call every file a test.

## Cost

`audit` runs one `tyf list` per file plus one `tyf refs` per auditable symbol.
On a large module that is a lot of LSP round-trips — pass the files you care
about rather than the whole package.
