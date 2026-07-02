# ADR-0007: Review finder waves with adversarial verification and a per-assertion RED gate

Date: 2026-07-02
Status: Accepted
Issue: #1004

## Context

The feature workflow's two review stages were broad and self-judging:

- The PR `reviewers` step dispatched six wide dimensions (Architecture,
  Security, Performance, Correctness, Conformance-to-AC, Test-quality), each
  reviewer both finding and judging its own findings, and each posting its own
  review to GitHub — with a recurring pending-review failure mode in the
  per-reviewer posting mechanics.
- The `bdd_review` step was a single broad "BDD/test quality" reviewer.
- The `red` step required failure evidence per test target, not per assertion.

The PR #1001 retrospective showed this structure leaking defects: that PR
passed both `bdd_review` and the `reviewers` phase yet shipped a tautological
BDD assertion (a constant compared against itself), a vacuous Then step,
retention tests asserting a compile-time fact, an unpinned cap boundary, and a
buffer-reclaim bug — all subsequently caught by an 8-angle find → verify
review. Broad self-judging reviewers converge on plausible-sounding summaries;
tautological and vacuous assertions survive because nobody is tasked with
refuting findings or with proving each assertion can fail.

## Decision

Restructure both review stages in `workflow-config.json` (mirrored in
`examples/config.json`, `docs/workflow.md`, and guarded by
`tests/workflow_config_template.rs` / `tests/workflow_docs.rs`):

1. **PR `reviewers` step becomes three waves — find → verify → single-post.**
   - *Wave 1*: parallel narrow finders, read-only, no GitHub writes, each given
     the PR number and ONE mechanical angle (line-by-line hunk scan,
     removed-behavior audit, cross-file tracer, security,
     performance/efficiency, reuse + altitude with same-defect-class grep, test
     falsifiability). Findings are structured as file:line, a one-line summary,
     and a required concrete failure scenario — vague findings die here.
     Conformance-to-AC leaves the wave; it stays in the standalone
     `conformance` step, which now explicitly covers documentation and
     protocol updates.
   - *Wave 2*: one adversarial verifier per deduped finding, prompted to
     REFUTE; verdicts CONFIRMED / PLAUSIBLE / REFUTED must quote the
     proving/disproving line. Skipped when Wave 1 finds nothing.
   - *Wave 3*: the master posts exactly one submitted (non-PENDING) review with
     all surviving findings inline. The per-reviewer posting mechanics — and
     their pending-review failure mode — are deleted. Multi-finder convergence
     on one line is a severity signal.
2. **`bdd_review` becomes three narrow parallel finders** (Gherkin discipline,
   Falsifiability, Coverage with both-sides boundary pinning); findings must
   quote the offending line and give a concrete fix. No verify wave — scope
   is small.
3. **Per-assertion RED evidence gate on the `red` step**: failure evidence is
   required per new Then step and per new test assertion, not per test target
   — every new assertion must individually be shown to fail before
   implementation. A tautology cannot produce that evidence, so it is rejected
   by construction.

## Consequences

- Finder prompts are mechanical and narrow, so findings are checkable; the
  adversarial verify wave filters hallucinated or vague findings before
  anything reaches the PR.
- Exactly one submitted review per PR: the pending-review failure mode and
  duplicate review noise disappear; finders never touch GraphQL.
- Tautological/vacuous test assertions are rejected structurally (per-assertion
  RED evidence) rather than depending on a reviewer noticing them.
- More sub-agent dispatches per review (finders + verifiers), traded for higher
  precision; Wave 2 is skipped on clean diffs to bound the cost.
- `fix_reviews`, `push_fixes`, `resolve_threads`, `conformance` and `pre_merge`
  semantics are unchanged; guard tests pin the native config and both mirrors
  to an identical token set so they cannot drift apart silently.
