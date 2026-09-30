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
- Avoid adding production code paths solely for tests; use existing test-support features and test doubles.

### Harness layout ratchet (#2358)

`quecto-agentic-harness/tests/architecture/layout.rs` enforces the L0 slice of
[the target architecture](https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture).
Paths below are relative to the harness `src/` directory. This slice freezes
existing flat-file growth and validates capability placement; it does not move
source files, change APIs, or declare existing capabilities migrated.

The checked-in immediate-child Rust-file budgets are:

| Directory | Maximum |
| --- | ---: |
| `domain` | 71 |
| `application` | 45 |
| `interface` | 6 |
| `infrastructure` | 28 |
| `composition` | 24 |
| `interface/cli` | 106 |
| `infrastructure/tools` | 122 |
| `infrastructure/persistence` | 36 |
| `application/swarm` | 23 |
| `domain/swarm` | 15 |

These are frozen baselines, not a generic 20-file cap. Count only immediate
regular `.rs` files and exclude only filenames ending in `_tests.rs`.
`mod.rs`, inline `#[cfg(test)]` code, and other test-support Rust files count;
nested descendants and non-Rust files do not. Exactly the maximum passes;
maximum plus one fails. When a structural split reduces the count, lower the
checked-in maximum in that PR rather than leaving growth headroom. Never raise
a budget or remove a policy row to evade checks. Deliberate retirement or
repointing of a tracked directory requires a reviewed migration. Review budget
changes against the merge base: the runtime check alone cannot detect an
increased maximum. Diagnostics identify the path,
actual count, and maximum.

A separate explicit migrated-capability table opts individual capabilities into
strict shape enforcement. It is empty in L0. Each opted-in root permits only the
file `mod.rs` and its explicitly declared role directories: a role-named file,
stray direct source, `*_tests.rs`, or undeclared directory is not allowed.
Declared roles must match that specific capability's wiki tree, not a broad
layer-wide union. For example, application capabilities usually use
`use_cases`, `ports`, and `dto`; interface CLI/REPL use `controllers`,
`presenters`, and `dto`; infrastructure persistence uses `file`, `sqlite`,
`records`, `locks`, and `migrations`. Domain usually uses `entities`,
`value_objects`, `services`, and `events`, but identity only uses
`value_objects`. Composition has concern directories rather than invented
business-capability roles. Do not invent a CLI `handles` allowance or mark a
role-less capability migrated using guessed roles. Sparse layouts are valid;
this validator imposes no minimum production-file count on a role directory.

Every immediate capability directory under `domain`, `application`,
`interface`, `infrastructure`, or `composition` needs an explicit placement row
with its layer, name, target-wiki section citation, and classification enum
(`Target`, `Transitional`, or `Testing`). `Target` rows cite the full wiki
page URL with the matching layer fragment (`#domain`, `#application`,
`#interface`, `#infrastructure`, or `#composition`). `Transitional` and
`Testing` rows cite that URL with `#capability-coverage-and-naming`.
Update the wiki and placement policy together when adding a capability.
Only explicit mappings are accepted;
unknown directories are rejected, not classified by a naming heuristic.
Future target rows may precede their directories. Current special mappings are
`domain/environment_registry` and `application/agent_loop` as transitional,
and `infrastructure/test_support` as testing. Migrated capabilities must also
have a cited placement row.

Policy paths and rows must be safe and unique. Monitored directories must
exist and be directories. Inspection fails closed on read/metadata errors,
symlinks (including broken links), special entries, and non-UTF-8 entry names;
errors are not silently discarded. This slice adds no retired-path list.

Run the focused layout tests, leaving full suites to CI and retaining normal
quality hooks:

```bash
cargo test --workspace --features quecto-agentic-harness/test-support --bins --test architecture layout:: -- --nocapture
```

Fixture tests should cover budget boundaries and ratcheting, filename counting,
strict migrated roots, layer-specific roles, explicit placements and citations,
invalid policy rows, and filesystem failures alongside the real-tree check.

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
