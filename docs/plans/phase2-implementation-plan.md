# Phase 2 implementation plan — `gerenuk impacted-tests`

Plan for [gerenuk-pitch-phase2-symbols-to-tests.md](gerenuk-pitch-phase2-symbols-to-tests.md),
fitted to the crate as it stands after phase 1 (`audit` / `doctor` / `changed-symbols`).

Every decision that deviates from the pitch is recorded as a one-page ADR under
[`docs/adr/`](../adr/README.md); this file is the work order.

## 0. Deviations from the pitch

| Pitch says | Plan does | ADR |
|---|---|---|
| `--repo`, `--json\|--pretty` | reuse the global `--workspace` / `--format` | — (same as phase 1) |
| query `tyf refs` by `file.py:LINE:COL` | as the pitch says — and it turned out to be forced, not merely preferable | [0002](../adr/0002-refs-queries-by-position.md) |
| one batched `tyf refs` call per BFS frontier | as the pitch says | [0003](../adr/0003-batch-the-frontier.md) |
| `via` includes `origin` (JSON example) | `via` excludes both endpoints (the prose definition) | [0005](../adr/0005-why-chain-excludes-endpoints.md) |
| `module_level_changes: [String]` | `[{module, file}]` — phase 2 needs the file to outline it | [0004](../adr/0004-phase1-schema-carries-positions.md) |
| in-process ripgrep-style scan over workspace `.py` files | file list comes from `git ls-files` + untracked | [0006](../adr/0006-file-list-from-git.md) |
| (silent) | a module-level change also selects test files importing that module | [0007](../adr/0007-module-level-selects-importers.md) |

Also settled:

- `impacted_tests[].symbol` is `null` when the whole file is selected (a
  module-level edge). A symbol id with **no colon is a module**, with a colon is
  a symbol — one convention for `via`, `origin` and `symbol` alike ([0008](../adr/0008-module-ids-have-no-colon.md)).
- Exit codes: `0` for either verdict, `2` for operational failure. Never `1`.
- `conftest.py` fixture-awareness stays a documented gap (phase 3).
- A `--changed` report is parsed strictly — lenient parsing there fails open
  ([0010](../adr/0010-a-replayed-report-is-parsed-strictly.md)).

## 1. Architectural impact

No new seam. `closure.rs` is pure and reaches the world through two traits:

- `closure::Refs` — "what references these symbols?", implemented over
  `tyf::Runner`;
- `closure::Index` — "what owns this line, is it an import, which files mention
  this word?", implemented over the working tree plus `git ls-files`.

The crate invariant is unchanged: `tyf::Runner::run` and `git::Git::run` remain
the only functions that spawn a process.

`impacted-tests` *does* construct a `tyf::Runner` — unlike `changed-symbols` —
but only after the cheap up-front verdicts have been checked, so a diff that
touches only non-Python files answers without `tyf` installed.

## 2. New and changed modules

```
src/closure.rs   NEW  pure BFS over the reverse reference graph + Refs/Index traits
src/impact.rs    NEW  glue: phase-1 report + tyf + working tree -> ImpactReport
src/pysource.rs  +    name_line on spans, module-scope import ranges, top_level()
src/changed.rs   +    line on SymbolChange, ModuleChange, Deserialize on the report
src/config.rs    +    max-depth / max-symbols / budget-ms
src/git.rs       +    ls_files()
src/cli.rs       +    the ImpactedTests command and its body
```

No new dependencies.

## 3. Work order (TDD, each step red → green)

### Step 1 — `pysource.rs` grows what the closure needs
`SymbolSpan::name_line` and `name_column` (the identifier's position, not the
decorator's),
`Module::imports` (line ranges of every `import` / `from … import`),
`Module::top_level()`, `Module::is_import_line()`.

### Step 2 — `changed.rs` schema
`SymbolChange::line` and `column`, `module_level_changes: Vec<ModuleChange>`,
and a strict `Deserialize` on the whole report so `--changed report.json` can
replay it without failing open. Update the insta snapshot and the phase-1 docs.

### Step 3 — `config.rs` budgets
`max_depth`, `max_symbols`, `budget_ms` as `Option<…>` so "unset" is
distinguishable from "set to the default", and CLI > config > default resolves
unambiguously.

### Step 4 — `git.rs::ls_files`
Tracked paths, NUL-separated. `impact.rs` unions it with `untracked()`.

### Step 5 — `closure.rs`, the pure core
Types (`SymbolQuery`, `Seed`, `RefSite`, `Site`, `IndexedSymbol`, `Limits`,
`Closure`, `Verdict`, `Reason`), the `Refs`/`Index` traits, and `walk()`.
Unit-tested against HashMap-backed fakes: linear chain, diamond, cycle,
self-recursion, edge into a test file, ignored-decorator dead end, import-line
dropping (including `__init__.py`), module-level ref → test-import scan, deleted
symbol → word scan, every budget trip, shortest-path `via`, determinism.

### Step 6 — `impact.rs`, the glue
`TyfRefs` (a `Refs` over `tyf::Runner`), `FsIndex` (an `Index` over the working
tree, parse-cached per file), seed construction from a `ChangedSymbols`, the
up-front verdicts, and `ImpactReport` + its renderers.

### Step 7 — `cli.rs`
`impacted-tests [--base REF] [--changed FILE] [--max-depth N] [--max-symbols N]
[--budget-ms MS]`. Body: load or compute the phase-1 report, check the up-front
verdicts, discover `tyf` only if the walk will actually run, walk, render.

### Step 8 — fixtures and end-to-end
Grow `tests/fixtures/sample_pkg` with a two-hop chain
(`pipelines.Enricher.run` → `api.enrich_endpoint` → `tests/test_api.py`) and a
`@registry.transformation` dead end, plus the pytest tests that cover them.
Extend the `tyf` stub to answer the new symbols. `tests/impacted_tests.rs`:
`TestRepo` + stubbed `tyf`, one insta snapshot (with `duration_ms` redacted),
and the degradation tests (`tyf` missing, `tyf` garbage mid-walk → `run_all`,
exit 0).

### Step 9 — docs
`docs/src/commands/impacted-tests.md`, `SUMMARY.md`, `ARCHITECTURE.md`,
`CLAUDE.md` invariants, README, and the `make test-impact` target that runs the
command against the fixture with a real `tyf`.
