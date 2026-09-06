# Triage a gerenuk report

`gerenuk run` said one line on stderr before handing over to pytest, or
`gerenuk impacted-tests` printed a report. Read the verdict before the list.

**Three outcomes.**

- `selected`: the walk completed and the listed tests are the whole answer.
  Each entry carries an arrow chain from the test back to the changed symbol;
  read it right to left. A surprise in the list is a real edge, not a guess.
- `run_all`: gerenuk could not bound the impact, so the full suite runs (or
  the `fallback-command` does). The reason line says why; work the ladder
  below. `impacted-tests` still exits `0`: "run everything" is an answer.
- nothing: `selected` with an empty list. `run` spawns nothing and exits `0`,
  indistinguishable from a green suite, which for a hook it is.

**The `run_all` ladder.** Take the first reason that matches.

1. `non-Python files changed`: the diff touched a file gerenuk cannot reason
   about - config, a template, data - and any test may depend on it, so no
   selection is trustworthy. Commit those files separately, or accept the
   full run; `fallback-command` is the knob for repositories where the full
   suite is too slow.
2. `a changed file did not parse`: fix the syntax error, then rerun.
3. `tyf is not available`: `uv add --dev ty-find`, then `gerenuk doctor`.
4. `tyf failed during the walk` or `the working tree could not be read`:
   `gerenuk -v impacted-tests` prints the underlying error. Usually the
   daemon (`tyf daemon status`, `tyf daemon stop`) or a permissions problem.
5. `the depth limit was reached`, `the symbol limit was reached` or `the time
   budget ran out`: a hub symbol. Raise the budget for this one run,
   `gerenuk impacted-tests --max-depth 20`, and read what it finds; make it
   policy only via `gerenuk guide tune`.
6. `a changed symbol is dispatched by an unresolvable decorator`: a registrar
   gerenuk cannot see. Add the decorator to `ignore-decorators` only if the
   tests reach the symbol some other way; otherwise the full run is right.

**JSON.** `--format json` prints one object: `verdict`, `reason` (`null` when
selected), `base`, `merge_base`, `impacted_tests` as `{file, symbol, via,
origin}` where `symbol` is `null` for a whole file and `via` is the chain,
`test_files_changed`, `ignored_symbols`, `stats` and `errors`.

**Replay.** Save one walk and map it more than once:

```bash
gerenuk impacted-tests --format json > impact.json
gerenuk run --impact impact.json -- -q
```

The saved report is parsed strictly, and a stale one selects the wrong tests,
so regenerate it after every diff. `gerenuk run --dry-run` prints the decision
and the exact argv without spawning anything, in either form.

**Do not:**

- Do not bypass the hook or drop the step to get past `run_all`. The full
  suite is the safe answer; a skipped one is no answer.
- Do not raise a budget in `pyproject.toml` to make one commit go through.

next: run `gerenuk run -- -q`
