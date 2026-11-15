# Inputs for the outline

Read these files; they are the contract.

- Pitch: `docs/pitch.md`

## Requirement ids to cover

R1, R2, R3, R4

## Scenarios

- `full`: The default: brief, dev, gate, add checks, review and QA in parallel, triage, fix, verify, record. Steps: brief, dev, gate, add_checks, review_qa, triage, fix, verify, record
- `real-data`: Phases that touch real data or paid APIs: full, plus a small real probe run before record. Steps: brief, dev, gate, add_checks, review_qa, triage, fix, verify, probe, record
- `small`: Low-risk phases (docs, config, one module): one combined review and QA, then fix and gate. Steps: brief, dev, gate, review_combined, fix, record
- `ui`: Phases that change pages: full, with QA driving a browser at desktop and phone width. Steps: brief, dev, gate, add_checks, review_qa, triage, fix, verify, record

## Default scenario

`small`. Any other choice, and any `set`, needs a one-line `reason`.

## Bounds

At most 12 phases, each one session (size S, M or L).

## The outline to review

See `.orch/plan.md` (draft).
