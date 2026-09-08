# 0016 — A framework hook overridden on a subclass is a reference

**Status:** accepted

## Decision

`audit` does not report a method as unreferenced when both hold:

- the class it sits in declares a base other than `object`;
- the method's own name is one a framework is known to call on a subclass —
  `logging.Filter.filter`, `Thread.run`, `TestCase.setUp`, an HTTP verb on a
  view, or a dispatching convention such as `visit_*`, `do_*`, `on_*`,
  `clean_*`, `*Event`.

The table lives in `hooks.rs`, grouped by framework, and is consulted with
the last segment of the qualname only. Bases come from `pysource`, so the rule
is lost for a file that does not parse, the same way the decorator rule is.

Either condition alone is not enough. `filter` on a class with no base is an
ordinary name; `helper` on a `logging.Filter` subclass is an ordinary method.

## Why

The rollout across four projects found no real dead code in `audit`'s output,
and every false positive was one of two shapes: a closure ([0002](0002-refs-queries-by-position.md)
fixed the query), or an override the framework calls — `filter` on a
`logging.Filter`. `tyf refs` answers with the definition alone, which is
correct: nothing in the workspace calls it. The caller is in the standard
library, reached through the base, and that is exactly the shape
[0012](0012-a-decorator-is-a-reference.md) handles for decorators.

An allowlist of names is a heuristic. The alternative — asking `ty` whether
the method overrides one on the base's MRO — is the right edge, and `tyf` has
no command for it today.

## Cost

- A genuinely dead `run` or `save` on a subclass is not reported. The base
  requirement narrows that, and the skipped symbol still counts in
  `N symbol(s) checked`.
- `class X(object)` is treated as `class X`. A base spelled through an alias
  or a call (`class X(make_base())`) reduces to no dotted name and does not
  count as a base.
- The table is opinionated and will be incomplete for the next framework.
  Adding a name is one line and one test.

## Revisit when

`tyf` can answer "what does this method override", which turns the table into
a real edge and removes both the base requirement and the guesswork.
