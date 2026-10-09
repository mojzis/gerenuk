# 0021 — Declared non-Python paths never widen the selection

**Status:** accepted

## Decision

`[tool.gerenuk] ignore-paths` lists glob patterns for non-Python files. In
phase 1, a non-Python file one of them matches goes to `ignored_paths` instead
of `non_python_changes`, so it never makes `impacted-tests` or `run` answer
`run_all`. Both reports, and the fallback payload's `report`, carry the list.
The default is empty. A `.py` file is never matched, binary or not.

Patterns are matched against the whole repository-relative path, with `*` and
`?` inside one segment and `**` as a whole segment. Any other glob syntax is
refused when the config loads, which is exit `2` for every command that reads
it.

## Why

The diff is branch-wide, from the merge base to the working tree. One README
edit early on a branch turned every later hook run on that branch into
`run_all`, and "commit it separately" did not help, because a separate commit
is still in the branch diff. Most such files — docs, changelogs, agent skill
files — are read by no test, and only the repository knows which.

The filter lives in `changed::analyze`, not in `impact::upfront_reason`, so
there is one place that decides what a non-Python change is, the
`changed-symbols` inventory shows the same split the verdict acts on, and the
fallback receives it without a second copy of the rule.

A `.py` file is exempt because the option exists to say "no test reads this",
and every Python file is something a test can import. A pattern like `**` must
not be able to switch symbol analysis off.

The matcher is small and hand-written rather than a dependency, and refuses
what it does not implement. A pattern that silently matched nothing — `[ab]`
or `{md,rst}` read literally — would be configuration that looks like it
works. Matching less is the safe direction, so a pattern with no `/` matches
only at the root rather than as a basename anywhere.

## Cost

A repository whose tests do read an ignored file — a doc linter, a test that
checks an agent instruction file — gets a confident selection that misses the
test that would catch the change. That is the one failure `run` must not have,
and here it is opt-in: the guide names the caveat next to the key.

`ImpactReport` gained a required field, so an impact report saved by an older
gerenuk no longer replays ([0010](0010-a-replayed-report-is-parsed-strictly.md)).
`ChangedSymbols` defaults its arrays, so an older `--changed` report still does.

## Revisit when

A repository needs a non-Python file to select specific tests rather than all
or none — a template to the tests that render it. That is a mapping, not an
ignore list, and would be a new key.
