# Architecture and contract guardrails run before push

## Scenario

Given Quecto follows clean/hexagonal architecture
When a developer pushes a feature branch
Then the pre-push hook must run the architecture boundary tests
And the pre-push hook must run the port contract test suite
And new public domain/application ports should require matching contract coverage or an explicit allowlist entry.

## Expected behavior

- `cargo test --test architecture` remains part of the pre-push test wave.
- `cargo test --test contracts` is part of the same pre-push test wave.
- The architecture tests reject inward-layer imports from outward layers.
- The architecture tests reject direct runtime I/O from the application layer.
- The architecture tests reject newly added public ports that are not represented in `tests/contracts.rs` unless explicitly documented as allowlisted.
- CI should run both architecture and contract tests so remote checks match local guardrails.
