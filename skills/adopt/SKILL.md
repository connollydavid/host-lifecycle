---
name: adopt
description: Establish governance and scaffold the rooms for an agentic project — copy or merge the CLAUDE.md spine, create cast/ plan/ call/, and write the .host stamp. Use after classify, when bringing a repo under the methodology.
---

# adopt

You establish governance and scaffold the five rooms, then stamp the adopted
template revision.

## Do

1. Governance, by case (from `classify`):
   - **(a)** Copy the template's `CLAUDE.md` in unchanged at the chosen revision,
     then record the repo's own build/test/style conventions under a
     project-specifics heading.
   - **(b)** **Merge.** For each existing rule: *subsumed* by the spine (drop, note
     in provenance), *project-specific* (keep under project-specifics), or
     *conflicts* (stop, get a human ruling). Preserve attribution/license.
   - **(c)** Upgrade instead — use the `upgrade` skill.
2. `host-lifecycle adopt <dir> <revision>` — creates `cast/ plan/ call/`
   idempotently and writes the `.host` stamp (`template`, `revision`, `adopted`).

## Reflect

A migration puts the whole project in view — both arms of the living-grammar doctrine
fire here.

- **gather**: a migration is the richest sweep for emergent tells. Run `host-lint
  gather` and triage each candidate tell-shape with the operator: propose it upstream
  to the shared grammar, declare it in the `LEXICON`, or leave it. The lane's grammar
  grows from what adoption surfaces.
- **reconcile**: adoption runs the full reconcile once — `host-lifecycle reconcile
  <dir>` re-checks every `host-reconcile`-annotated restatement of methodology against
  the freshly adopted spine. Reword a drifted restatement to match the spine; box a
  frozen citation; forward-correct an immutable record. Prefer pointing at the spine
  over restating it — a restatement is a reconciliation liability you choose to carry.

## Judgment

The case-(b) merge is the model-effort here; the scaffolding is mechanical. Do not
impose a rule that contradicts the repo's existing style. Build at least one
persona (`cast/`) with the human before planning the work it serves.

## MUST

The rooms and the `.host` stamp are mandatory for every agentic project — no
opt-out. The `revision` must be exact; a later case-(c) upgrade diffs from it.
