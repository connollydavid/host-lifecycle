# host-lifecycle

The token-free lifecycle tool for an agentic host. It does the mechanical,
rule-bound work — allocating zero-padded register numbers, scaffolding
milestones / decisions / personas, and validating that the tree matches its
index — so the agent does not spend tokens on it.

Names come from [`host-grammar`](https://github.com/connollydavid/host-grammar),
the same crate [`host-lint`](https://github.com/connollydavid/host-lint) checks
against, so what this generates is exactly what the checker accepts
(generator/checker symmetry).

    host-lifecycle validate <dir>          # every NNNN-slug entry is well-formed
    host-lifecycle next <dir>              # print the next zero-padded number

It also drives the mechanical half of *migration* — bringing an existing repo
under the methodology (see the template's `MIGRATION.md`):

    host-lifecycle classify <dir>          # migration case: a|b|c
    host-lifecycle adopt <dir> <revision>  # scaffold cast/ plan/ call/ + write the stamp
    host-lifecycle version <dir>           # print the adopted template revision

`adopt` is idempotent (existing rooms are left untouched) and takes `--dry-run`
to preview. It records the adopted template revision in a `.host` stamp
at the repo root, so a later upgrade knows exactly which revision to diff from.

## Remap — the enforced adoption rename

An adoption clean-break renames old ordinal concepts (`Phase 4`) to content names.
`remap` does that **deterministically** from a declared dictionary, so the rewrite
is map-only by construction — no token outside the dictionary is ever touched, so
there is no fabrication and no drift across files (the failure mode of a free-form,
fan-out rewrite).

    host-lifecycle remap --check <dir>            # tells left after the dictionary applies
    host-lifecycle remap --apply <dir> [--dry-run]  # apply it (archive-first; clean git tree required)

The dictionary is a root `.host-remap` file, `old => new` per line (`#` comments,
blanks ignored), matched case-insensitively and at word boundaries (`Phase 1`
rewrites `Phase 1` but not `Phase 12`), longest match first (`Phase 5.0` before
`Phase 5`):

    # .host-remap
    Phase 5.0 => mcp-integration bring-up
    Phase 5   => mcp-integration
    Phase 4   => command-execution

- **`--check`** applies the dictionary in memory, runs `host-lint` over the result
  (honouring the repo's `.host-lint-allow`), and reports every tell that *remains* —
  the undispositioned ones, each needing a dictionary entry, an allow-list entry, or
  an acknowledgement. Exit 1 on a remaining flag, 3 on a warning, 0 when clean. So a
  clean `--check` is the gate: every detected concept has been consciously handled.
- **`--apply`** writes the substitutions, skipping VCS/build dirs and submodule
  paths (it never descends into the software submodules). It refuses unless the git
  tree is clean, so the prior commit is the verbatim archive the methodology
  requires. The `.host-remap` file is itself transient: it names the old concepts,
  so a second stage removes it once the remap is verified (its durable copy lives in
  the migration decision record).

`host-lint` stays a pure detector that faults on tells and never reads the
dictionary; all rename policy lives here, applied once, token-free.

## Software — the bare store with worktrees

The *Where* room (the software under test) embeds as a **bare object store plus
named worktrees**, not a gitlink submodule (the template's `call/0010`):
`<name>.git/` is the shared store, `<name>/` is the canonical worktree at the
pinned SHA, and `<name>.<line>/` are parallel worktrees — one per agent or live
release branch. `software` realises and audits that layout from a `.host-software`
recipe.

    host-lifecycle software --materialize <dir>  # clone the bare store(s) + worktrees
    host-lifecycle software --check <dir>         # each canonical worktree at its pin?

The recipe is a root `.host-software` file, one git-config-style stanza per
component (`#` comments, blanks ignored):

    # .host-software
    [software "host-lint"]
        url       = https://github.com/connollydavid/host-lint.git
        pin       = 2ef53995855e4ec363ba5b587b176d49b9aad7a5
        worktrees = host-lint.review

- **`--materialize`** clones each `<name>.git` (setting the remote-tracking
  refspec `git clone --bare` omits), adds the canonical worktree `<name>/` at
  `pin`, initialises nested submodules per worktree, and adds each listed parallel
  worktree on a branch named by its `<line>` suffix. Idempotent — anything already
  present is skipped — and the trees are gitignored, materialised locally from the
  recipe.
- **`--check`** verifies each component's bare store and canonical worktree exist
  and the worktree sits at the recorded `pin` — the audit that replaces a submodule
  gitlink's `git submodule status`. It also flags **worktree-absence hazards**: a
  host-tracked symlink whose target resolves into a worktree path dangles wherever
  the software is not materialized (a fresh clone, CI), so it is reported as a
  `HAZARD` (`call/0005`). Exit 1 on a missing/drifted component or a hazard, 0 when
  all are at their pin and no tracked symlink reaches into a worktree.

Released into the public domain (Unlicense).
