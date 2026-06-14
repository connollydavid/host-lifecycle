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
to preview. It records the adopted template revision in a `.agentic-host` stamp
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

Released into the public domain (Unlicense).
