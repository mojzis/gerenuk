# Architecture

## Crate shape

`gerenuk` is a binary plus a library. `src/main.rs` parses arguments and maps an
outcome to an exit code; everything else lives in the library so it can be
tested directly.

```
src/
  main.rs       exit-code mapping, tracing setup
  lib.rs        module declarations
  cli.rs        clap definitions + command bodies
  tyf.rs        subprocess adapter around the `tyf` binary
  git.rs        subprocess adapter around the `git` binary
  model.rs      wire types for tyf's JSON
  workspace.rs  project-root detection, test-path heuristic
  analyze.rs    the audit rules (pure)
  report.rs     human/JSON rendering (pure)

  # changed-symbols
  diff.rs       unified-diff parser (pure)
  pysource.rs   tree-sitter symbol extraction, line -> symbol (pure)
  modpath.rs    file path -> dotted module path
  config.rs     [tool.gerenuk] in pyproject.toml
  changed.rs    diff + sources -> ChangedSymbols report (pure)

  # impacted-tests
  closure.rs    BFS over the reverse reference graph (pure)
  impact.rs     tyf + working tree behind closure's traits, ImpactReport
```

Decisions worth their own page live in [`../adr/`](../adr/README.md); this file
describes the shape, those record why it is that shape.

## The two impure seams

`tyf` and `git` are the only two modules that spawn a process. The spawn sites
are `tyf::Runner::run` and the private `git::Git::output` — reached only through
`git::Git::run` and `git::Git::try_run`, which differ in whether a non-zero exit
is an error or an answer. Everything downstream takes parsed values. That is deliberate: it means
the rules and the renderers are unit-testable with no `tyf`, no `ty`, no `git`
and no Python environment.

`ty-find` ships a binary and no `[lib]` target, so the integration is over
stdout with `--format json` rather than a crate dependency. `tyf` prefixes some
responses with a human banner even under `--format json`, which is why
`tyf::extract_json` seeks to the first `[` or `{` before parsing.

`git` is shelled out to for the same reason it usually is: `-M` rename detection
and merge-base resolution are exactly the parts a reimplementation gets subtly
wrong, and `changed-symbols` runs them a handful of times per invocation.
`changed::Sources` is the trait that hides it — `GitSources` is the real
implementation, and the unit tests supply a `HashMap` instead.

## Three commands, three data sources

`audit` asks `tyf` about symbols that exist now. `changed-symbols` asks `git`
what moved and `tree-sitter` who owns those lines; it never constructs a
`tyf::Runner`, which is why it works in a checkout with no `ty` installed.
`impacted-tests` is the one that needs both — but only after the verdicts that
need neither (`non_python_changes`, `parse_errors`) have been settled, so a
diff of a `pyproject.toml` alone still answers with no `ty` installed.

They share `workspace.rs` (root detection, the test-path heuristic),
`pysource.rs` and `report::Format`.

## The closure's two traits

`closure.rs` is as pure as `analyze.rs` and `changed.rs`, and for the same
reason: everything it needs from outside arrives through a trait.

| Trait | Question | Real implementation |
|---|---|---|
| `closure::Refs` | who references these symbols? | `impact::TyfRefs`, over `tyf::Runner` |
| `closure::Index` | what owns this line, is it an import, which files mention this word? | `impact::FsIndex`, over the working tree |

`Refs::refs` takes the whole BFS frontier rather than one symbol, and
`impact::TyfRefs` sends it to `tyf` as a single invocation — so `stats.tyf_calls`
counts levels, not symbols ([ADR 0003](../adr/0003-batch-the-frontier.md)).
Symbols are addressed by `file:line:col` position, because `tyf refs` has no
name form for a nested definition and a name cannot separate two same-named
symbols ([ADR 0002](../adr/0002-refs-queries-by-position.md)).

`walk` never returns an error. A seam that fails mid-walk degrades to
`Verdict::RunAll` with the message recorded in `errors`, because the consumer is
a pre-commit hook and "run everything" is a usable answer where a crash is not
([ADR 0009](../adr/0009-run-all-is-a-success.md)).

Its unit tests supply a `MapRefs` and a `MapIndex` backed by `BTreeMap`s — but
`MapIndex` runs the *real* `pysource` classification over its fake files, so the
import and line-attribution rules under test are the ones the binary uses.

## Testing strategy

| Layer | Where | Needs `tyf`? | Needs `git`? |
|---|---|---|---|
| Wire-type parsing | `src/model.rs`, `src/tyf.rs` unit tests | no | no |
| Audit rules | `src/analyze.rs` unit tests | no | no |
| Rendering | `src/report.rs` unit tests | no | no |
| Argument parsing | `src/cli.rs` unit tests | no | no |
| Diff parsing | `src/diff.rs` unit tests | no | no |
| Line → symbol | `src/pysource.rs` unit tests | no | no |
| Change classification | `src/changed.rs` unit tests | no | no |
| Closure / BFS rules | `src/closure.rs` unit tests | no | no |
| tyf + working-tree glue | `src/impact.rs` unit tests | no | no |
| git adapter | `src/git.rs` unit tests | no | yes |
| End-to-end (audit) | `tests/cli.rs`, `tests/audit.rs` | no — stubbed | no |
| End-to-end (changed-symbols) | `tests/changed_symbols.rs` | no | yes |
| End-to-end (impacted-tests) | `tests/impacted_tests.rs` | no — stubbed | yes |
| Real LSP output | `make test-impact` (`scripts/impact-smoke.sh`) | **yes** | yes |

`tests/common/mod.rs` writes a small shell script that answers `list` and `refs`
with canned JSON, then points `GERENUK_TYF` at it. The canned payloads mirror
what real `tyf` returns for `tests/fixtures/sample_pkg`, so the fixture package
and the expectations stay in step.

`tests/fixtures/sample_pkg` is a real, installable Python package with its own
pytest suite (`make test-fixture`). Its symbols are arranged to cover each
reference state gerenuk distinguishes:

| Symbol | State |
|---|---|
| `describe` | referenced from `cli.py` and `pipelines.py` — not flagged |
| `ShelterService.summary` | referenced from `cli.py` — not flagged |
| `ShelterService.seniors` | referenced only from `tests/` — `note` |
| `legacy_export` | referenced nowhere — `warn` |
| `ShelterService._sorted_by_age`, `__init__` | skipped by the name filter |

It also carries a two-hop chain for `impacted-tests` to walk —
`service:describe` → `pipelines:Enricher.run` → `api:enrich_endpoint` →
`tests/test_api.py` — and one registry dead end, `pipelines:normalise_species`,
matched by the fixture's own `[tool.gerenuk] ignore-decorators`. `cargo test`
stubs `tyf`, so that chain is exercised against real LSP output by
`make test-impact`, which copies the fixture into a throwaway repository (the
fixture lives inside gerenuk's own checkout, so running in place would diff
gerenuk instead).

Changing the fixture's call graph means updating both the pytest suite and the
canned payloads in `tests/audit.rs`.

`tests/changed_symbols.rs` builds throwaway repositories with the real `git`
binary (`common::TestRepo`), pointing `GIT_CONFIG_GLOBAL` and
`GIT_CONFIG_SYSTEM` at `/dev/null` so a developer's signing keys, hooks or
`init.defaultBranch` cannot change the result. None of those tests set
`GERENUK_TYF`, and one runs with an empty `PATH` to prove `tyf` discovery is
never attempted.

## Adding a closure rule

1. Add the case to `closure::Walk`, with a unit test against `MapIndex`.
2. If it needs something new from the outside, add it to `Refs` or `Index` —
   not a third seam.
3. Extend `tests/impacted_tests.rs`'s `graph_repo` with a file in the new shape,
   and its canned `tyf` payloads to match.
4. Document the rule in `docs/src/commands/impacted-tests.md`, and record the
   decision in `docs/adr/` if it was a close call.

## Adding a rule

1. Add the case to `analyze::classify`, with a unit test in `analyze.rs`.
2. If it needs data `tyf` does not yet provide, add the call to `tyf::Runner`
   and a wire type to `model.rs`.
3. Extend `tests/fixtures/sample_pkg` with a symbol in the new state, and the
   canned payloads in `tests/audit.rs` to match.
4. Document the rule in `docs/src/commands/audit.md`.
