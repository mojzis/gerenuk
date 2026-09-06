# Setup gerenuk in this repository

gerenuk is not wired into this repository yet. Run everything below at the
repository root, in this order.

**1. Install.** References resolve through `tyf` (from ty-find), which drives
ty's language server; the first real run starts ty-find's background daemon,
and later runs reuse it.

```bash
uv add --dev gerenuk ty-find pytest
```

**2. Check the wiring.** `gerenuk doctor` prints the workspace and the `tyf`
it found, and exits `2` if either is missing. Nothing is analysed.

**3. Wire it into the commit hook.** As a madoqua step, in `pyproject.toml`:

```toml
[tool.madoqua]
extend_check = [
  { name = "gerenuk", cmd = "gerenuk run -- -q", pass_files = false, timeout_s = 120 },
]
```

`pass_files = false` because gerenuk takes no file list: it diffs the working
tree against `origin/main` (then `main`, then `master`), not the index, so
unstaged edits and untracked files count in a hook. `--base <REF>` overrides
the ref. Everything after `--` goes to pytest verbatim.

**4. Name the pytest**, only if plain `pytest` is not the one on PATH:

```toml
[tool.gerenuk]
pytest-command = ["uv", "run", "pytest"]
```

**5. Verify with a real diff.** With no diff at all, `gerenuk run` selects
nothing, spawns nothing and exits `0`, which proves only that it ran. Push, so
that `origin/main` matches; change one symbol a test reaches; then:

```bash
gerenuk impacted-tests
```

Expect `verdict selected` and that test with its arrow chain back to the
symbol. `gerenuk run --dry-run` then shows the pytest argv that would follow.
A `run_all` on this first try is what `gerenuk guide triage` is for.

**6. Exit codes.** `run` returns pytest's own code once pytest starts, `0` for
an empty selection, `2` when gerenuk could not run. `impacted-tests` and
`changed-symbols` never return `1`: they are inventories, not verdicts.
`audit` is the one command that returns `1` for findings.

next: run `gerenuk impacted-tests`
