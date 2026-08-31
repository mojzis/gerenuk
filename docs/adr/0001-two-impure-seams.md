# 0001 — `tyf` and `git` are the only subprocesses

**Status:** accepted (phase 1, recorded retrospectively).
Amended by [0011](0011-a-third-seam-that-only-execs.md), which adds
`pytest::Runner::exec` as a third seam and says why that one is different.

## Decision

`tyf::Runner::run` and `git::Git::run` are the only functions in the crate that
spawn a process. Everything downstream takes already-parsed data.

## Why

It is what makes `analyze`, `changed`, `closure` and `report` unit-testable with
no `tyf`, no `ty`, no `git` and no Python environment. `cargo test` is hermetic
because of this rule and nothing else.

## Cost

Every new external capability has to arrive as a trait with a fake in the tests
(`changed::Sources`, `closure::Refs`, `closure::Index`) rather than as a direct
call. That is a real friction on each new feature.

## Revisit when

A third seam is genuinely unavoidable — and then say so in a new record rather
than adding one quietly. That happened once, in
[0011](0011-a-third-seam-that-only-execs.md).
