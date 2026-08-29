# 0006 — The workspace file list comes from `git ls-files`

**Status:** accepted

## Decision

The textual scans (deleted-symbol name search, module-importer search) enumerate
`.py` files from `git ls-files` plus `git ls-files --others --exclude-standard`,
not from a directory walk.

## Why

`impacted-tests` already requires a repository, and the alternative is either a
new dependency (`ignore`, `walkdir`) or a hand-written walk with a hand-written
list of directories to skip — `.venv`, `node_modules`, `__pycache__`, `build`,
vendored trees. `.gitignore` already encodes exactly that list, correctly, per
project. Reusing it costs one `git` call through a seam that already exists.

## Cost

A `.py` file that is git-ignored is invisible to the scans. That is the intended
reading of "ignored", and the alternative was silently scanning `.venv`.

## Revisit when

Someone needs `impacted-tests` outside a repository. The command cannot work
there anyway — there is no diff.
