---
name: upgrade
description: Upgrade an adopted project across template revisions — apply the UPGRADING.md ledger actions not yet applied (by ledger position), record each through the tool, and re-apply spine doc changes. Use for a case-(c) project when the template has moved on.
---

# upgrade

You move an adopted project forward across the template revision span — re-applying
spine changes **and** the structural migrations the span introduced.

## Do

1. Make the template present at the target revision: `upgrade` reads `UPGRADING.md`
   from `<root>/host-template/` or a registered submodule carrying it. If absent,
   register it and check it out at the target:
   `git submodule add <template-url> host-template && (cd host-template && git checkout <target>)`.
2. `host-lifecycle upgrade <dir>` lists the ledger entries **not yet applied, by
   ledger position** — never by git ancestry (ledger SHAs are a linear-commit
   artifact and some are orphaned from HEAD, so ancestry mis-classifies them). A
   legacy single-`revision` stamp is migrated **once** to a derived `baseline`
   automatically — no manual edit. `host-lifecycle upgrade --next` prints the single
   next safe action.
3. For each pending entry, apply its `action` (e.g. convert a submodule to a bare
   store; untrack a worktree symlink; wire a verification lane + skills; add a
   `.obligations` manifest), honoring its `requires` (a minimum tool version), then:
   **`host-lifecycle upgrade --record <id>`** (id, unambiguous prefix, or ledger
   ordinal) — it validates the id, **refuses if a `depends` is unapplied**, runs the
   entry's `verify` post-condition (or requires `--unverified call/NNNN` when it has
   none), and appends an append-only claim. A late `independent` entry may be
   cherry-recorded without an earlier unrelated one; deferred entries stay pending and
   re-list. **`host-lifecycle upgrade --advance`** compacts a contiguous applied run
   into the `baseline`.
4. Re-apply the spine doc changes across the span; leave project-specifics alone.
5. **Never hand-edit `.host`.** `--record`/`--advance` write the stamp — a `baseline`
   ledger id plus an append-only `applied` set, *not* a template revision. Record the
   upgrade in `call/` and `MEMORY.md`.
6. Run the `verify` sweep (`software --check` re-checks every recorded claim).

## Judgment

A doc diff shows the prose but not the actions — the ledger is the source of truth
for *what to do*. Methodology is inherited by copy-at-version; never re-litigate a
spine rule as a project `call/`.

## MUST

A case-(c) project upgrades through the ledger, not by eyeballing a diff — no
opt-out. Apply every pending entry through `--record` (which gates `depends`/`verify`)
before `--advance`. Never hand-edit the stamp, and never order by git ancestry.
