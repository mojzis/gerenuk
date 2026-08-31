# `gerenuk audit`

Report symbols that nothing references, and symbols only tests reach.

```
gerenuk audit [OPTIONS] <FILE>...
```

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
