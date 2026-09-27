---
name: ci
description: Judge the CI lanes' receipts offline — every workflow discovered from .github/workflows/ must carry a receipted outcome at the current revision. Run at session start and before reporting a turn done; record receipts citing run URLs.
---

# ci

You judge the lanes so a red one is the turn's first fact, never an email
discovered later. A receipted failure is a recorded blocker — the fix rides the
next turn — while an unreceipted lane is silence, the state the contract refuses.

## Do

1. `host-lifecycle ci <dir>` — judge offline: every lane discovered from
   `.github/workflows/` (the host root and each component) must carry a `ci`
   receipt at the revision under judgment (the host root at HEAD, each component
   at its recorded pin). Exit 1 names every finding with its remedy.
2. A run concluded since the last check? Record it:
   `host-lifecycle ci <dir> --record <label>/<lane> --run <run-url>
   [--revision <sha>] [--conclusion <c>]` — the receipt cites the run; a
   receipted failure is a recorded blocker whose fix rides the next turn.
3. Before reporting a turn done: `host-lifecycle ci <dir> --gate` — exit 0
   means every discovered lane is receipted at the judged revision (a receipted
   failure passes; the fix is the next turn's first fact). Non-zero: fix, or
   record the outcome — never stop with a lane unreceipted.

## Reflect

The receipts are the memory: a lane that ran green for months and was never
receipted is invisible to every gate — which is why the receipts exist. When a
release moves a pin, the new revision's lanes are unreceipted until its run
concludes; record the outcome the moment it does, and name the run URL in every
blocker you leave behind.
