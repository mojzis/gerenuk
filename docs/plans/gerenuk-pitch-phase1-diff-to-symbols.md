# gerenuk — impact-based pytest selection (Phase 1: diff → changed symbols)

> Named after the gerenuk, the savanna's most selective browser: it stands on its hind
> legs and eats exactly the leaves it chose, while everyone else grazes the whole field.
> Use this line of thinking for the README tone: precise, a little dry, no mascot kitsch.

## Project context (for orientation only — do NOT build phases 2+ now)

`gerenuk` is a Rust CLI that will power a git pre-commit hook for Python repos: instead of
running the full pytest suite on every commit, it determines which tests are affected by
the current change and runs only those. CI still runs the full suite; under-selection is
acceptable, so we optimize for precision and speed, not perfect recall.

Planned phases:

1. **Diff → changed symbols** (THIS PHASE): map the git diff to a set of changed Python
   symbols (functions/methods/classes) plus coarse-grained signals.
2. Reverse reference closure: walk callers-of-callers via the `ty` LSP (through an
   existing local daemon, `tyf`) until reaching test symbols.
3. Test selection & pytest invocation: emit `pytest` node IDs, fixture-aware expansion,
   selection summary output.
4. Pre-commit integration, telemetry (compare vs. file-level import closure).

Phase 1 must be useful standalone: a CLI subcommand that prints changed symbols as JSON,
testable against real repos.

## Phase 1 deliverable

A Rust binary crate `gerenuk` with one working subcommand:

```
gerenuk changed-symbols [--base <ref>] [--repo <path>] [--json|--pretty]
```

Behavior:

- Determine the diff range: `merge-base(HEAD, <base>)` → working tree (staged AND
  unstaged; the hook will later run on staged, but phase 1 takes everything). Default
  `--base origin/main`, fall back to `main`, then `master`; error clearly if none exist.
- Collect changed files. Partition into:
  - Python files inside the package(s) or tests → symbol analysis
  - everything else → reported as `non_python_changes` (later triggers full-run bail-out)
- For each changed Python file, compute changed line ranges from the diff hunks and map
  them to enclosing symbols using **tree-sitter** (`tree-sitter-python`). Do not shell
  out to Python; do not use the `ty` daemon in this phase.
- Print a JSON report (schema below).

## Symbol mapping rules

Symbols are identified as `module.path:QualName`, e.g.
`mypkg.pipelines.enrich:Enricher.run`, `mypkg.utils:parse_ts`. Module path derives from
the file path relative to the package root (support both src-layout `src/<pkg>/...` and
flat layout; detect via `pyproject.toml` if present, else heuristic: directory containing
`__init__.py` chains).

Mapping a changed line to a symbol:

- Line inside a function/method body (including its signature and default-value
  expressions) → that function/method.
- Line on a decorator → the decorated function/class.
- Line in a class body but outside any method (class attributes, dataclass fields) → the
  class itself.
- Line at module level (imports, constants, module-level code) → the module, reported as
  a coarse `module_level` change (phase 2 will treat this as "whole module changed").
- Nested functions/classes → attribute to the outermost enclosing top-level definition
  AND record the full qualname (`outer.<locals>.inner` is not needed; just
  `Outer.inner`-style qualnames for classes, and for closures attribute to the enclosing
  named function).
- Docstring-only or comment-only changes → still attribute normally (do not try to be
  clever about "semantic no-ops" in phase 1; note it as a possible future flag).

**Ignored decorators.** The monorepo contains many registry-style functions (e.g. data
transformations) that are registered via a decorator and invoked by a runner — they are
neither called directly nor covered by targeted tests. Changes to them must not produce
noise or wasted closure work:

- Config key `ignore-decorators` (see Configuration below) holds a list of dotted names.
- A changed function/method/class whose definition carries a matching decorator is NOT
  added to `changed_symbols`; it goes to `ignored_symbols` instead, with the matched
  decorator recorded.
- Matching is syntactic suffix-matching on the decorator expression's dotted name, with
  or without call parentheses: config entry `transformation` matches `@transformation`,
  `@transformation(...)`, and `@registry.transformation`; config entry
  `registry.transformation` matches only the latter two. Import aliases
  (`from x import transformation as t; @t`) are NOT resolved — document this limitation.
- If a symbol has both a matching and a non-matching decorator, it is still ignored (the
  registry decorator wins).
- Ignoring applies to the decorated symbol only, not to other symbols in the same file.

**Deletions and renames matter.** A deleted function's callers must be selected later, so:

- Addition/modification hunks are mapped against the **new** file content (working tree).
- Deletion hunks are mapped against the **old** blob (`git show <merge-base>:<path>`),
  parsed separately with tree-sitter.
- A symbol present in both old and new mapping is `modified`; only-old is `deleted`;
  only-new is `added`. Renames (git rename detection) produce a `deleted` + `added` pair
  — do not attempt rename pairing in phase 1.

## Output schema

```json
{
  "base": "origin/main",
  "merge_base": "<sha>",
  "changed_symbols": [
    {
      "symbol": "mypkg.pipelines.enrich:Enricher.run",
      "file": "src/mypkg/pipelines/enrich.py",
      "kind": "method",
      "change": "modified"
    }
  ],
  "ignored_symbols": [
    {
      "symbol": "mypkg.transforms.daily:normalize_prices",
      "file": "src/mypkg/transforms/daily.py",
      "kind": "function",
      "change": "modified",
      "ignored_by": "transformation"
    }
  ],
  "module_level_changes": ["mypkg.settings"],
  "non_python_changes": ["pyproject.toml", "data/schema.sql"],
  "test_files_changed": ["tests/test_enrich.py"],
  "errors": []
}
```

- `kind`: `function | method | class | module`
- `change`: `added | modified | deleted`
- Changed files under a test directory (`tests/` or matching `test_*.py` /
  `*_test.py`) are listed in `test_files_changed` and are NOT symbol-analyzed (a changed
  test file simply selects itself later).
- Parse failures (syntax errors in working tree) must not crash the tool: report the file
  in `errors` and treat it as a `module_level` change.

## Implementation notes / constraints

- Rust 2021+, use `tree-sitter` + `tree-sitter-python` crates; `gix` or shelling out to
  `git` CLI is acceptable for diff/merge-base (prefer shelling out for correctness and
  simplicity in phase 1 — parse `git diff -U0 --no-color -M <merge-base>` output, and
  `--name-status` for the file partition).
- Deterministic output: sort all arrays.
- Fast: target < 100ms on a diff touching ~20 files (excluding git subprocess time).
- Errors to stderr, JSON to stdout, exit code 0 on success even when the diff is empty
  (empty arrays), non-zero only on operational failure (not a git repo, base not found).

## Configuration (minimal)

Read `[tool.gerenuk]` from the repo root `pyproject.toml` (no separate config file):

```toml
[tool.gerenuk]
ignore-decorators = ["transformation", "registry.transformation"]
```

- Missing table or file → empty ignore list, everything else defaulted.
- This is the only config key in phase 1. Keep the config loader trivially extensible
  (later phases add test-dir patterns, base ref, etc.) but do not add unused keys now.

## Testing

- Unit tests for the line→symbol mapper using inline Python source strings + synthetic
  line ranges (cover every rule above: decorators, class attrs, nested defs, async defs,
  overloads, module level, deletions via old-blob parsing).
- Integration tests: create temporary git repos in tests (`tempfile` + `git` CLI), commit
  a base state, apply modifications, assert full JSON output. Cover: modify body, add
  function, delete function, rename file, change non-Python file, syntax error in
  working tree, src-layout and flat layout.
- One end-to-end snapshot test with `insta` against a small fixture repo.
- Ignore-decorator tests: bare vs. called vs. dotted decorator forms, suffix matching,
  mixed decorators, decorated class, config absent vs. present, and the alias
  limitation documented as a non-matching case.

## Non-goals for phase 1

- No reference/caller analysis, no `ty`/`tyf` integration.
- No pytest invocation, no fixture parsing.
- No config beyond `ignore-decorators` (everything else: sensible defaults + CLI flags).
- No handling of notebooks, stubs (`.pyi`), or generated code.
