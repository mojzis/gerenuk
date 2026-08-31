# gerenuk — impact-based pytest selection (Phase 3: impacted tests → pytest)

> Phase 2 landed as `gerenuk impacted-tests`: the verdict machinery
> (`selected` / `run_all`), why-chains, budgets, the `--changed` replay pattern
> and the closure's `Refs`/`Index` seams all exist. Phase 3 is mostly two new
> pure modules (fixture resolution, node-id selection) plus the crate's first
> deliberate amendment to the two-impure-seams invariant: a third seam that
> spawns `pytest` — and does nothing else.

## What phase 3 is

Turn an `ImpactReport` into a pytest invocation and run it. The deliverable is
one subcommand:

```
gerenuk run [--base <REF> | --impact <FILE>]
            [--max-depth <N>] [--max-symbols <N>] [--budget-ms <MS>]
            [--dry-run] [-- <pytest args>…]
```

- By default, computes the impact report internally (library call, same code
  path as `impacted-tests` — never a subprocess of itself), which in turn
  computes changed-symbols internally. One command, zero intermediate files.
- `--impact <report.json>` replays a saved phase-2 JSON report instead, parsed
  strictly (same posture as `--changed`, ADR 0010 — lenient parsing here fails
  open into a wrong selection). It conflicts with `--base` and the budget flags,
  which belong to the walk that already happened.
- Everything after `--` is appended to the pytest argv verbatim
  (`-x`, `-k`, `-n auto`, whatever).
- `--dry-run` prints the decision and the exact argv without spawning anything.

## The three-way outcome — why gerenuk owns the invocation

A selection has three possible answers, and an argument list can only express
two of them:

| Outcome | pytest argv | Trap |
|---|---|---|
| run these | `pytest a.py::t1 b.py` | — |
| run everything | `pytest` | — |
| run **nothing** | *(there is none)* | an empty argv **is** "run everything" |

`pytest $(gerenuk something)` therefore inverts the best case: a diff that
impacts no tests would run the full suite. That is why phase 3 is `gerenuk run`
and not `gerenuk print-node-ids` — the empty selection must short-circuit
*before* a shell ever interpolates it. (`--dry-run` exists for scripting and
debugging, but its human output states the decision, it does not emit a bare
interpolatable list.)

The mapping from verdict to action:

| Verdict | Action | Exit |
|---|---|---|
| `selected`, non-empty | `pytest <ids> <passthrough>` | pytest's |
| `selected`, empty | **no spawn.** "no tests impacted" to stderr | `0` |
| `run_all` (any reason) | `pytest <passthrough>` — full suite | pytest's |

"Empty" means no node ids *and* no surviving `test_files_changed`. The
`run_all` row is the safety argument from phase 2 carried to its conclusion:
gerenuk degrading mid-walk still produces a correct hook, just not a fast one.

## Node-id mapping

`impacted_tests[].symbol` was designed to be node-id-shaped; phase 3 is where
that cheque is cashed, and where its edge cases live.

- `symbol: null` → the file path is the node id (ADR 0008 said phase 3 would
  find the null case honest; it does — a whole-file selection is just the file).
- `tests.test_x:test_fn` → `tests/test_x.py::test_fn` — take `file` as-is,
  translate the qualname's dots to `::`. Class-based tests follow:
  `tests.test_x:TestFoo.test_bar` → `tests/test_x.py::TestFoo::test_bar`.
- **Collectibility gate.** pytest only collects `test*` functions and `Test*`
  classes without an `__init__` (default conventions). The walk records the
  *enclosing symbol* of a reference, which is frequently a helper
  (`_make_shelter`) or a fixture — handing pytest a non-collectible node id is
  a usage error (exit 4) that fails the entire run. So each qualname is checked
  segment-by-segment against the default conventions (tree-sitter tells us
  whether a `Test*` class defines `__init__`):
  - all segments collectible → node id as above;
  - trailing segments non-collectible (a def nested inside a test) → trim to
    the longest collectible prefix (`file::test_x`);
  - first segment non-collectible → it is a helper or a fixture. A fixture
    (recognised below) gets fixture expansion; anything else degrades to the
    **whole file**. Over-selection, safe, and the why-chain shows why.
- After mapping and expansion, whole-file entries supersede their own node ids
  (same collapse rule the closure already applies, re-run because expansion can
  introduce new whole-file entries).
- pytest's ini-level overrides of the conventions (`python_functions = check_*`)
  are not read in phase 3 — documented gap, see Deferred.

Parametrised tests need nothing: `file::test_fn` selects every parametrisation,
which is the correct grain for impact selection.

## Fixture awareness

pytest injects fixtures by *name*, not by reference, so a fixture is invisible
to `tyf` from the consuming side — the walk dead-ends at the fixture's
definition. Two concrete failure modes in today's output force this feature:

1. A changed symbol referenced from a fixture body: phase 2 records
   `tests.conftest:shelter` as an impacted test. Not collectible; and its file,
   `conftest.py`, collects zero tests — the whole-file fallback selects nothing.
2. A directly edited `conftest.py`: it lands in `test_files_changed` (it is
   under `tests/`), and "the file selects itself" again selects nothing.

Both under-select silently, which is the one thing the design promised never to
do. The rule set, all tree-sitter, all pure, no `tyf`:

**Recognising fixtures.** A def whose decorators suffix-match `pytest.fixture`
or `fixture` (same syntactic matcher as `ignore-decorators`: bare, called, or
dotted; aliases not resolved). The fixture's name is the def's name, unless a
`name="…"` **string-literal** kwarg overrides it. `autouse=True` (literal) is
recorded. A non-literal `name=` makes the fixture unresolvable → coarse
fallback below.

**Fixture scope** (who could consume it):

| Defined in | Scope |
|---|---|
| a test module | that module |
| a `conftest.py` | every test file in that directory's subtree |

**Consumers within the scope:**

- test functions (and methods) whose parameter list contains the fixture name;
- functions carrying `@pytest.mark.usefixtures("name")` with string-literal
  arguments, and members of `Test*` classes so decorated;
- other fixtures whose parameters contain the name — followed transitively
  through the fixture map (cycles terminate on the visited set, as ever);
- `autouse=True` → every test in scope, no name matching.

**Coarse fallbacks**, when precision is not available:

- unresolvable fixture (dynamic `name=`) → every test file in scope;
- a `conftest.py` with **module-level** changes (imports, constants — the
  module executes at collection time for the whole subtree) → every test file
  in its subtree;
- a `conftest.py` in `test_files_changed` whose changed symbols cannot be
  read → every test file in its subtree.

Expanded entries keep the audit trail: the fixture's symbol id is prepended to
the why-chain, so the human output reads

```
tests/test_service.py::test_summary
  ← tests.conftest:shelter ← sample_pkg.service:describe
```

— same "the chain names the edge to blame" posture, and `tests.conftest:shelter`
is a real symbol id a human can feed to `tyf refs`.

Fixtures defined outside test paths (pytest plugins, `pytest_plugins`
indirection) are invisible — documented gap, see Deferred.

## `test_files_changed`, finally filtered

The phase-1 plan noted a deleted test file stays in `test_files_changed` and
"phase 3 filters". It does:

- entries not present in the working tree → dropped;
- `conftest.py` entries → fixture/subtree expansion (above), never a node id;
- everything else → the file is its own node id, as always intended.

## The third seam

`tyf::Runner::run` and `git::Git::output` are no longer the only spawn sites:
`pytest.rs` becomes the third, and the invariant is amended rather than broken
(ADR 0001 gets a successor). What keeps the seam honest is its shape:

- **exec-and-replace, not run-and-parse.** On Unix the final act is
  `CommandExt::exec` — gerenuk's process *becomes* pytest. No output capture,
  no TTY mediation, no signal forwarding, no progress reinterpretation;
  Ctrl-C, colours and the exit code are pytest's own because the process is
  pytest. (Windows: spawn, wait, propagate the code — behaviourally identical,
  one honest `#[cfg]`.)
- gerenuk says its piece to **stderr before** the exec, one line:

  ```
  gerenuk: 4 node id(s) from 2 origin(s) in 640 ms — details: gerenuk impacted-tests
  gerenuk: full suite — non-Python files changed
  gerenuk: no tests impacted — nothing to run
  ```

- argv assembly is a pure function (`Selection` + passthrough + resolved
  runner → `Vec<OsString>`), unit-tested with no spawn anywhere near it.

**Finding pytest.** First of: `GERENUK_PYTEST` (the test-stub hook, same
pattern as `GERENUK_TYF` / `GERENUK_GIT`), config, `pytest` on `PATH`. The
config key is an argv, not a string, because the common real-world value is a
multi-word runner:

```toml
[tool.gerenuk]
pytest-command = ["uv", "run", "pytest"]
```

**Exit codes.** After the exec, the code is pytest's, verbatim — that *is* the
hook contract (`0` green, `1` failures, and pytest's `2`–`5` mean what pytest
says they mean). Before any spawn, gerenuk's own operational failures exit `2`
as everywhere else; there is no ambiguity because pytest never ran. The
`selected`-and-empty outcome exits `0` — deliberately indistinguishable from a
green suite, because for a hook that is exactly what it is.

## `--dry-run`

- Human: the decision, the summary line, and the argv one element per line —
  legible, not interpolatable.
- `--format json`: a `Selection` report — the impact report's verdict plus
  `node_ids`, `expanded` (fixture expansions with their chains), `dropped`
  (deleted test files, superseded node ids), and the assembled `argv`. This is
  the phase-3 schema pin, the way `--changed` pinned phase 1's and `--impact`
  pins phase 2's.

## Implementation notes

New modules, same seam discipline:

```
src/fixtures.rs  NEW  pure: fixture map + consumer resolution (tree-sitter)
src/select.rs    NEW  pure: ImpactReport + fixture map + working-tree facts
                      -> Selection (three-way outcome, node ids, chains)
src/pytest.rs    NEW  the third seam: runner resolution, argv assembly, exec
src/cli.rs        +   the Run command
src/config.rs     +   pytest-command
```

- `select.rs` reaches the world through a small trait (`closure::Index`
  re-used, or a subset — file existence, parse, subtree listing); unit tests
  supply the map-backed fake that already runs real `pysource` classification.
- `fixtures.rs` leans on `pysource.rs` for decorator extraction — the
  suffix-matcher exists since phase 1 (`ignore-decorators`); it grows literal
  kwarg reading (`name=`, `autouse=`) and parameter-name listing.
- `impact.rs`'s `ImpactReport` needs `Deserialize` (strict) for `--impact`,
  exactly as `changed.rs` did for `--changed`.
- Performance is a non-issue: fixture mapping parses only test files, which the
  parse cache from phase 2 already holds warm when the walk ran in-process.

## Configuration

```toml
[tool.gerenuk]
ignore-decorators = ["transformation"]        # phase 1
max-depth = 10                                # phase 2
max-symbols = 500
budget-ms = 30000
pytest-command = ["uv", "run", "pytest"]      # phase 3
```

CLI beats config beats default, as established. No key for the fixture rules —
they are behaviour, not policy, until a real repo argues otherwise.

## Testing

- `fixtures.rs` unit tests: decorator forms (bare / called / dotted /
  `pytest.fixture` vs `fixture`), `name=` literal and non-literal, `autouse`,
  parameter matching, `usefixtures` on functions and classes,
  fixture-requests-fixture chains, a fixture cycle, conftest subtree scoping,
  a fixture shadowing a conftest fixture by name (module wins — pytest's rule).
- `select.rs` unit tests: null symbol → file; helper → whole file; nested def →
  trimmed prefix; `Test*` with `__init__` → whole file; conftest entries →
  subtree; deleted test file dropped; collapse after expansion; the three-way
  outcome incl. `selected`-and-empty; chain prepending; determinism.
- `pytest.rs`: argv assembly is pure unit tests. The spawn is covered
  end-to-end with a stub — `GERENUK_PYTEST` pointing at a script that records
  its argv and exits with a chosen code — the same trick `GERENUK_TYF` plays.
  Asserted: recorded argv for all three outcomes, exit-code propagation, and
  that the empty outcome spawns **nothing** (the recording file must not
  exist).
- Fixture package grows a `tests/conftest.py` with a consumed fixture, a
  chained fixture and an autouse one, plus the pytest tests using them; the
  canned `tyf` payloads follow, as the invariant demands.
- One `insta` snapshot of `--dry-run --format json`.
- `make test-run`: the fixture, a real `tyf` **and real pytest** — the full
  pipeline exercised once outside `cargo test`, extending `test-impact`.

## Non-goals for phase 3

- No pre-commit wiring, no staged-vs-worktree handling, no telemetry (phase 4).
- No parametrisation-level node ids (`[param]`) — function grain is the grain.
- No reading of pytest ini/toml collection conventions or `addopts`.
- No fixture discovery outside test paths (plugins, `pytest_plugins`), no
  `indirect=` parametrisation, no dynamically generated fixtures.
- No pytest output interpretation of any kind — the seam stays exec-thin.
- No `--failed-first` / ordering / parallelism opinions; `-- -n auto` exists.

## Deferred — known gaps to revisit

Phase-4 telemetry (selected vs. full-suite comparison) is the trigger, as
before:

1. **Collection-convention drift.** A repo overriding `python_functions` /
   `python_classes` makes the collectibility gate wrong in both directions.
   Fix when seen: read the conventions from pytest config; until then the gate
   mirrors pytest's defaults and the mismatch degrades to whole-file.
2. **Plugin-provided fixtures.** A changed fixture living in an installed
   plugin or a non-test module is invisible; its consumers dead-end unexpanded.
   The walk still reaches the *definition* module if it is in the repo, so the
   miss is narrower than it sounds — but it exists.
3. **`usefixtures` beyond literals** (variables, `pytestmark` lists) fall to
   the coarse scope rule. Harmless until a repo leans on them heavily.
4. **Exit-code aliasing.** After exec, gerenuk cannot distinguish its own
   never-happened failures from pytest's `2`; acceptable because pre-exec
   failures already exited before pytest existed, but worth restating in the
   hook docs when phase 4 writes them.
