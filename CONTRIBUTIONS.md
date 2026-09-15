# Contributing to Quecto

Thank you for your interest in contributing to Quecto. This repository is a Rust workspace containing the core agentic harness plus terminal, API, MCP, runtime-management, and shared protocol crates.

## Before you start

1. Open or find an issue for non-trivial changes so scope and acceptance criteria are clear.
2. Keep changes focused. Avoid drive-by refactors, unrelated formatting churn, or broad rewrites in documentation/bugfix PRs.
3. Do not commit secrets, personal config, generated build output, logs, or local runtime artifacts.

## Development setup

Install the local hooks from the repository root:

```bash
scripts/install-hooks.sh
source scripts/activate-hooks.sh
```

The hooks are intended to prevent common repository-quality failures. Do not bypass them with `--no-verify`.

Build the workspace:

```bash
cargo build --workspace
```

## Common checks

Run formatting before submitting:

```bash
cargo fmt --all -- --check
```

Run package or workspace tests relevant to your change:

```bash
# One workspace shape for every invocation (see README "Common checks"):
# `-p <crate>` resolves a different dependency feature set and forces rebuilds.
cargo test --workspace --features quecto-agentic-harness/test-support --bins --lib
cargo test --workspace --features quecto-agentic-harness/test-support --bins --test architecture --test contracts
cargo test --workspace --features quecto-agentic-harness/test-support --bins --test integration --test docs --test parent_loss --test selected_termination
cargo test --workspace --features quecto-agentic-harness/test-support --bins --test api_bdd --test mcp_bdd
bash scripts/run-bdd-shards.sh --suite non-real-bdd --shards 24 --timeout 12m
bash scripts/run-bdd-shards.sh --suite tui-bdd --package quecto-tui --test-target tui_bdd --shards 8 --timeout 12m
```

CI runs the libtest targets (everything but the cucumber `*_bdd` targets) under
`cargo nextest run … --profile ci`, one process per test scheduled across
binaries; with `cargo-nextest` installed the pre-push gate does the same and
the `--lib` row above drops from ~26 s to ~20 s on 32 cpus:

```bash
cargo nextest run --workspace --features quecto-agentic-harness/test-support --bins --profile ci --lib
```

`run-bdd-shards.sh` builds the test binary once and runs it per shard; with
`--coverage` the instrumented build lives in a persistent `target/llvm-cov-<suite>`
directory, so a second coverage run only re-executes the scenarios (~70 s instead
of ~200 s for the non-real lane). CI uses 8 shards on its 4-vcpu runners; 24 is
still the fastest local setting on a large box.

Run clippy for touched packages, or the strict workspace command when practical:

```bash
cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings
```

Repository scripts may provide narrower/faster checks used by hooks and CI, including:

```bash
scripts/check-quality.sh
scripts/check-bdd-quality.sh
scripts/check-bdd-tags.sh
```

## Testing expectations

- Documentation-only changes should at least pass formatting-sensitive checks where applicable and should keep links/commands accurate.
- Code changes should include tests that would fail without the implementation.
- Bug fixes should include a regression test that reproduces the wrong behavior first.
- Behavior-preserving refactors should characterize existing behavior before restructuring.
- BDD features live under package `tests/features/` directories where applicable.

## Architecture expectations

Quecto uses layered and feature-oriented boundaries in different crates. Preserve the existing direction of each package:

- Keep domain/application logic independent from transport and interface concerns where a crate enforces Clean Architecture boundaries.
- Prefer ports/traits at application boundaries and concrete adapters in infrastructure/interface layers.
- Keep the `quecto-line-io` protocol cap/framing behavior centralized rather than duplicating wire constants.
- Avoid adding production code paths solely for tests; use existing test-support features and test doubles.

## Documentation expectations

Update relevant documentation when behavior, configuration, CLI flags, environment variables, or public APIs change. Common docs to consider:

- Root [README.md](README.md) for workspace-level changes.
- Package READMEs for package-specific behavior.
- Harness docs under [quecto-agentic-harness/docs/](quecto-agentic-harness/docs/).
- API/UDS protocol docs when wire shapes or endpoints change.

## Security and secrets

- Never commit real API keys, OAuth tokens, passwords, private keys, certificates, cookies, or credential files.
- Never print secrets in test failures, logs, examples, screenshots, or issue comments.
- Use obvious placeholders such as `YOUR_API_KEY` or `sk-EXAMPLE-not-a-real-key` in docs/tests.
- If your change touches command execution, credentials, sandboxing, MCP tools, network access, auth, or runtime management, call that out explicitly in the PR description.
- Report suspected vulnerabilities privately; see [SECURITY.md](SECURITY.md).

Before publication or large releases, run a secret scan such as:

```bash
gitleaks detect --source . --redact
```

## Pull request checklist

Before asking for review, contributors are expected to run the built-in Quecto workflow through at least two complete adversarial-review loops on their change. Each loop should use the repository workflow rather than an ad-hoc prompt: run the narrow read-only finders, adversarially verify every surviving finding, fix accepted issues, and repeat until two full cycles have completed. Record the workflow evidence in the PR description, including what each loop found or that no findings survived verification.

Include in your PR description:

- What changed and why.
- How you tested it.
- Evidence from at least two full built-in adversarial-review workflow loops.
- Any user-facing docs/config updates.
- Any security, migration, compatibility, or operational notes.

A good PR is small enough to review, has evidence-backed tests/checks, has survived repeated adversarial review, and leaves the repository easier to understand than it found it.
