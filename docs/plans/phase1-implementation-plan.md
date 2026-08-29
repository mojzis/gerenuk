# Phase 1 implementation plan — `gerenuk changed-symbols`

Plan for [gerenuk-pitch-phase1-diff-to-symbols.md](gerenuk-pitch-phase1-diff-to-symbols.md),
fitted to the crate as it stands today (`audit` / `doctor` on top of `tyf`).

## 0. Decisions that deviate from the pitch

These are deliberate; each is cheap to reverse.

| Pitch says | Plan does | Why |
|---|---|---|
| `--json \| --pretty` | reuse the existing global `--format {human,json}` | one output flag for the whole binary; `changed-symbols` gets a human renderer too |
| `--repo <path>` | reuse the existing global `--workspace <path>` | same concept, already implemented and tested |
| "symbol in both old and new *mapping* → `modified`" | classify by presence in the old/new **parsed symbol tables**, not the touched sets | a pure-deletion hunk inside a surviving function produces old-side lines only; the pitch wording would call that function `deleted` |
| (silent) | untracked, unstaged files are included, all lines counted as added | "working tree" means working tree; a brand-new module is the most common thing a pre-commit hook sees |
| `pyproject.toml` detects the package root | `__init__.py` chain is primary; strip a leading `src/` when no chain exists | strictly more reliable than parsing build-backend package tables; the gap is PEP-420 namespace packages, documented as a limitation |

Also settled:

- `.pyi` counts as `non_python_changes` (stubs are a phase-1 non-goal).
- A **deleted** test file is still listed in `test_files_changed`. Harmless; phase 3 filters.
- Exit code is always `0` on a successful run, `2` on operational failure. `changed-symbols`
  never returns `Outcome::FindingsReported` — the existing `0/1/2` contract is untouched.

## 1. Architectural impact

The crate has one invariant that this phase breaks: *`tyf::Runner::run` is the only function
that spawns a process*. Phase 1 needs `git`.

**Resolution:** a second, equally narrow seam. `git::Git::run` becomes the only other
spawner, and everything downstream of it is pure. The invariant becomes "there are exactly
two impure seams, `tyf::Runner::run` and `git::Git::run`". Update `CLAUDE.md` and
`docs/dev/ARCHITECTURE.md` in the same change that introduces the module.

Second impact: `Cli::run` currently calls `Runner::discover(&root)?` **before** matching the
subcommand. `changed-symbols` must work with no `tyf` and no `ty` installed. Move the
discovery into the `Audit` and `Doctor` arms.

## 2. New modules

```
src/git.rs      impure seam: merge-base resolution, name-status, raw diff, old blobs
src/diff.rs     pure unified-diff parser (hunk headers → per-file line ranges)
src/pysource.rs pure tree-sitter symbol extraction + line→symbol mapping
src/modpath.rs  pure file path → dotted module path
src/config.rs   [tool.gerenuk] loader
src/changed.rs  orchestration over parsed data + the ChangedSymbols report type
```

`changed.rs` takes already-parsed git output, so the whole classification is unit-testable
with no git repo — the same property `analyze.rs` has today with respect to `tyf`.

New dependencies: `tree-sitter`, `tree-sitter-python`, `toml`. Dev: `insta`.
All MIT/Apache-2.0 — no `deny.toml` change needed.

## 3. Work order (TDD, each step red → green)

### Step 1 — `config.rs`
`Config { ignore_decorators: Vec<String> }`, loaded from `<root>/pyproject.toml`
`[tool.gerenuk] ignore-decorators`. Unknown keys are accepted (forward compatibility).

Tests: no file; file without `[tool.gerenuk]`; table present; malformed TOML errors with the
path in the message.

### Step 2 — `modpath.rs`
`module_path(repo_root, rel_path) -> Option<String>`.

Walk up from the file's directory while each level holds `__init__.py`; the module is that
chain plus the file stem. `__init__.py` yields the package itself. No chain → strip a leading
`src/` component and use the remaining path, dotted.

Tests: `src/mypkg/a/b.py` → `mypkg.a.b`; flat `mypkg/a.py` → `mypkg.a`;
`src/mypkg/__init__.py` → `mypkg`; `scripts/loose.py` with no `__init__.py` → `scripts.loose`.

### Step 3 — `diff.rs`
Pure parser over the text of `git diff -U0`. Produces
`Vec<FileDiff { old_path: Option<PathBuf>, new_path: Option<PathBuf>, old_lines: Vec<LineRange>, new_lines: Vec<LineRange> }>`.

Must handle: `@@ -a,b +c,d @@` with omitted counts, zero counts (`@@ -5,0 +6,2 @@`),
`/dev/null` sides, `diff --git` rename headers, binary-file stanzas, and multiple files in one
stream. Git is invoked with `-c core.quotePath=false` so paths are not C-escaped.

Tests: one per case above, from inline diff strings.

### Step 4 — `git.rs`
Thin `Git { repo: PathBuf }` with:

- `resolve_base(explicit) -> Result<(String /*ref*/, String /*sha*/)>` — try the explicit ref,
  else `origin/main`, `main`, `master` via `git rev-parse --verify --quiet`; then
  `git merge-base HEAD <ref>`. Error names every candidate tried.
- `name_status(merge_base)` → `git diff --name-status -M` (working tree: no `--cached`, no
  second commit, so staged **and** unstaged).
- `untracked()` → `git ls-files --others --exclude-standard`.
- `raw_diff(merge_base)` → one `git diff -U0 --no-color -M <mb>` for the **whole** diff, fed to
  `diff.rs`. One subprocess, not one per file — this is what keeps the 100 ms target reachable.
- `show_blob(merge_base, path) -> Result<Option<String>>` → `git show <mb>:<path>`; `None` when
  the path did not exist there.

Tests: integration-level only (needs a real repo) — covered in step 8.

### Step 5 — `pysource.rs`
Parse with `tree-sitter-python` into
`Vec<SymbolSpan { qualname, kind, start_line, end_line, decorators: Vec<String> }>`,
1-based inclusive lines.

Walk rule: recurse into **class bodies only, never function bodies**. That single choice
delivers three of the pitch's requirements at once — methods become `Class.method`, nested
classes become `Outer.Inner`, and any line inside a closure lands in its enclosing named
function's span. `async def` is the same node with an `async` child, so it needs nothing extra.

A `decorated_definition` span starts at its first decorator line, so decorator edits map to the
decorated symbol. A class-body line outside every method resolves to the class, because
`map_line` picks the **deepest** span containing the line; a line in no span is module level.

`root.has_error()` → the caller records the file in `errors` and treats it as `module_level`.

Decorator matching (`decorator_matches(dotted, entry)`): reduce the decorator expression to its
dotted name (drop call parentheses and arguments), then match when
`dotted == entry || dotted.ends_with(&format!(".{entry}"))`. Import aliases are not resolved —
documented as a known limitation.

Tests, from inline sources: function body, signature line, default-value expression, decorator
line, class attribute, dataclass field, nested def, nested class, `async def`, `@overload`
pairs, module-level import, module-level constant, docstring-only edit, syntax error.
Decorator tests: `@transformation`, `@transformation(...)`, `@registry.transformation`,
entry `registry.transformation` **not** matching bare `@transformation`, mixed
matching+non-matching decorators (ignored wins), decorated class, and the alias case asserted
as *non*-matching.

### Step 6 — `changed.rs`
Pure function taking `(config, repo_root, merge_base, name_status, file_diffs, blob_loader)`.

Partition first: test `.py` (via the existing `workspace::is_test_path`) → `test_files_changed`;
other `.py` → analysis; everything else → `non_python_changes`.

For each analysed file: parse the working-tree content into the **new** table and the
`git show <mb>:<path>` blob into the **old** table. Map new-side line ranges against the new
table, old-side ranges against the old table. Then per touched symbol:

- in both tables → `modified`; only old → `deleted`; only new → `added`.
- a symbol carrying a matching ignored decorator (checked in whichever table it exists) goes to
  `ignored_symbols` with `ignored_by`, never to `changed_symbols`.
- a touched line in no span → `module_level_changes` gets the module path (deduped).

Symbol ids are `module.path:QualName`. Every output array is sorted; the report serialises to
the pitch's schema verbatim.

Tests: classification driven entirely by synthetic inputs — no git, no filesystem.

### Step 7 — CLI wiring
Add `Command::ChangedSymbols { base: Option<String> }`. Move `Runner::discover` into the
`Audit`/`Doctor` arms. Human renderer: counts plus a grouped list; JSON renderer is the schema.

Tests: parsing (`--base`, default, `--format json`), plus `Cli::command().debug_assert()` which
already runs.

### Step 8 — integration tests (`tests/changed_symbols.rs`)
New helper in `tests/common/mod.rs`: build a temp git repo (`git init`, deterministic
`user.name`/`user.email`, `-c commit.gpgsign=false`), commit a base state, mutate the tree,
run the binary with **no** `GERENUK_TYF` set — proving the command needs no `tyf`.

Cases from the pitch: modify a body; add a function; delete a function; delete a whole file;
rename a file (asserting the `deleted` + `added` pair, no rename pairing); change a non-Python
file; syntax error in the working tree; src-layout; flat layout; untracked new file; staged +
unstaged mixed; `--base` pointing at a missing ref (exit 2, clear message); empty diff (exit 0,
empty arrays); `ignore-decorators` present vs. absent.

Plus one `insta` snapshot of the full JSON against a small fixture repo.

### Step 9 — docs
`docs/src/commands/changed-symbols.md`, an entry in `docs/src/SUMMARY.md` (the `llms.txt`
generator reads it), a README line, and the `ARCHITECTURE.md` / `CLAUDE.md` invariant update
from §1. Re-run `docs/inject-shared.sh` if the shared prose changes.

## 4. Risks

- **tree-sitter version drift.** `tree-sitter` and `tree-sitter-python` must agree on the ABI;
  pin both and let the compile error surface early rather than at runtime.
- **`git` on PATH.** The integration tests require it. CI already has it; note the prerequisite
  in the docs alongside `tyf` and `uv`.
- **`-U0` and rename detection interact.** `-M` rewrites the header paths; the parser must read
  paths from `---`/`+++` (or the rename headers), never from `diff --git` alone.
- **Performance.** One `git diff` for everything, but still one `git show` per modified file.
  If the 100 ms target slips, batch the blobs through `git cat-file --batch`. Measure before
  optimising.

## 5. Definition of done

`make review` green (fmt, clippy `-D warnings`, tests, fixture pytest, audit, deny), every
pitch mapping rule covered by a named test, `gerenuk changed-symbols --format json` producing
the pitch schema on this repo with no `tyf` installed, and the docs built.
