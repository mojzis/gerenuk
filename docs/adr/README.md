# Architecture decision records

One file per decision, numbered, past tense, short. A record says what was
decided, what it costs, and what would make us revisit it — not how the code
works. That belongs in [`../dev/ARCHITECTURE.md`](../dev/ARCHITECTURE.md).

Records are immutable once merged. Changing your mind means a new record that
supersedes the old one, and a `Superseded by` line added to it.

| # | Decision | Status |
|---|---|---|
| [0001](0001-two-impure-seams.md) | `tyf` and `git` are the only subprocesses | accepted |
| [0002](0002-refs-queries-by-position.md) | `tyf refs` is queried by position, not by name | accepted |
| [0003](0003-batch-the-frontier.md) | One `tyf refs` call per BFS level | accepted |
| [0004](0004-phase1-schema-carries-positions.md) | The phase-1 JSON carries definition lines and module files | accepted |
| [0005](0005-why-chain-excludes-endpoints.md) | `via` excludes both endpoints | accepted |
| [0006](0006-file-list-from-git.md) | The workspace file list comes from `git ls-files` | accepted |
| [0007](0007-module-level-selects-importers.md) | A module-level change also selects its test importers | accepted |
| [0008](0008-module-ids-have-no-colon.md) | A symbol id without a colon is a module | accepted |
| [0009](0009-run-all-is-a-success.md) | `run_all` is a successful answer, not an error | accepted |
| [0010](0010-a-replayed-report-is-parsed-strictly.md) | A `--changed` report is parsed strictly | accepted |
| [0011](0011-a-third-seam-that-only-execs.md) | A third impure seam, and it only execs | accepted |
| [0012](0012-a-decorator-is-a-reference.md) | A registering decorator is a reference to what it decorates | accepted |
| [0013](0013-a-renaming-import-is-followed.md) | A renaming import is followed; a plain one is still dropped | accepted |
| [0014](0014-run-all-delegates-to-a-fallback.md) | `run_all` delegates to a configured fallback, through the exec seam | accepted |
| [0015](0015-the-guide-is-the-docs.md) | `gerenuk guide` ships the docs pages and reads one file to pick one | accepted |
| [0016](0016-a-framework-hook-on-a-subclass-is-a-reference.md) | A framework hook overridden on a subclass is a reference | accepted |
| [0017](0017-a-test-helper-is-a-step.md) | A helper in a test file is a step, not an answer | accepted |
| [0018](0018-a-registrar-is-resolved-by-position.md) | A registrar is resolved by position before it is scanned by name | accepted, amends 0012 |
| [0019](0019-a-declared-suite-time-skips-the-walk.md) | A declared suite time skips the walk | accepted |
