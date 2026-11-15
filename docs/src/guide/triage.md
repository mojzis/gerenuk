# Triage a gerenuk report

`gerenuk run` said one line on stderr before handing over to pytest, or
`gerenuk impacted-tests` printed a report. Read the verdict first.

**Three outcomes.**

- `selected`: the walk completed and the listed tests are the whole answer.
  Each entry carries an arrow chain from the test back to the changed symbol;
  read it right to left. A surprise in the list is a real edge, not a guess.
  An `impacted conftests` entry is not a test: `run` expands it to every test
  file in its subtree.
- `run_all`: gerenuk could not bound the impact, so the full suite runs (or
  the `fallback-command` does). The reason says why; work the ladder below.
  `impacted-tests` still exits `0`: "run everything" is an answer.
- nothing: `selected` with an empty list. `run` spawns nothing and exits `0`,
  indistinguishable from a green suite, which for a hook it is.

**The `run_all` ladder.** Take the first reason that matches.

1. `HEAD has no commit yet`: the first commit has nothing to diff against, so
   the full run is the answer. Nothing to fix; the next commit selects normally.
2. `non-Python files changed`: the diff touched a file gerenuk cannot reason
   about - config, a template, data. Files no test reads, like docs, go in
   `ignore-paths`; otherwise accept the full run, or set `fallback-command`.
3. `a changed file did not parse`: fix the syntax error, then rerun.
4. `tyf is not available`: `uv add --dev ty-find`, then `gerenuk doctor`.
5. `tyf failed during the walk` or `the working tree could not be read`:
   `gerenuk -v impacted-tests` prints it; often the daemon (`tyf daemon
   status`, `tyf daemon stop`) or permissions.
6. `the depth limit was reached`, `the symbol limit was reached` or `the time
   budget ran out`: a hub symbol. Raise the budget for this one run,
   `gerenuk impacted-tests --max-depth 20`, and read what it finds; make it
   policy only via `gerenuk guide tune`.
7. `a changed symbol is dispatched by an unresolvable decorator`: a registrar
   gerenuk cannot see. Add it to `ignore-decorators` only if the tests reach
   the symbol some other way; otherwise the full run is right.
8. `the full suite is faster than a selection`: `suite-ms` in `pyproject.toml`
   says so and `run` took it at its word. Remove the key if the suite grew.

**JSON.** `--format json` prints one object: `verdict`, `reason` (`null` when
selected), `base`, `merge_base`, `impacted_tests` as `{file, symbol, via,
origin}` (`symbol` null for a whole file), `test_files_changed`,
`ignored_symbols`, `ignored_paths`, `stats` and `errors`.

**Replay.** Save one walk and map it more than once:

```bash
gerenuk impacted-tests --format json > impact.json
gerenuk run --impact impact.json -- -q
```

A stale saved report selects the wrong tests: regenerate it after every diff.
`gerenuk run --dry-run` prints the decision and the exact argv without
spawning anything, and says when node ids were folded into a whole file.

**Do not:** bypass the hook or drop the step to get past `run_all` - the full
suite is the safe answer - or raise a budget in `pyproject.toml` for one commit.

next: run `gerenuk run -- -q`
