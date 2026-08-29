# 0010 — A `--changed` report is parsed strictly

**Status:** accepted

## Decision

`changed::ChangedSymbols` requires `base` and `merge_base`, rejects unknown
keys, and defaults only the arrays. A `--changed` file that fails any of those
is exit `2`.

This is the opposite of `config::Config`, which accepts unknown keys on purpose.

## Why

Lenient parsing here fails *open*. With `#[serde(default)]` on the struct, `{}`
— a truncated write, a key typo, a report from a different gerenuk — parses
into an empty report, which walks to a confident `verdict: selected` selecting
no tests at all. A CI job piping that into pytest runs nothing and goes green.

The whole design says an answer gerenuk cannot trust must be `run_all`
([0009](0009-run-all-is-a-success.md)) — and a report it cannot even read is not
an answer, so exit `2` is the correct verdict rather than `run_all`.

The forward-compatibility argument that justifies leniency in `pyproject.toml`
does not apply: a `--changed` file is machine-generated minutes earlier by a
matching binary, not hand-maintained across versions.

## Cost

A report saved by an older gerenuk cannot be replayed by a newer one. That is
the intent: [0004](0004-phase1-schema-carries-positions.md) changed the schema,
and a report missing a definition's column would resolve to no references.

## Revisit when

The schema stabilises enough to be worth versioning explicitly, at which point a
`schema_version` field beats `deny_unknown_fields`.
