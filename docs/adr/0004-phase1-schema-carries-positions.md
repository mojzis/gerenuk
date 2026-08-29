# 0004 — The phase-1 JSON carries definition lines and module files

**Status:** accepted

## Decision

`changed-symbols` output gained two things:

- `changed_symbols[].line` — the line of the definition's *name*;
- `module_level_changes` became `[{module, file}]` instead of `[module]`.

## Why

The pitch pins the phase-1 schema as the interface between the phases, and
`--changed report.json` has to replay a walk with nothing else. Seeding a
module-level change means outlining that module, which needs its file; dropping
`tyf`'s `include_declaration` self-reference needs the definition line.

## Cost

A breaking change to a published JSON schema, at 0.1.0 with one release behind
it. `module_level_changes` is no longer a list of strings, so a consumer that
`jq`-ed it has to add `[].module`.

## Revisit when

Never, hopefully. Adding a field is cheap; changing an element's type is not, so
this was the moment to do it.

## Correction (2026-08-29)

The Decision above lists two additions; the change shipped three. It also added
`changed_symbols[].column`, without which
[0002](0002-refs-queries-by-position.md) has no position to query and
[0010](0010-a-replayed-report-is-parsed-strictly.md) has nothing to be strict
about — both records already refer to the field as this one's. Recorded here
rather than by editing the Decision, which is immutable.
