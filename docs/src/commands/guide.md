# `gerenuk guide`

Print the instructions for one of the three moments someone — usually an agent
— meets gerenuk.

```sh
gerenuk guide          # let gerenuk choose
gerenuk guide setup    # not wired into this repo yet
gerenuk guide triage   # a run said run_all, or a selection surprised you
gerenuk guide tune     # the reference: base, budgets, keys, binaries
```

| Topic | For |
|---|---|
| [`setup`](../guide/setup.md) | A repository gerenuk is not wired into yet. |
| [`triage`](../guide/triage.md) | Reading a report: the three outcomes and the `run_all` ladder. |
| [`tune`](../guide/tune.md) | The base ref, the budgets, `[tool.gerenuk]` and the binaries. |

## Choosing a topic

With no topic, gerenuk reads one file, `./pyproject.toml`, and prints `triage`
when a `[tool.madoqua]` table in it names gerenuk as a step — parsed rather than
grepped, so a mention in a comment or in `dependencies` does not count — and
`setup` otherwise. It never walks up, so run it at the repository root. `tune`
is never auto-selected: it is a reference, and nothing about a repository's
state says "you need the reference right now".

The first line of the output names the topic and why it was chosen:

```
# gerenuk guide: configured via pyproject.toml [tool.madoqua] -> triage
```

An explicit topic reads nothing from disk and prints `# gerenuk guide: tune`
instead.

## What it needs

Nothing. `guide` is dispatched before the workspace is looked for, so it runs
in a directory that is not a repository, with no `tyf`, no `git` and no Python
environment — `uvx gerenuk guide` in an empty directory works. Like
`changed-symbols` it is an inventory, not a verdict: it exits `0`, or `2` if it
could not write its output, and never `1`.

## Where the text comes from

The three pages under [Agent guide](../guide/setup.md) are `include_str!`d into
the binary, so the site and the CLI serve the same bytes. Tests hold them to
it: every gerenuk command a guide shows is fed through the real argument
parser, every `--flag` it names exists on some command, every `[tool.gerenuk]`
key it shows is one the config deserializer accepts (and every accepted key is
named by a guide), the triage ladder names every `run_all` reason by the exact
label the CLI prints, and no page may exceed 60 lines — one that grows past a
screenful stops being read.
