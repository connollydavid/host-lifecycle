---
name: remap
description: Apply the live-file rename from ordinal names to content names via the .host-remap dictionary, dispositioning every naming tell. Use during a migration to rename plan/milestone files deterministically and clear host-lint flags.
---

# remap

You turn the rename map from `classify` into a deterministic, map-only substitution
— the migration's live layer.

## Do

1. Write the rename map into a `.host-remap` dictionary (`old => new` per line).
2. `git mv` each ordinal-named file to its content-named home.
3. `host-lifecycle remap --check <dir>` until clean: disposition **every** remaining
   tell — a dictionary entry, a `.host-lint-allow` entry (genuine vocabulary), or an
   excluded path. (The discipline mirrors `obligations`: nothing left undispositioned.)
4. Commit the clean tree (the verbatim archive), then `host-lifecycle remap --apply
   <dir>` (makes only the declared substitutions — map-only by construction).
5. Commit, then `git rm .host-remap`; its durable copy goes in a `call/` decision.

## Judgment

The record layer (`MEMORY.md`, closed milestone bodies) is append-only history —
**exclude** it with `.host-lintignore`, never rewrite it. Shallow = one PR; Staged =
governance → tooling → bulk rename; Deep (human) = archive-first, then rewrite history.

## MUST

A migration renames through the dictionary, not by hand — no opt-out. Every tell is
dispositioned before `--apply`.
