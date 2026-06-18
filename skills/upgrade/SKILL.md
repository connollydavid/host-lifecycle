---
name: upgrade
description: Upgrade an adopted project across template revisions — apply the UPGRADING.md ledger actions newer than the .host stamp, re-apply spine doc changes, and re-stamp. Use for a case-(c) project when the template has moved on.
---

# upgrade

You move an adopted project forward across the template revision span — re-applying
spine changes **and** the structural migrations the span introduced.

## Do

1. Fetch the template to the target revision.
2. `host-lifecycle upgrade <dir>` — prints every `UPGRADING.md` ledger entry **newer**
   than the repo's stamped revision (by git ancestry, so same-day revisions order
   right).
3. Apply each printed entry's `action`, in order (e.g. convert a submodule to a bare
   store; untrack a worktree symlink; wire a verification lane + skills; add a
   `.obligations` manifest). Honor each entry's `requires` (a minimum tool version).
4. Re-apply the spine doc changes across the span; leave project-specifics alone.
5. Re-stamp `.host` to the target revision; record it in `call/` and `MEMORY.md`.
6. Run the `verify` sweep.

## Judgment

A doc diff shows the prose but not the actions — the ledger is the source of truth
for *what to do*. Methodology is inherited by copy-at-version; never re-litigate a
spine rule as a project `call/`.

## MUST

A case-(c) project upgrades through the ledger, not by eyeballing a diff — no
opt-out. Apply every entry newer than the stamp before re-stamping.
