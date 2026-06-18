---
name: embed
description: Embed the software under development as a bare store with worktrees (the Where room) and maintain the .host-software recipe. Use when adding software to an agentic project, materializing it locally, or checking it is at its pin.
---

# embed

You put the software in the *Where* room as a **bare store with worktrees**, not a
submodule, and keep `.host-software` as the reproducibility anchor.

## Do

1. Commit a `.host-software` recipe: one `[software "<name>"]` stanza per
   component (`url`, pinned SHA, worktree set; build provenance —
   `build`/`toolchain`/`deploy`/`artifact`; per-platform `[build "<name>"
   "<platform>"]` if it ships on several platforms). Gitignore the trees.
2. `host-lifecycle software --materialize <dir>` — clone each `<name>.git` bare
   store and add the canonical worktree `<name>/` at its pin (+ parallel lines).
3. Recreate generated skill symlinks (`link-skills.sh`) and the software skill
   link after materializing — never git-track a symlink into an un-materialized
   path (worktree-absence coherence).
4. `host-lifecycle software --check <dir>` — every worktree at its pin; no HAZARD.

## Judgment

If the software is already a gitlink submodule, convert it in place (preserve the
pin, de-register the gitlink, write `.host-software`) — moving no software commit.
Software *initiated* under the methodology has reproducible builds; record the
recipe and prove it with `--verify-build`.

## MUST

The software lives as a bare store with worktrees, recorded in `.host-software` —
no opt-out. The pin replaces a gitlink as the audit anchor; never push a host
commit whose pin is unpushed.
