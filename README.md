# host-lifecycle

The token-free lifecycle tool for an agentic host. It does the mechanical,
rule-bound work — allocating zero-padded register numbers, scaffolding
milestones / decisions / personas, and validating that the tree matches its
index — so the agent does not spend tokens on it.

Names come from [`host-grammar`](https://github.com/connollydavid/host-grammar),
the same crate [`host-lint`](https://github.com/connollydavid/host-lint) checks
against, so what this generates is exactly what the checker accepts
(generator/checker symmetry).

    host-lifecycle validate <dir>   # every NNNN-slug entry is well-formed
    host-lifecycle next <dir>       # print the next zero-padded number

Released into the public domain (Unlicense).
