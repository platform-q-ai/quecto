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
cargo test --workspace --lib --bins
cargo test -p quecto-agentic-harness
cargo test -p quecto-tui
cargo test -p quecto-api
cargo test -p quecto-mcp
cargo test -p quecto-runtime-manager
cargo test -p quecto-line-io
```

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

Include in your PR description:

- What changed and why.
- How you tested it.
- Any user-facing docs/config updates.
- Any security, migration, compatibility, or operational notes.

A good PR is small enough to review, has evidence-backed tests/checks, and leaves the repository easier to understand than it found it.
