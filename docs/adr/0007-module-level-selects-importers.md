# 0007 — A module-level change also selects its test importers

**Status:** accepted

## Decision

For each entry in `module_level_changes`, the closure seeds every top-level
symbol of that module (as the pitch says) **and** selects every test file that
textually imports it.

## Why

The pitch's rule covers definitions only. A changed module *constant* is not a
definition the outline reports, so a test that imports and asserts on it would
be missed — while the whole point of `module_level_changes` is "anything in this
module may behave differently".

The same textual test-import scan is already needed for module-level references
found mid-walk, so this costs no new machinery.

## Cost

Over-selection: editing an import line in a module selects every test that
imports it, whether or not the test touches the changed line. Over-selection is
the safe direction for a pre-commit hook.

## Revisit when

Phase-4 telemetry shows module-level changes dominating the selected set.
