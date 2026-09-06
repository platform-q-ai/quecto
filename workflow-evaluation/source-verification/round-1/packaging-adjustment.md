# Minimal crate-local fixture adjustment

Supersedes README.md's external-include packaging limitation. Cargo.toml has no
explicit package include/exclude; no crate-local .gitignore exists. Existing
`tests/fixtures/` is the appropriate crate-owned test artifact location.

Copied all five canonical candidates byte-for-byte to:
`quecto-agentic-harness/tests/fixtures/workflow-approved-round-1/*.json`.
Canonical experiment candidates/manifest unchanged. `fixture-copy-hashes.json`
asserts identical SHA-256 and byte equality for every copy against approved manifest.
`templates_tests.rs` now includes only crate-local relative paths. No runtime or
external workspace dependency remains for these tests. Three substantive tests
retained; shorter include declarations reduce additions from 152 to 137 lines.
No extra generalized fixture framework or hash-test dependency added.

Checks (all exit 0):
- `cargo package -p quecto-agentic-harness --list --allow-dirty`: all five new fixture
  paths present in package file list. This lists files; it does not build/publish.
- `cargo test -p quecto-agentic-harness --lib approved_candidates_`: 3 passed;
  full typed copied-candidate/source equality, roundtrip/binding, schema checks.
- `cargo test -p quecto-agentic-harness --lib domain::workflow::`: 63 passed.
- `cargo test -p quecto-agentic-harness --lib workflow_spec`: 25 passed.
- `rustfmt --edition 2024 --check` on both owned source files; `git diff --check`.

Also reconstructed expected templates.rs from HEAD plus exactly the six approved
old→new guidance substitutions and asserted whole-file byte equality with working
source. See `approved-diff-verification.json`: no other template changes, refactor
unchanged. Frozen evaluator hashes verified intact. No commit or agent interaction.

Package membership and crate-local paths are verified, but no extracted-package
build was attempted; existing unrelated package dependencies/tests may have their
own standalone constraints. Parent must include the five new crate fixtures with
the test/source commit. Future candidate changes should intentionally version the
crate fixture copies without editing canonical historical experiment artifacts.
