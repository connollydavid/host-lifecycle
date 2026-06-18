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

## Judgment

Triage is the model-effort: a HAZARD or a flagged tell is a real defect to fix, not
to silence. Mind main-only CI triggers — the full sweep across every affected repo
must be green.

## MUST

No milestone is "done" and no migration "complete" until this sweep passes — no
opt-out.
