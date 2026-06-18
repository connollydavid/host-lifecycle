---
name: classify
description: Preview a repo before bringing it under the agentic-host methodology — print the migration case (a/b/c) and draft the rename map + merge plan. Use at the very start of adopting or upgrading a project, before changing anything.
---

# classify

The first lifecycle phase: look before you touch. You determine the repo's starting
state and write down what the migration will do, applying nothing yet.

## Do

1. `host-lifecycle classify <dir>` → the case: **a** (no `CLAUDE.md`), **b** (a
   `CLAUDE.md` predating the methodology), **c** (a `.host` stamp — already adopted).
2. `host-lint --all` → naming tells in live tracked files (you will fix these).
3. `host-lint --log` → tells in history (informational; never rewritten outside a
   Deep rewrite).
4. Write down the **rename map** (each ordinal-named file → its content-named home
   under `plan/`) and, for case (b), the **merge plan**.

## Judgment

The case decides the governance path; the mode (Preview → Shallow PR → Staged →
Deep) decides the blast radius. Default to Shallow. Choose Deep only when rewriting
history buys coherence worth the disruption, and never on history you do not own.

## MUST

Every adoption or upgrade **MUST** start here — there is no opt-out. Apply nothing
in this phase; the output is a plan, handed to `adopt`/`remap`.
