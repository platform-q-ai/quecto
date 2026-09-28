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

## Mutation testing (CI only)

Mutation testing runs in CI, never on a workstation: every mutant is a rebuild
plus a test run, and local runs have exhausted memory. When `merge-requested` is
applied, `.github/workflows/mutation.yml` diffs the PR against its base
(`git diff origin/master...HEAD`) and runs `cargo mutants --in-diff` on the
changed lines only, sharded across runners (one shard per 4 mutants, at most
16, `--shard k/N`). A diff of more than 64 mutants still runs, with more
mutants per shard, and its summary says it is oversize; beyond about 190 the
shards may run out of time and the report fails as incomplete. Each shard's
`mutants.out` is uploaded as an artifact and summarised in its job summary; the
**Mutation Testing (diff)** job combines the shards, lists every MISSED and
TIMEOUT mutant (`file:line:col: mutation`), and fails when any mutant is
MISSED or TIMEOUT, or when the shards did not test and classify every planned
mutant. A TIMEOUT fails like a MISSED: a change under which the tests hang is
not caught, so make the tests fail fast under it (a bounded wait, an iteration
cap a test asserts). A diff with no mutable Rust code (docs-only, tests-only)
passes without building. A push cancels a run in progress (it also removes the
label).

The check is advisory: it is not a required status check, and making it one
is the repository owner's branch-protection decision. It is never skipped (a
skipped required check would count as passing): in a run started by any other
label or by a push, **Mutation Testing (diff)** fails with "not run for this
event", as it does when a run is cancelled, so a head shows it passing only
after mutation testing ran on it and passed. Re-apply `merge-requested` to
replace a failure of that kind.

`.cargo/mutants.toml` holds the configuration: tests run under nextest in the
Workspace Tests shape (`--workspace --features quecto-agentic-harness/test-support`),
and `additional_cargo_args` selects `--bins --lib --test contracts --test
integration` for both the build (`cargo nextest run --no-run`) and the test
phase, so a mutant builds and runs only those targets. Test files, test-support
modules and binary entry points are not mutated. CI skips the baseline run
(Workspace Tests already prove the unmutated tree passes) and sets explicit
limits: 1500 s per build, 900 s per test run. Failing tests are retried once
(`NEXTEST_RETRIES=1`); a test that fails under a mutant and then passes on the
retry counts as passing, so a flaky test can turn a caught mutant into a MISSED
one, never the reverse.

A MISSED mutant means no test that ran failed when that changed line was
altered. The cucumber BDD suites, the real-process targets (`parent_loss`,
`selected_termination`) and the docs and architecture suites do not run here,
so code that only they pin shows as MISSED and the check fails. Judge each
MISSED line (they are common in `application/**/use_cases`, which BDD
scenarios drive): if no test or scenario asserts the changed behaviour, add a
test that fails with the mutation; if a scenario does, prefer adding a unit or
contract test at the use case's boundary as well, or say in the PR why the
scenario is enough. To see what a diff would mutate without building anything
(listing only; do not run the mutants locally):

```bash
git diff origin/master...HEAD > mutants-pr.diff   # in the checkout; mutants-pr.diff is scratch
cargo mutants --list --in-diff mutants-pr.diff
rm mutants-pr.diff
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
