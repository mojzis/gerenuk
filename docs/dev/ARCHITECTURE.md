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
  model.rs      wire types for tyf's JSON
  workspace.rs  project-root detection, test-path heuristic
  analyze.rs    the rules (pure)
  report.rs     human/JSON rendering (pure)
```

## The one impure seam

`tyf::Runner::run` is the only function that spawns a process. Everything
downstream takes parsed values. That is deliberate: it means the rules and the
renderers are unit-testable with no `tyf`, no `ty`, and no Python environment.

`ty-find` ships a binary and no `[lib]` target, so the integration is over
stdout with `--format json` rather than a crate dependency. `tyf` prefixes some
responses with a human banner even under `--format json`, which is why
`tyf::extract_json` seeks to the first `[` or `{` before parsing.

## Testing strategy

| Layer | Where | Needs `tyf`? |
|---|---|---|
| Wire-type parsing | `src/model.rs`, `src/tyf.rs` unit tests | no |
| Rules | `src/analyze.rs` unit tests | no |
| Rendering | `src/report.rs` unit tests | no |
| Argument parsing | `src/cli.rs` unit tests | no |
| End-to-end | `tests/cli.rs`, `tests/audit.rs` | no — stubbed |

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

## Adding a rule

1. Add the case to `analyze::classify`, with a unit test in `analyze.rs`.
2. If it needs data `tyf` does not yet provide, add the call to `tyf::Runner`
   and a wire type to `model.rs`.
3. Extend `tests/fixtures/sample_pkg` with a symbol in the new state, and the
   canned payloads in `tests/audit.rs` to match.
4. Document the rule in `docs/src/commands/audit.md`.
