# `gerenuk changed-symbols`

Map the working tree's diff to the Python symbols it changed.

```
gerenuk changed-symbols [--base <REF>]
```

Unlike [`audit`](audit.md), this command needs only `git` — no `tyf`, no `ty`,
no Python environment. It is the first stage of impact-based test selection:
[`impacted-tests`](impacted-tests.md) walks the reference graph from these
symbols out to the tests that reach them.

`git` is taken from `PATH` unless `GERENUK_GIT` points at a specific binary.

## Example

```
$ gerenuk changed-symbols
base main (merge-base 5ddda1f)

changed symbols (2)
  modified  method    mypkg.pipelines.enrich:Enricher.run  src/mypkg/pipelines/enrich.py
  added     function  mypkg.utils:parse_date               src/mypkg/utils.py

ignored symbols (1)
  modified  function  mypkg.daily:normalize_prices         src/mypkg/daily.py  (@transformation)

module-level changes (1)
  mypkg.pipelines.enrich  src/mypkg/pipelines/enrich.py

non-Python changes (1)
  schema.sql
```

`--format json` emits the same information as a single object:

```json
{
  "base": "main",
  "merge_base": "5ddda1f2cd66afdf789771b20a3bf667df78f050",
  "changed_symbols": [
    {
      "symbol": "mypkg.pipelines.enrich:Enricher.run",
      "file": "src/mypkg/pipelines/enrich.py",
      "kind": "method",
      "line": 42,
      "column": 9,
      "change": "modified"
    }
  ],
  "ignored_symbols": [
    {
      "symbol": "mypkg.daily:normalize_prices",
      "file": "src/mypkg/daily.py",
      "kind": "function",
      "line": 17,
      "column": 5,
      "change": "modified",
      "ignored_by": "transformation"
    }
  ],
  "module_level_changes": [
    {
      "module": "mypkg.pipelines.enrich",
      "file": "src/mypkg/pipelines/enrich.py"
    }
  ],
  "non_python_changes": ["schema.sql"],
  "test_files_changed": [],
  "errors": []
}
```

`kind` is `function`, `method` or `class`; `change` is `added`, `modified` or
`deleted`; `line` and `column` are where the definition's *name* starts, not its
first decorator, and both are 1-based. Every array is sorted, so two runs on the
same tree produce byte-identical output.

The schema is the interface between the phases:
`impacted-tests --changed report.json` replays a saved report instead of
diffing the working tree, which is why each entry carries enough to be walked
from — a definition's exact position, and a file for every module-level change.
Every field above is required: a saved report is parsed strictly, so one that
omits `column` is rejected rather than half-read.

## What gets diffed

The range is `merge-base(HEAD, <base>)` → **working tree**: staged and unstaged
edits together, plus untracked files that `.gitignore` does not cover. Taking
the merge base rather than the branch tip means work landing on `main` after you
branched is not attributed to you.

`--base` defaults to the first of `origin/main`, `main`, `master` that exists.
A `--base` you pass explicitly is used as given; if it does not resolve, the run
fails with exit code `2` rather than falling back.

## How lines become symbols

Changed line ranges come from `git diff -U0`, so there is no context to
misattribute. Each line is mapped to the innermost definition that encloses it,
parsed with `tree-sitter-python`.

| Changed line | Attributed to |
|---|---|
| Anywhere in a function or method body, its signature, or its default values | that function or method |
| A decorator | the definition it decorates |
| A class body, outside any method (attributes, dataclass fields) | the class |
| Inside a nested function or closure | the enclosing **named** function |
| Inside a nested class | `Outer.Inner`, and its methods `Outer.Inner.method` |
| Module level — imports, constants, module-level statements | `module_level_changes` |

Blank lines are the one exception: they carry no meaning, and appending a
function inserts two of them, so they are not treated as module-level changes.
Docstring and comment edits *are* attributed normally — phase 1 does not try to
detect semantic no-ops.

Symbols are named `module.path:QualName`. The module path is the unbroken chain
of directories above the file that contain `__init__.py`, which resolves
src-layout and flat layout identically. PEP 420 namespace packages fall back to
the file path with a leading `src/` removed.

## Added, modified, deleted

A symbol's verdict comes from whether it *exists* on each side of the diff, not
from which side the hunk touched:

| Old side | New side | Verdict |
|---|---|---|
| present | present | `modified` |
| present | absent | `deleted` |
| absent | present | `added` |

So deleting lines from a function that still exists is a modification — its
callers were never orphaned. Deletions are found by parsing the old blob out of
git, which is why a removed function can still be named.

**Renames are not paired.** `git` detects them, but a moved module is a new
module: every symbol in the old path is reported `deleted` and every symbol in
the new path `added`, so phase 2 still revisits the callers of the old name.

## Files that are not analysed

Checked in this order, so the first row that matches wins:

| Kind | Where it goes |
|---|---|
| Anything that is not `.py`, including `.pyi` stubs and binaries | `non_python_changes` |
| Test files — a `tests`/`test` directory, or `test_*.py` / `*_test.py` | `test_files_changed` |
| Files that do not parse | `errors`, and the module is reported as module-level |

The order matters for the phases downstream: a non-Python file *under* `tests/`
— a fixture `.json`, say — is a non-Python change, which makes
`impacted-tests` answer `run_all`. A changed test file needs no symbol analysis:
it will select itself later.

## Ignoring registry decorators

Registry-style functions — registered by a decorator and invoked by a runner —
are neither called directly nor covered by targeted tests, so chasing their
callers is wasted work. List their decorators in the repository root's
`pyproject.toml`:

```toml
[tool.gerenuk]
ignore-decorators = ["transformation", "registry.task"]
```

Matching is syntactic dotted-suffix matching on the decorator expression, with
or without call parentheses:

| Config entry | Matches | Does not match |
|---|---|---|
| `transformation` | `@transformation`, `@transformation(...)`, `@registry.transformation` | `@transform` |
| `registry.transformation` | `@registry.transformation`, `@a.registry.transformation` | `@transformation` |

A matching decorator moves the symbol to `ignored_symbols` with the entry that
matched, rather than dropping it. One match is enough: a symbol carrying both a
matching and a non-matching decorator is still ignored. Ignoring applies to the
decorated definition only, never to its neighbours.

**Limitation:** import aliases are not resolved. `from registry import
transformation as t` followed by `@t` will not match, because the check never
leaves the file's syntax.

## Exit codes

`0` on any successful run, including one that reports hundreds of symbols — the
output is an inventory, not a verdict. `2` when the run could not complete: not
a git repository, or a `--base` that does not resolve.
