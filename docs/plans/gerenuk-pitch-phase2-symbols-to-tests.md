# gerenuk — impact-based pytest selection (Phase 2: changed symbols → impacted tests)

> Phase 1 landed as `gerenuk changed-symbols`, and the repo grew an `audit` command
> along the way — which means the `tyf` subprocess adapter, the JSON wire types, the
> banner-stripping workaround, the workspace/test-path heuristics and the stubbed-tyf
> test harness all already exist. Phase 2 is mostly a new pure module plus a closure
> loop wired between two seams that are already there.

## What phase 2 is

Walk the reverse reference graph from the symbols `changed-symbols` reports until
reaching test code, and report which test files/functions are impacted. Still no pytest
invocation (phase 3); the deliverable is a standalone subcommand that is useful on its
own for "what does this change touch?" questions.

```
gerenuk impacted-tests [--base <ref>] [--repo <path>] [--json|--pretty]
                       [--max-depth <n>] [--changed <report.json>]
```

- By default, computes the changed-symbols report internally (library call, same code
  path as the subcommand — never a subprocess of itself).
- `--changed <report.json>` accepts a saved phase-1 JSON report instead, for debugging
  and for replaying a diff without the working tree. This also pins the contract: the
  phase-1 schema is the interface between the phases.

## The closure primitive: `tyf refs`, not `tyf calls --in`

Two candidate primitives exist in `tyf`; choose **`refs`** and drive the loop ourselves:

- `refs` returns *all* references (calls, decorator usage, assignments to callbacks,
  subclassing), not just call edges — better recall for exactly the dynamic-ish patterns
  a call hierarchy misses.
- `tyf calls --in` caps transitive depth at 5 and requires ty ≥ 0.0.41; `refs` works on
  the whole tested ty range. Owning the loop means owning cycle handling, budgets,
  dedup, and the why-chain (below).
- `refs` accepts positions (`file.py:LINE:COL`), multiple symbols per invocation, and
  `--stdin`. Batch one BFS frontier per `tyf` call; with the daemon warm each call is
  ~50–100 ms, so a depth-4 closure is a handful of subprocess round-trips.

**Prefer position-based lookups.** `changed-symbols` knows each symbol's definition
file and line; querying by position sidesteps name collisions (`run`, `process`,
`__init__`-adjacent names exist everywhere). Name-based `refs` is the fallback when no
position is available.

## The algorithm

BFS over symbols, working tree only:

1. **Seed the frontier** with:
   - every entry in `changed_symbols` (all of `added | modified | deleted`);
   - for each module in `module_level_changes`: all top-level symbols of that module
     (`tyf list <file>` provides the outline) — a module-level change conservatively
     means "anything in this module may behave differently";
   - `ignored_symbols` are never seeded — that was the point of phase 1's filter.
2. **Expand a frontier**: one batched `tyf refs` call for the whole level.
3. **Classify each returned reference location**:
   - In a test file (existing `workspace.rs` heuristic) → record as an **impacted test**:
     file + enclosing test symbol qualname (reuse `pysource.rs` line→symbol on the
     working tree; this is exactly the phase-3-ready shape for node IDs). Do not expand
     further from test code.
   - A **plain import line** (`import x`, `from x import y` at module level) → drop it.
     Rationale: the actual usage sites inside that module appear as their own
     references, so the import line is redundant. Dropping import lines is the single
     biggest precision win of the whole design. This includes imports in
     `__init__.py` — re-exports are therefore invisible (see Deferred).
   - Any other reference in non-test code → map to enclosing symbol
     (`pysource.rs`), dedupe against the visited set, add to the next frontier. A
     reference at true module level (a module-level *call* of a changed symbol in
     module M) is handled cheaply: word-boundary scan of **test files only** for
     `import M` / `from M import`, select those tests with a `via` noting the
     module-level edge. No symbol expansion of M (see Deferred).
   - Reference inside a symbol that carries an ignored decorator → record under
     `ignored_symbols` in the output (with the decorator), do not expand. Registry
     functions must stay silent in phase 2 too.
4. **Stop** when the frontier is empty, or a budget trips (below).

### Deleted symbols

ty cannot resolve references to a name that no longer has a definition in the working
tree — this is the one place the LSP cannot help. Rule for `change: deleted`:

- Word-boundary textual search for the bare name across workspace `.py` files
  (ripgrep-style; `rg` is already a soft dependency of `tyf`, but implement in-process
  with a plain scan — no new binary dependency).
- Map each hit to its enclosing symbol with `pysource.rs`; those symbols enter the
  frontier as if they referenced the deleted symbol.
- This over-matches (comments, docstrings, same-named locals). Acceptable: deletions
  are rare per-commit, over-selection is safe, and phase 1 already decided renames are
  a `deleted` + `added` pair — the `added` half walks precisely, the `deleted` half
  walks coarsely. Document this asymmetry.

### Budgets and the run-all verdict

A pre-commit hook must never hang and never silently under-select because the walk gave
up. The command therefore always emits a **verdict**:

- `selected` — closure completed, `impacted_tests` is trustworthy (modulo the
  documented static-analysis caveats).
- `run_all` — the tool is telling the caller to run the full suite, with a `reason`:
  - `non_python_changes` non-empty (already reported by phase 1);
  - phase-1 `errors` non-empty (a file that didn't parse is a file we can't reason about);
  - frontier exceeded `--max-symbols` (default 500) or depth exceeded `--max-depth`
    (default 10);
  - wall-clock budget exceeded (default 30 s, `--budget-ms`);
  - `tyf` unavailable or failed mid-walk (`doctor`-style detection up front; any
    mid-closure `tyf` error degrades to `run_all`, never a crash — exit code stays 0,
    because "run everything" is a successful answer for a hook).

Exit codes follow the established convention: `0` verdict produced (either kind), `2`
operational failure (not a git repo, bad `--changed` file). There is no `1`: unlike
`audit`, impacted tests are not "findings".

## Why-chains

For every impacted test, keep the shortest path that reached it:

```
tests/test_enrich.py::test_run_enriches
  ← mypkg.api:enrich_endpoint ← mypkg.pipelines.enrich:Enricher.run (modified)
```

BFS gives shortest paths for free (record each symbol's predecessor on first visit).
This is the trust-building feature: when the selection looks wrong, the chain shows
exactly which edge to blame, and `tyf refs <symbol>` confirms it by hand — same
"findings are leads" posture as `audit`.

## Output schema

```json
{
  "verdict": "selected",
  "reason": null,
  "base": "origin/main",
  "merge_base": "<sha>",
  "impacted_tests": [
    {
      "file": "tests/test_enrich.py",
      "symbol": "tests.test_enrich:test_run_enriches",
      "via": [
        "mypkg.api:enrich_endpoint",
        "mypkg.pipelines.enrich:Enricher.run"
      ],
      "origin": "mypkg.pipelines.enrich:Enricher.run"
    }
  ],
  "test_files_changed": ["tests/test_enrich.py"],
  "ignored_symbols": [],
  "stats": {
    "seeds": 3,
    "visited": 41,
    "max_depth_reached": 3,
    "tyf_calls": 5,
    "duration_ms": 640
  },
  "errors": []
}
```

- `test_files_changed` passes through from phase 1 — a changed test selects itself,
  no walking needed.
- `via` is the why-chain: the intermediate symbols between the test and `origin`,
  nearest-to-the-test first, excluding both endpoints. Empty `via` means the test
  references the changed symbol directly.
- Sorted, deterministic, JSON to stdout, everything else to stderr — same contract as
  phase 1.

## Implementation notes

- **`tyf refs` wire format (verified against ty-find source):** each reference is
  `{file, line, column, context}` where `context` is the tightest enclosing dotted
  symbol path or the literal `"module scope"`. There is no reference-kind field (LSP
  `textDocument/references` has none), so import-vs-usage classification is gerenuk's
  job via tree-sitter — but only `"module scope"` refs need the import-line check.
  Refs arrive pre-partitioned into `references` / `test_references` (tyf's test
  heuristic ≈ ours, plus `conftest.py`); re-verify with our own heuristic anyway.
- **Mandatory flags:** pass `--references-limit 0` on every call — the default is 20
  and silently truncates the `references` array (only `reference_count` stays true),
  which would drop closure edges. `include_declaration` defaults to true, so filter
  out the queried symbol's own definition or every symbol becomes its own caller.
- New modules, same seam discipline as the architecture doc prescribes:
  - `closure.rs` — the BFS, **pure**: takes a `Refs` trait (`fn refs(&[SymbolQuery]) ->
    Result<Vec<RefLocation>>`) plus a `SymbolIndex` trait (line→symbol per file). Unit
    tests supply HashMap-backed fakes; no `tyf`, no filesystem.
  - `impact.rs` — glue: phase-1 report + `tyf::Runner` + `pysource` → `ImpactReport`.
  - `tyf.rs` grows `refs`/`list` calls if `audit` doesn't already make them, `model.rs`
    grows the wire types — step 2 of the existing "adding a rule" recipe.
- `pysource.rs` gets one new entry point: parse a working-tree file once, answer many
  line→symbol queries against it (cache parsed trees per file for the whole walk; refs
  cluster heavily in the same files).
- The `--changed` flag means `changed.rs`'s report type needs `serde::Deserialize` too.
- Performance target: < 2 s wall for a typical few-symbol diff with a warm daemon;
  first-run daemon cold start (~1–2 s) is `tyf`'s cost, not ours — mention it in docs.

## Configuration

```toml
[tool.gerenuk]
ignore-decorators = ["transformation", "registry.transformation"]  # phase 1
max-depth = 10
max-symbols = 500
budget-ms = 30000
```

CLI flags override config. No other keys; test-path patterns stay heuristic until a
real repo proves the heuristic wrong.

## Testing

- `closure.rs` unit tests against synthetic graphs: linear chain, diamond, cycle,
  self-recursion, edge into test file, edge into ignored-decorator symbol, import-line
  dropping (including in `__init__.py`), module-level ref → test-import scan,
  budget trips (depth, size), shortest-path
  why-chains, deterministic ordering.
- Extend the canned-`tyf` stub (`tests/common/mod.rs`) to answer `refs` and `list`
  with payloads mirroring a grown `tests/fixtures/sample_pkg`: give the fixture a
  small call chain ending in its pytest suite, plus a deleted-symbol scenario and a
  registry-decorated dead end. Keep the fixture's real pytest suite and the canned
  payloads in step, as today.
- End-to-end: `TestRepo` (real git) + stubbed `tyf` → assert full JSON including
  verdicts. One `insta` snapshot.
- A `make` target that runs `impacted-tests` against the fixture with **real**
  `tyf`/`ty` (like `make review` does for audit today) — the closure loop's contract
  with real LSP output should be exercised somewhere, just not inside `cargo test`.
- Degradation tests: `tyf` binary missing → `run_all(tyf_unavailable)`, exit 0;
  `tyf` returning garbage mid-walk → `run_all`, error recorded, exit 0.

## Non-goals for phase 2

- No pytest node-ID emission, no fixture-aware expansion, no `pytest` invocation
  (phase 3 — but `impacted_tests[].symbol` + `file` is deliberately node-ID-shaped).
- No pre-commit wiring, no telemetry (phase 4).
- No rename pairing, no dynamic-dispatch or re-export heuristics (see Deferred).
- No caching of closure results across invocations (the daemon is the cache that
  matters; revisit only if real-repo numbers say otherwise).
- No attempt to make deleted-symbol search precise.

## Deferred — known gaps to revisit

Decisions made for phase 2 simplicity, kept here so they aren't rediscovered as bugs.
Phase-4 telemetry (selected vs. full-suite comparison) is the trigger for revisiting
each:

1. **Re-export blindness.** Import lines are dropped everywhere, including
   `__init__.py`, so a symbol consumed only through a package re-export
   (`from pkg import thing` where `pkg/__init__.py` re-exports it) will under-select…
   except it usually won't: ty resolves the re-exported usage sites back to the real
   definition, so most re-export consumers still appear as ordinary refs. The gap is
   narrower — `__all__`-driven star imports and module-object attribute access. If
   telemetry shows misses that trace to re-exports, add the `__init__.py` expansion
   rule (references landing in an `__init__.py` re-enter the frontier under the
   package qualname).
2. **Module-level execution.** A changed symbol called at import time of module M
   currently selects only tests that import M directly (textual scan). Transitive
   importers of M are not walked — a non-test module importing M and being tested
   elsewhere is missed. If this bites, upgrade to seeding all of M's top-level symbols
   into the frontier (`tyf list` gives the outline), guarded by `max-symbols`.
3. **Deleted-symbol imprecision.** The textual fallback over- and under-matches by
   design (see Deleted symbols). Rename pairing (phase 1 non-goal) would remove most
   deleted-symbol walks entirely and is the likelier fix than making the search
   smarter.
