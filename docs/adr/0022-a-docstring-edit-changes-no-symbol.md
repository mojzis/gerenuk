# 0022 — A docstring-only edit changes no symbol, by default

**Status:** accepted

## Decision

In phase 1, a changed line that holds nothing but a docstring is not
attributed to the symbol it sits in, nor to its module. A symbol or module
that only such lines touched is listed under `docstring_changes` — symbol ids,
a module's without a colon (ADR 0008) — and seeds nothing. This is the default;
`[tool.gerenuk] docstrings = "code"` turns it off.

A docstring is the first statement of a module, class or `def` body when it is
a plain string literal: not an f-string, t-string or bytes literal, which are
code, and not an implicit concatenation, which is left as code rather than
reasoned about. A comment may precede it. Only the lines the string has to
itself count: `def f(): """doc"""` leaves its line to the `def`.

The rule applies only when the same module exists on both sides of a hunk
diff. A deleted or renamed module breaks its importers whatever it held, and
an untracked file is attributed whole.

## Why

Rewording a docstring is the commonest change that cannot fail a test, and it
cost a full selection: a function's callers' tests, or for a module docstring
every importer's. It is a speed default, chosen for the hook a developer waits
on.

The filter is per line, beside the blank-line rule, rather than a comparison
of each symbol's code with docstrings stripped. Any other line the diff
touches still counts, so a docstring edit next to a real one is a modification
as before, and a statement inserted above a docstring is code on its own line.
tree-sitter has already parsed both sides, so the cost is one more pass over
the statements: about 2–5% of `changed-symbols` on a 272-file, 190 000-line
diff, and a fraction of a millisecond per file.

## Cost

This rule *narrows* a selection, which every other degrade in gerenuk refuses
to do. Code that reads `__doc__` at runtime — docopt, a CLI framework's help
text, an API schema — gets a confident selection that misses the test that
checks that output. Making it the default trades that for speed in every other
project; the docs name the caveat next to the key.

Doctests are not a new gap: gerenuk only selects files `collects_tests`
accepts, so a doctest in a source module was never selected.

`ChangedSymbols` gained an array that defaults, so an older `--changed` report
still replays. The fallback payload gained a field, which keeps version `1`
(ADR 0014).

## Revisit when

A project needs docstrings counted for some modules only — a CLI package that
uses docopt beside a library that does not. That is a path list, not a policy.
