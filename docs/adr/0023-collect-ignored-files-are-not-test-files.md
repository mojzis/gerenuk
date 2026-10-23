# 0023 — A `collect-ignore`d file is not part of the test tree

**Status:** accepted

## Decision

`[tool.gerenuk] collect-ignore` lists glob patterns, in the syntax
`ignore-paths` uses (ADR [0021](0021-ignored-paths-never-widen.md)), for files
under a test path that pytest never collects. `select::select` applies them
and nothing else does:

- a matched file is left out of the `FixtureMap` before any file is read, so a
  `conftest.py` subtree, a fixture's consumers and the fixtures visible to a
  test never include it;
- an `impacted_tests` entry or a `test_files_changed` entry in a matched file
  produces no node id and is listed under `dropped` as `collect_ignored`;
- a matched `conftest.py` is dropped the same way, not expanded.

`changed-symbols` and `impacted-tests` do not read the key. The default is
empty, and a pattern `glob.rs` cannot read fails `Config::load`.

## Why

gerenuk decides what is a test file by path convention (`tests/`,
`test_*.py`). Repositories keep things under those paths that pytest is told
to skip: sample inputs named `test_*.py`, a vendored or legacy directory in
`collect_ignore_glob` or `--ignore`, a project's own `conftest.py` for a tree
that never runs. gerenuk handed such a file to pytest whenever a conftest
above it changed, or a fixture it names did, or the file itself did. pytest
then collects what it was explicitly given, which fails the run or runs code
that was never meant to be a test.

Reading pytest's own configuration instead was rejected for the reason the
collection conventions are not read (see `run`'s known gaps): `collect_ignore`
is Python in a `conftest.py`, `--ignore` may live in `addopts`, a wrapper
script or a CI file. The repository says it once, where gerenuk can read it.

The filter lives in the selection, not in `workspace::is_test_path`, so the
inventories stay a description of the code: `impacted-tests` still says a
reference came from the file, and `run --dry-run` says why it was not run.

## Cost

This is the one place `run` narrows a selection on purpose. A pattern that
covers a real test file means a change that reaches only that test runs
nothing, and the hook goes green. As with `ignore-paths`, it is opt-in, and
every dropped entry is listed, so a dry run shows what was set aside.

A fixture defined in a matched file is invisible too, including to a test
outside the pattern that would have used it. That is the right answer when
the file really is never collected — pytest would not see the fixture
either — and a wrong one otherwise.

## Revisit when

gerenuk reads pytest's own configuration, at which point the key becomes a
supplement to `collect_ignore` rather than a copy of it.
