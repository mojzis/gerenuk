# 0015 — The guide ships the docs, and reads one file

Status: accepted

## Decision

`gerenuk guide [setup|triage|tune]` prints agent-facing instructions for the
three moments someone meets the tool: it is not wired up here, a run said
`run_all` or a selection surprised them, or the reference is needed. This is
the family convention madoqua set (its ADR 0004) and biston and zorilla
follow; the pages are written to the same rules.

The prose is not in `guide.rs`. Each topic is a page under `docs/src/guide/`,
`include_str!`d into the binary, so the published site and the CLI serve the
same bytes and there is nowhere for a second copy to drift.

`guide` is dispatched at the top of `Cli::run`, before the workspace is looked
for, so it runs in a directory that is not a repository with no `tyf`, no
`git` and no Python. With no topic it reads exactly one file,
`./pyproject.toml`, and prints `triage` when a `[tool.madoqua]` table in it
names gerenuk as a step — parsed, not grepped — and `setup` otherwise. It
never walks up. `tune` is never auto-selected. The first output line names the
topic and why it was chosen.

`[tool.gerenuk]` alone does not count as configured: budgets tune a walk, they
do not make one run on commit, and the reader of such a repository still has
to wire the step.

Tests hold the pages to their job, and they are the point of the design:

- Every gerenuk invocation a page shows is extracted and fed through the real
  `clap::Command`. A guide that shows a command the CLI rejects is worse than
  no guide.
- Every `--flag` a page names on its own exists on some command.
- Every `[tool.gerenuk]` key a page shows is one the deserializer accepts,
  asked of serde rather than written down twice (`config::keys`), and every
  accepted key is named by a guide.
- The triage ladder names every `run_all` reason by the exact label
  `Reason::label` prints.
- No page exceeds 60 lines, every page is ASCII, and every page ends with
  exactly one `next: run` line.

Like `changed-symbols`, `guide` is an inventory rather than a verdict: `0` or
`2`, never `1`.

## Why

The audience is an agent, and an agent reads what the tool hands it, not the
website. In the aesop dogfood gerenuk worked end to end in the madoqua hook
and still cost three round trips to set up, because nothing told the reader
that the diff is the working tree rather than the index, what the madoqua
step looks like, or that `tyf` has to be on `PATH`.

Reading `./pyproject.toml` and nothing else keeps `guide` on the right side
of the seams (ADR 0001, 0011): one `std::fs::read_to_string` in a module that
exists to do it, no process spawned, no repository needed.

## What it costs

A fourth file read outside the three seams. It is a single `stat` and read of
a path the reader is told to stand next to, tolerant of every failure, and it
is the whole reason the module exists — the same argument `config.rs` makes.

Detection is heuristic. A repository that runs gerenuk from a hook other than
madoqua reads as unconfigured and gets setup instructions, which is wrong but
harmless: the instructions are idempotent, and `gerenuk guide triage` always
works by name.

The extraction the parse check relies on is not a shell. It understands a `|`
pipeline, a `>` redirect and `<placeholder>` holes, and nothing else; a page
author who writes `&&` or `;` inside a command span gets a test failure that
blames the CLI. That is loud rather than silent, which is the right way
round, and the constraint is written in the `guide.rs` module doc.

The guides compress what `docs/src/commands/` says at length. Only the
machine-checkable half — commands, flags, keys, reason labels — is guarded.

## What would make us revisit it

A fourth topic. Three is a set someone can hold in their head; four is a
manual with a table of contents, and then the right move is to point at the
site rather than grow the binary.

A second hook runner worth detecting. Then detection grows a second source,
as madoqua's has four — but not a walk up the tree.
