# Repository agent guidance

For repository behavior, treat current source code, tests, build configuration,
and generated assets as the authority. Treat asset manifests as provenance
records that must be verified against their generators and consumers.
Documentation describes the implementation; it does not override it.

Before changing the repository:

- inspect the current branch and working tree without disturbing unrelated or
  user-owned changes;
- read the production code and tests for the area being changed;
- verify generated assets and opaque fixtures through their consumers, build
  tools, hashes, and source manifests;
- preserve license and attribution text unless a verified upstream change
  requires an update.

Documentation should be compact and operational. Avoid milestone histories,
transient benchmark results, copied implementation detail, speculative plans,
and line-number references. Prefer commands and invariants that can be checked
against the current tree.

Comments should explain non-obvious invariants, ownership, units, compatibility
constraints, or safety boundaries. Do not narrate edits, retain superseded
design history, restate obvious code, or use comments as a planning journal.

Before finishing a material change:

- run `cargo fmt --all -- --check`;
- run `cargo check --workspace --all-targets --all-features`;
- run focused tests and then the broadest practical workspace test suite;
- run `cargo doc --workspace --all-features --no-deps` after documentation or
  public doc-comment changes;
- run `git diff --check` and inspect the final diff for accidental files.

Do not stage, commit, push, or open a pull request unless the user requests it.
