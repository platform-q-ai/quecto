# PR #1920 review fixes

## Intake and disposition
Read PR diff, both reviews (5172470153 and 5172533068), all six inline comments, and #1836. Six comments describe three overlapping valid verification gaps; no production regression is claimed or inferred.

Checklist:
- Replace dependency substring/denylist checks with parsed affirmative paths; reject filesystem, grouped/qualified bypasses and trailing-comment camouflage.
- Characterize populated query snapshots across lifecycle states, detachment, ordering, and concurrent whole/duplicate-free inventory.
- Exercise query-only public dispatch, exact complete wire objects/all five labels, and zero-call malformed/missing/unsupported rejection paths.

## RED
`cargo test -p quecto-agentic-harness --test architecture query_dependency_guard_rejects_filesystem_imports` failed at the correct-behavior assertion: the existing predicate accepted `use std::fs;` and `fs::read`. Output: /tmp/1920-red.log. Regression runs the old predicate extracted without behavioral changes. Removing the affirmative parser fix restores this failure.

Application/adapter findings concern missing characterization, not known runtime faults; their tests will be falsified using temporary behavioral mutations, then restored.

## GREEN and sweep
Replaced both reviewed predicates with syntax-parsed dependency paths (`syn`, dev-only), expanding grouped imports and validating each qualified path independently. Positive/rejection probes pass, including std filesystem module imports and unapproved paths hidden beside approved comments. 49 architecture tests pass. Initial full library run: 4368 passed. Strict all-target clippy passes after matches!-style refactor.
Sweep: the related infrastructure whole-line allowlist was the in-scope sibling and is fixed/tested. No other `contains(allowed)` or `allowed_application_dependencies` guards remain in the harness tests/application owner. Other `for forbidden` hits are unrelated documentation/template vocabulary checks, not dependency authorization.

## Characterization and independent review
- Query Running-only mutant: 3 new tests failed; restored; all 4 owner tests pass.
- Public listing/kill-coupling mutant: both query-only success tests failed; restored; all 9 adapter tests pass.
- Independent local review found an attribute traversal gap: serde derive was invisible. Added a failing derive rejection probe, then parsed derive paths and allowed only approved query attributes/dependencies. Architecture suite is GREEN again (49 tests).
- Full `cargo test -p quecto-agentic-harness`: passed, including 4370 library tests and every integration/doc target.
- Required BDD features: 3 features, 40 scenarios, 318 steps passed (`--features test-support --test bdd -- --input 'tests/features/script_managed_{environments_slice2,liveness_slice3,runtime_slice5}.feature'`). An initial repository-prefixed glob selected zero scenarios; corrected above.
- `cargo clippy -p quecto-agentic-harness --all-targets -- -D warnings`, formatting, and diff whitespace checks passed.

No finding was dismissed as invalid. Both review rounds describe the same three valid verification gaps. Runtime listing semantics remain unchanged; the only query instrumentation is cfg(test).
