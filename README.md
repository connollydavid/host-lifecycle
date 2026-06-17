# host-lifecycle

The token-free lifecycle tool for an agentic project (e.g. `agentic-acme`). It does the mechanical,
rule-bound work — allocating zero-padded register numbers, scaffolding
milestones / decisions / personas, and validating that the tree matches its
index — so the agent does not spend tokens on it.

Names come from [`host-grammar`](https://github.com/connollydavid/host-grammar),
the same crate [`host-lint`](https://github.com/connollydavid/host-lint) checks
against, so what this generates is exactly what the checker accepts
(generator/checker symmetry).

    host-lifecycle validate <dir>          # every NNNN-slug entry is well-formed
    host-lifecycle next <dir>              # print the next zero-padded number

Validating a `call/` dir also runs the **scope gate** (anti-ouroboros): a live
(`accepted`) decision must carry a `Scope:` header and must not be
`Scope: methodology`. The methodology is owned by the template spine, not
re-litigated as a project decision; once settled upstream, a methodology decision
is retired the MADR way — `Status: superseded by the spine`, in place — and so is
no longer `accepted` and passes the gate.

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

Both `--check` and `--apply` also cover spec files (`.allium`/`.tla`/`.cfg`), which
`host-lint`'s scannable set omits — so a migration's declared substitutions reach
spec-internal cross-references too, and the summary line reports how many spec files
were included. (The rewrite stays map-only, so running it over plain-text spec
bodies is safe.)

`host-lint` stays a pure detector that faults on tells and never reads the
dictionary; all rename policy lives here, applied once, token-free.

## Software — the bare store with worktrees

The *Where* room (the software under test) embeds as a **bare object store plus
named worktrees**, not a gitlink submodule (the template's `call/0010`):
`<name>.git/` is the shared store, `<name>/` is the canonical worktree at the
pinned SHA, and `<name>.<line>/` are parallel worktrees — one per agent or live
release branch. `software` realises and audits that layout from a `.host-software`
recipe.

    host-lifecycle software --materialize <dir>    # clone the bare store(s) + worktrees
    host-lifecycle software --check <dir>          # each canonical worktree at its pin?
    host-lifecycle software --verify-build <dir>   # rebuild from the pin; artifact reproduces?

The recipe is a root `.host-software` file, one git-config-style stanza per
component (`#` comments, blanks ignored):

    # .host-software
    [software "host-lint"]
        url       = https://github.com/connollydavid/host-lint.git
        pin       = 2ef53995855e4ec363ba5b587b176d49b9aad7a5
        worktrees = host-lint.review
        worktree  = host-lint.256k perf/256k-single-context a0506f2

A component carries two worktree forms. The bare **`worktrees`** list names
parallel dirs whose branch is derived from the `<line>` suffix and whose tree is
created at the component `pin`. The explicit **`worktree = <dir> <branch> <pin>`**
form (repeatable) pins a parallel line to its *own* branch and *own* commit — use
it whenever a parallel line is not simply the canonical pin on a renamed branch,
so `--materialize` reproduces it faithfully instead of silently landing it at the
canonical pin.

- **`--materialize`** clones each `<name>.git` (setting the remote-tracking
  refspec `git clone --bare` omits), adds the canonical worktree `<name>/` at
  `pin`, initialises nested submodules per worktree, adds each listed `worktrees`
  parallel worktree on a branch named by its `<line>` suffix, and creates each
  explicit `worktree` line on its own branch at its own pin (`-B`). Idempotent —
  anything already present is skipped — and the trees are gitignored, materialised
  locally from the recipe.
- **`--check`** verifies each component's bare store and canonical worktree exist
  and the worktree sits at the recorded `pin` — the audit that replaces a submodule
  gitlink's `git submodule status`. It also flags **worktree-absence hazards**: a
  tracked symlink whose target is **not itself tracked here** points into a
  separately-materialized path (a software worktree, or a sub-path of a tool
  submodule) and dangles wherever that path is not materialized (a fresh clone, CI,
  a partial submodule init), so it is reported as a `HAZARD` (`call/0005`). A
  symlink to a submodule *root* (a tracked gitlink) is fine — it resolves to the
  empty dir git leaves on checkout. Each explicit `worktree` line is audited at its
  own branch and pin too. Exit 1 on a missing/drifted component or a hazard, 0 when
  all are at their pin and no tracked symlink reaches into a worktree.

### Reproducible builds — the production anchor

Software *initiated* under the methodology must have **reproducible builds**: its
deployed artifact is byte-reproducible from the pinned source plus a recorded build
recipe. That is what makes the pin a true production anchor (a clean rebuild from the
pin equals what is deployed) rather than just a source pin. A component records the
provenance in its stanza:

    [software "host-lint"]
        url          = https://github.com/connollydavid/host-lint.git
        pin          = 2ef5399...
        build        = cargo build --release --locked
        toolchain    = rust-1.84.0
        deploy       = host-lint                 # which line ships (canonical or a worktree dir)
        artifact     = target/release/host-lint <sha256>   # worktree-relative path + expected hash
        repro-exempt = call/0007                 # escape clause — see below

- **`--check`** also audits provenance (cheap, no build): the `deploy` line must be a
  recorded worktree, a `repro-exempt` must cite an existing decision, and where the
  `artifact` is present in the canonical worktree its hash must match the record.
- **`--verify-build`** is the proof (the heavy lane, for a CI job): it materialises a
  throwaway worktree at the `pin`, runs `build`, hashes `artifact`, and fails unless it
  reproduces the recorded sha.

**Escape clause.** Pre-existing/migrated software (not initiated under the methodology)
may not be reproducible yet. It may carry `repro-exempt = call/NNNN` citing a recorded
**case decision** (a software-scoped `call/` decision documenting why and the interim
provenance); `--verify-build` then reports it (warn) and skips the rebuild comparison,
while `--check` still requires the citation to resolve. The exemption is meant to be
retired as the component converges on reproducibility — it is never available to
greenfield software.

## Upgrade — version to version

Adopting the methodology is one event; the template then moves on, and an adopted
repo must **upgrade** across the revision span — re-applying spine changes *and*
the structural migrations a span introduced (e.g. re-embedding the software as a
bare store). A doc diff shows the prose; it does not say "convert the submodule"
or "bump a tool." The template carries an `UPGRADING.md` **ledger** that does, one
`[upgrade "<revision>"]` stanza per action, keyed by the revision it landed at:

    [upgrade "8c28e33"]
        title    = Software is a bare store with worktrees (call/0004)
        action   = Convert the embedded submodule — MIGRATION.md "Converting an existing submodule".
        requires = host-lifecycle v0.3.0

    host-lifecycle upgrade <dir>   # actions newer than the repo's .host stamp

`upgrade` reads the stamp's revision and prints every ledger entry **strictly
newer** than it — decided by git ancestry against the template, so same-day
revisions order correctly (a date cannot). Fetch the template to the target
revision first; an entry the local template cannot resolve is treated as pending
(the repo is behind it). The list is the to-do for `stamped → current`.

## Book — publish the rooms

The methodology defines five rooms and two spec formats; `book` is the one
canonical way to **publish** them, so an adopter does not hand-roll an mdBook
generator that drops a room or re-derives the `call/0005` src-scoping wrong.

    host-lifecycle book <dir> [--dry-run]   # generate book.toml + docs/ + SUMMARY.md
    host-lifecycle book --check <dir>        # fail unless every room renders a page

The book **title** is the `.host` stamp's `name` (so it is deterministic regardless
of the checkout directory), falling back to the directory name when the stamp
carries none:

    # .host
    template = "https://github.com/connollydavid/host-template"
    revision = "<sha>"
    name     = "agentic-host"

The landing page is a dedicated **home** (a prefix chapter), so no room becomes the
site's front page: a repo `README.md`/`home.md` is used verbatim if present, else a
generated overview linking each room.

`book` writes a `book.toml` with **`src = "docs"`** (never `src = "."`, which would
walk the un-materialized worktrees — `call/0005`) and regenerates `docs/` from
scratch: a `docs/SUMMARY.md` in **lifecycle order** — Cast (Who) → Plan + specs
(What/When) → Software (Where) → Call (Why) → Reference/CLAUDE (How) → Memory — with
every spec rendered as a fenced code page — including specs nested in
`spec/<topic>/` subdirectories, whose path is mirrored in the page tree — and a
**Where stub** parsed from `.host-software` (component, url, pin, worktrees,
materialize command — read from the committed recipe, so no worktree need be on
disk). Run it in CI before `mdbook build`; `book.toml` and `docs/` are generated
output, gitignored.

A decision whose MADR `Status:` is `superseded`/`deprecated`/`rejected` is
**record-layer**: `book` moves it out of its live room into a trailing
**"Archive / Record"** section, prepends a banner, and suffixes its nav label
(e.g. `(superseded)`), so retired decisions are not shipped as current chapters.
`Status:` is the only record signal — `book` does not infer record-ness from the
naming-audit's `.host-lintignore`, which carries unrelated meanings.

`--check` is the stub-coverage gate: it fails (exit 1) naming any room that has
source material but renders no page with content, so a generator that drops a room
(or ships a content-free page) cannot pass green. A room with no source — a fresh
`call/`, a project with no `.host-software` yet — is legitimately empty and skipped.

## Pinning a released version

A release tag is **annotated**, so `git ls-remote <repo> v0.6.2` resolves to the
tag *object*, not the commit — recording that as a submodule gitlink is silently
wrong. Pin the dereferenced commit:

    git ls-remote https://github.com/connollydavid/host-lifecycle 'v0.6.2^{}'   # the commit
    git rev-list -n1 v0.6.2                                                      # same, locally

Released into the public domain (Unlicense).
