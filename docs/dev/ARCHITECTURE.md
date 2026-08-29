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
```

## The two impure seams

`tyf::Runner::run` and `git::Git::run` are the only functions that spawn a
process. Everything downstream takes parsed values. That is deliberate: it means
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

## Two commands, two data sources

`audit` asks `tyf` about symbols that exist now. `changed-symbols` asks `git`
what moved and `tree-sitter` who owns those lines. They share `workspace.rs`
(root detection, the test-path heuristic) and `report::Format`, and nothing
else — in particular, `changed-symbols` never constructs a `tyf::Runner`, which
is why it works in a checkout with no `ty` installed.

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
| git adapter | `src/git.rs` unit tests | no | yes |
| End-to-end (audit) | `tests/cli.rs`, `tests/audit.rs` | no — stubbed | no |
| End-to-end (changed-symbols) | `tests/changed_symbols.rs` | no | yes |

`tests/common/mod.rs` writes a small shell script that answers `list` and `refs`
with canned JSON, then points `GERENUK_TYF` at it. The canned payloads mirror
what real `tyf` returns for `tests/fixtures/sample_pkg`, so the fixture package
and the expectations stay in step.

`tests/fixtures/sample_pkg` is a real, installable Python package with its own
pytest suite (`make test-fixture`). Its symbols are arranged to cover each
reference state gerenuk distinguishes:

| Symbol | State |
|---|---|
| `describe`, `ShelterService.summary` | referenced from `cli.py` — not flagged |
| `ShelterService.seniors` | referenced only from `tests/` — `note` |
| `legacy_export` | referenced nowhere — `warn` |
| `ShelterService._sorted_by_age`, `__init__` | skipped by the name filter |

Changing the fixture's call graph means updating both the pytest suite and the
canned payloads in `tests/audit.rs`.

`tests/changed_symbols.rs` builds throwaway repositories with the real `git`
binary (`common::TestRepo`), pointing `GIT_CONFIG_GLOBAL` and
`GIT_CONFIG_SYSTEM` at `/dev/null` so a developer's signing keys, hooks or
`init.defaultBranch` cannot change the result. None of those tests set
`GERENUK_TYF`, and one runs with an empty `PATH` to prove `tyf` discovery is
never attempted.

## Adding a rule

1. Add the case to `analyze::classify`, with a unit test in `analyze.rs`.
2. If it needs data `tyf` does not yet provide, add the call to `tyf::Runner`
   and a wire type to `model.rs`.
3. Extend `tests/fixtures/sample_pkg` with a symbol in the new state, and the
   canned payloads in `tests/audit.rs` to match.
4. Document the rule in `docs/src/commands/audit.md`.
