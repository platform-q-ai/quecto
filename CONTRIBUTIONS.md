# Contributing to Quecto

Thank you for your interest in contributing to Quecto. This repository is a Rust workspace containing the core agentic harness plus terminal, API, MCP, runtime-management, and shared protocol crates.

## Before you start

1. Open or find an issue for non-trivial changes so scope and acceptance criteria are clear.
2. Keep changes focused. Avoid drive-by refactors, unrelated formatting churn, or broad rewrites in documentation/bugfix PRs.
3. Do not commit secrets, personal config, generated build output, logs, or local runtime artifacts.

## Development setup

Focused architecture hook tests require a GNU-compatible executable named `timeout` on `PATH`
(see the [GNU timeout manual](https://www.gnu.org/software/coreutils/manual/html_node/timeout-invocation.html)).
On Linux, install GNU coreutils using your distribution's package manager.
On macOS, install [Homebrew coreutils](https://formulae.brew.sh/formula/coreutils) and expose its unprefixed commands:

```bash
brew install coreutils
export PATH="$(brew --prefix coreutils)/libexec/gnubin:$PATH"
```

`gtimeout` alone does not satisfy the tests' `timeout` invocation. This prerequisite does not imply macOS full-suite support or verification.

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

## Local pre-push and authoritative CI

The pre-push hook is a **lint-only**, no-build gate targeting 20 seconds on a
warm development checkout (about 30 seconds is the timing acceptance bound).
It always runs these seven commands, regardless of changed files:

- `scripts/check-quality.sh`
- `scripts/check-golden-manifest.sh`
- `bash scripts/check-layout-ratchet-tests.sh`
- `bash scripts/check-layout-ratchet.sh`
- `scripts/check-bdd-quality.sh`
- `scripts/check-bdd-tags.sh`
- `cargo fmt --all -- --check`

The repository-quality chain, BDD quality, BDD tags and formatting run
independently in parallel; the hook waits for each result and any failure
blocks the push. The repository-quality chain stops at its first failure.
No Clippy, compiled architecture/contracts/docs tests, or nextest detection
runs in the hook. Run relevant focused tests during development; a successful
local hook is not proof that merge validation passed.

All six relocated checks remain in `.github/workflows/ci.yml`, workflow
**Authoritative Merge CI**. Apply `merge-requested` to request the authoritative
merge run; reapply the label after every push so the current head is validated.
`workflow_dispatch` is also available for diagnosis. Do not bypass required CI.

| Relocated check | Job ID (check name) | Step name |
| --- | --- | --- |
| Strict workspace Clippy (all targets, test-support, complexity lints) | `static-quality` (Static Quality) | Strict workspace Clippy |
| Strict standalone quecto-tui Clippy | `static-quality` (Static Quality) | Strict standalone quecto-tui Clippy |
| Strict standalone quecto-agentic-harness Clippy | `static-quality` (Static Quality) | Strict standalone quecto-agentic-harness Clippy |
| Architecture tests | `workspace-tests` (Workspace Tests) | All workspace unit and integration tests except sharded BDD targets |
| Port contract tests | `workspace-tests` (Workspace Tests) | All workspace unit and integration tests except sharded BDD targets |
| Docs tests | `workspace-tests` (Workspace Tests) | All workspace unit and integration tests except sharded BDD targets |

The compiled invariants run with the workspace/test-support/bins shape under
nextest's `ci` profile; workspace tests depend on Static Quality and Dependency
Policy. Full tests, BDD and dependency checks remain CI responsibilities;
coverage runs nightly on master. Actual merge requirements are controlled by
repository protection/rulesets as well as workflow admission, not this table.

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
binaries. For optional focused local testing with `cargo-nextest` installed,
the `--lib` row above drops from ~26 s to ~20 s on 32 cpus:

```bash
cargo nextest run --workspace --features quecto-agentic-harness/test-support --bins --profile ci --lib
```

To find slow BDD scenarios, run a lane with per-scenario timings and one
scenario at a time per shard (with the default 25-way co-scheduling a
scenario's clock also runs while others hold the executor):

```bash
QUECTO_BDD_TIMING=1 QUECTO_BDD_CONCURRENCY=1 QUECTO_BDD_KEEP_SCRATCH=1 \
  bash scripts/run-bdd-shards.sh --suite non-real-bdd --shards 32 --timeout 20m
cat .git/non-real-bdd-shards.*/shard-*.log | grep '^BDD_TIMING' | sort -t$'\t' -k3 -rn | head
# columns: wall seconds, CPU seconds (process + reaped children), feature, scenario
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

## Mutation testing (local, scoped, optional)

CI does not run mutation testing. When a change needs its test strength
checked, run `cargo mutants` locally on the change's own diff, restricted to
the tests that exercise it, and under a memory cap (a full run is a rebuild
plus a test run per mutant, and unbounded local runs have exhausted memory):

```bash
git diff origin/master...HEAD > /tmp/pr.diff
systemd-run --user --scope -q -p MemoryMax=20G \
  cargo mutants --in-place --in-diff /tmp/pr.diff --baseline=skip \
    --timeout 300 --build-timeout 900 -- -E 'test(/swarm/)'
```

Run it in a scratch worktree (`--in-place` mutates the checkout). A MISSED
mutant means no selected test failed when that line changed; a TIMEOUT means
the tests hang under it, so make them fail fast (a bounded wait, an asserted
iteration cap). `.cargo/mutants.toml` holds the shared configuration.

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
- `quecto-image` owns what an image is: the MIME allowlist and its wire spelling (`ImageMime::parse_exact`), signature sniffing (`ImageMime::sniff`), the size limit (`MAX_IMAGE_BYTES`), base64 (strict for images from outside, lenient only for reading headers of images already held) and header parsing (`dimensions`, `is_animated_gif`). A `UserImageBlock` is made only from an admitted `ImageAttachment` and carries its `ImageMime`. Admit images from outside only as an `ImageAttachment` (`new`, `from_bytes`, `validate_images`); never keep a second copy of any of these. It is a pure leaf crate (base64 and serde only), so the harness domain may depend on it as it does on `base64`.
- Avoid adding production code paths solely for tests; use existing test-support features and test doubles.

### Harness layout ratchet (#2358)

[`quecto-agentic-harness/tests/architecture/layout.rs`](quecto-agentic-harness/tests/architecture/layout.rs)
is the sole executable policy for budgets, capability placements and permitted
roles. It follows the [target architecture wiki](https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture).
L0 moves no source files and declares no existing capability migrated.

The ratchet counts immediate-child production `.rs` files, including `mod.rs`
and excluding `*_tests.rs`. Counts must exactly equal their table entries.
When moving flat files into capability folders, lower the corresponding budget
to the actual count in the same PR. A budget may never rise: unused headroom
must not be preserved or refilled. Count-neutral renames and replacements are
allowed; this is a count ratchet, not a filename baseline.

`scripts/check-layout-ratchet.sh` compares the budget table with the merge base.
CI and pre-push reject raised budgets and new rows, while allowing lowered
budgets. Remove a row only when its directory no longer exists; a missing
budgeted directory also fails the Rust check. The gate requires exactly one
unambiguous budget definition, including commented definitions. This historical
comparison is separate from the architecture test's current-tree check.

A migrated capability permits its declared role directories plus `mod.rs` and
its sibling `mod_tests.rs`, but no other top-level source or test files. Omitted
roles are fine; each role directory present must contain at least one `.rs`
file anywhere beneath it, including nested modules. The wiki's two-files-per-role
guidance remains a review rule, not an
automated minimum. Add or update a placement row with its wiki section when
introducing a capability, keeping the wiki and executable policy in sync.
Do not copy budget values, placements or role lists into documentation.

The checker canonicalizes the repository root once, so a symlinked ancestor
of the checkout is allowed. Symlinks, special entries and unreadable entries
inside the scanned tree are rejected, with path-specific remediation.

Run focused gates locally; leave full suites to CI:

```bash
bash scripts/check-layout-ratchet-tests.sh
scripts/check-layout-ratchet.sh
cargo test --workspace --features quecto-agentic-harness/test-support --bins --test architecture layout:: -- --nocapture
```

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
