---
name: verify
description: Run the agentic-host gate sweep — validate plan/ and call/, software --check (pins, spec lanes, obligations), book --check, and the commit-hook tell test. Use before declaring a milestone done or a migration complete.
---

# verify

You run the gates that make the host trustworthy from a fresh session. Done means
the whole sweep is green, not one check.

## Do

1. `host-lifecycle validate plan/` and `host-lifecycle validate call/` → `ok`
   (every `NNNN-slug` well-formed; accepted decisions carry a `Scope:` ≠ methodology).
2. `host-lifecycle software --check <dir>` → each worktree at its pin; **no HAZARD**
   — this also enforces the spec lanes (a `.allium` needs `check`+`analyse` in CI and
   a `.obligations` manifest; a `.tla` needs a TLC lane) and the worktree-symlink
   coherence.
3. For each `.allium`, `host-lifecycle obligations <spec> --tests <dir>` → every
   `allium plan` obligation dispositioned.
4. `host-lifecycle book --check <dir>` → every room renders a page.
5. A throwaway commit with a tell in its message → the hook blocks it.

## Reflect

An agent perceives neither the register it emits nor the restatements its own change
stales, so both are re-examined on purpose — the two arms of the living-grammar
doctrine, prompted here rather than left to chance.

- **gather** (advisory): run `host-lint gather` and triage the candidate tell-shapes it
  surfaces — a recurring word-then-numeral shape the lane does not yet catch, the
  residue the grammar misses. Propose a confirmed tell upstream to the shared grammar,
  declare it in the `LEXICON`, or leave it. The pass informs the operator; it does not
  block the gate.
- **reconcile** (binding here): run `host-lifecycle reconcile <dir>` to re-check each
  `host-reconcile`-annotated restatement of methodology against the spine truth. On a
  development host that authors its own spine changes, the verify gate is the binding
  trigger — `software --check` runs reconcile in its recheck, so a drifted restatement
  is a HAZARD, not advisory. Reword a live restatement to match the spine; box a frozen
  citation; forward-correct an immutable record (a `call/` body, a `Status: done` doc,
  `MEMORY.md`). A reconcile fix stays local and never propagates; a gathered tell
  graduates upstream.

## Judgment

Triage is the model-effort: a HAZARD or a flagged tell is a real defect to fix, not
to silence. Mind main-only CI triggers — the full sweep across every affected repo
must be green.

## MUST

No milestone is "done" and no migration "complete" until this sweep passes — no
opt-out.
