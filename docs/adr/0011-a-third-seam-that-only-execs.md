# 0011 — A third impure seam, and it only execs

Status: accepted. Amends [0001](0001-two-impure-seams.md), which stays in force
for everything else.
Amended by [0014](0014-run-all-delegates-to-a-fallback.md), which lets the
same seam exec a configured fallback command with a payload on its stdin.

## Decision

`src/pytest.rs` spawns a process. It is the third and last such place in the
crate, alongside `tyf::Runner::run` and the private `git::Git::output`.

It is allowed because of its shape, not because running pytest was needed:

- **Exec-and-replace, not run-and-parse.** On Unix the final act is
  `CommandExt::exec` — gerenuk's process *becomes* pytest. Nothing is captured,
  nothing is parsed, nothing is reinterpreted, because after the call there is
  no gerenuk left to do any of it. Windows has no `exec`, so it spawns, waits
  and propagates the code; one `#[cfg]`, behaviourally identical.
- **Everything around it is pure.** Runner resolution, argv assembly and the
  one-line summary are ordinary functions over data, unit-tested with no spawn
  anywhere near them. The seam itself is four statements.
- **It is terminal.** No caller consumes its result, so it cannot grow a parser,
  a progress bar or a retry loop without that being a visible, deliberate
  change to this record.

## Why it had to be gerenuk that runs pytest

A selection has three possible answers and an argument list can only express
two:

| Outcome | pytest argv |
|---|---|
| run these | `pytest a.py::t1 b.py` |
| run everything | `pytest` |
| run **nothing** | *(there is none)* |

`pytest $(gerenuk print-node-ids)` therefore inverts the best case: a diff that
impacts no tests would run the full suite. The empty selection has to
short-circuit before a shell ever interpolates it, and that means owning the
invocation. `--dry-run` exists for scripting, but its human output states a
decision — it is deliberately not a bare interpolatable list.

## Cost

- The invariant "two seams" is now "three seams", and a reader has to know why
  the third is different. Hence this record.
- After the exec, gerenuk cannot distinguish its own never-happened failures
  from pytest's exit `2`. Acceptable: every pre-exec failure has already exited
  before pytest existed.
- On Windows there is no `exec`, so the seam spawns and waits — and two
  outcomes there have no exit code of pytest's to propagate: a signal-terminated
  child, which has none at all, and a code outside a byte, since a Windows exit
  code is a full `i32` and a crashed pytest arrives as something like
  `0xC0000005`. Both become `2`, aliasing gerenuk's own operational failure.
  Acceptable for the same reason as above, and the Unix path — the one the hook
  contract is written for — has neither case.
- `exec` has no test that proves the process was replaced. The integration
  tests assert on the argv a stubbed pytest recorded and on the propagated exit
  code, which is the observable half.

## What would make us revisit

Wanting to *know* how the run went — retries, a summary line after the fact,
telemetry comparing selected against full-suite times (phase 4). Any of those
needs run-and-parse, which is a different seam with a different argument, and
would supersede this record rather than stretch it.
