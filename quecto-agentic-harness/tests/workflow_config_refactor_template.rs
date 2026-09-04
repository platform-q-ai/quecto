//! Structural assertions for the native `refactor` workflow template and the
//! shape-based selector in `workflow-config.json`. These encode the lessons of
//! PR #1031: GREEN-first characterization instead of RED, mutation-based
//! falsifiability, a frozen characterization suite, a source-text-assertion
//! ban, a deletion ledger, four-class parity evidence, and shape-based
//! template routing.

mod common;

use serde_json::Value;

// Slice 2 (workflow-composable-templates PRD §3.2 / AC7): the refactor
// template is pinned against the canonical `workflows/` folder — the single
// source of truth — via the same directory loader the runtime uses.
fn read_native_config() -> Value {
    common::canonical_workflow_config()
}

fn template<'a>(config: &'a Value, id: &str) -> &'a Value {
    config["workflow"]["templates"]
        .as_array()
        .expect("workflow templates should be an array")
        .iter()
        .find(|template| template["id"] == id)
        .unwrap_or_else(|| panic!("`{id}` workflow template should exist"))
}

fn steps(config: &Value) -> &[Value] {
    template(config, "refactor")["steps"]
        .as_array()
        .expect("refactor workflow steps should be an array")
}

fn step<'a>(config: &'a Value, key: &str) -> &'a Value {
    steps(config)
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("refactor template should have a `{key}` step"))
}

fn guidance<'a>(config: &'a Value, key: &str) -> &'a str {
    step(config, key)["guidance"]
        .as_str()
        .unwrap_or_else(|| panic!("step `{key}` should have string guidance"))
}

fn step_index(config: &Value, key: &str) -> usize {
    steps(config)
        .iter()
        .position(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("refactor template should have a `{key}` step"))
}

// --- Selector routing -------------------------------------------------------

#[test]
fn selector_routes_by_issue_shape_not_hardcoded_feature() {
    let config = read_native_config();
    let selector = config["workflow"]["selector_prompt"]
        .as_str()
        .expect("selector_prompt should be a string");
    assert!(
        !selector.contains("single Quecto repository workflow template, 'feature'"),
        "selector must no longer hard-code the feature template"
    );
    // Relationship-bearing clauses, not independent tokens: an inverted
    // selector (zero-behaviour-change -> 'feature') must fail these.
    for clause in [
        "zero behaviour change to code the project ships (refactor, consolidation, extraction, dedup, moving state, renames — acceptance criteria are structural/parity-only), select 'refactor'",
        "adding or altering observable behaviour, and maintenance work such as docs, CI, tooling or dependency changes — select 'feature'",
        "mixes a behaviour change with a zero-behaviour-change refactor, STOP and report that the issue must be split",
    ] {
        assert!(
            selector.contains(clause),
            "selector should contain the routing clause `{clause}`: {selector}"
        );
    }
}

#[test]
fn templates_have_mutually_exclusive_when_to_use() {
    let config = read_native_config();
    let feature = template(&config, "feature")["when_to_use"]
        .as_str()
        .expect("feature when_to_use");
    let refactor = template(&config, "refactor")["when_to_use"]
        .as_str()
        .expect("refactor when_to_use");
    assert!(
        feature.contains("behaviour-adding") || feature.contains("behaviour-changing"),
        "feature when_to_use should be scoped to behavioural work: {feature}"
    );
    assert!(
        feature.contains("'refactor'"),
        "feature when_to_use should point refactors at the refactor template"
    );
    assert!(
        refactor.contains("zero behaviour change"),
        "refactor when_to_use should require zero behaviour change: {refactor}"
    );
    assert!(
        refactor.contains("split"),
        "refactor when_to_use should require mixed issues to be split"
    );
}

// --- Step ordering ----------------------------------------------------------

#[test]
fn refactor_pipeline_steps_are_exactly_ordered() {
    let config = read_native_config();
    let expected = [
        "hooks",
        "scope",
        "characterize",
        "mutation",
        "freeze_review",
        "restructure",
        "parity",
        "version_bump",
        "commit",
        "push",
        "pr",
        "reviewers",
        "fix_reviews",
        "push_fixes",
        "resolve_threads",
        "conformance",
        "pre_merge",
        "cleanup",
    ];
    let actual: Vec<&str> = steps(&config)
        .iter()
        .map(|s| s["key"].as_str().expect("step key should be a string"))
        .collect();
    assert_eq!(
        actual, expected,
        "refactor step keys must match exactly, in order — no inserted, removed, or reordered steps"
    );
}

// --- Readiness gate + parity contract (scope) -------------------------------

#[test]
fn scope_step_runs_blocking_and_warning_readiness_gate() {
    let config = read_native_config();
    let g = guidance(&config, "scope");
    for token in [
        "READINESS GATE",
        "BLOCKING",
        "WARNING",
        "named consequence",
        "__UNRESOLVED__",
        "draft",
        "approved",
    ] {
        assert!(g.contains(token), "scope guidance should contain `{token}`");
    }
}

#[test]
fn scope_step_keeps_structural_goals_out_of_tests() {
    let config = read_native_config();
    let g = guidance(&config, "scope");
    assert!(
        g.contains("NEVER become test assertions"),
        "structural goals must be review-time checks, not test assertions"
    );
    assert!(
        g.contains("PARITY CONTRACT"),
        "scope must produce a parity contract"
    );
    assert!(
        g.contains("boundary cases") && g.contains("full-set vs visible-window"),
        "parity contract must enumerate boundary cases including full-set vs window"
    );
}

// --- Characterization (GREEN-first) ------------------------------------------

#[test]
fn characterize_step_is_green_first_on_unmodified_code() {
    let config = read_native_config();
    let g = guidance(&config, "characterize");
    assert_eq!(step(&config, "characterize")["phase"], "green");
    assert!(
        g.contains("UNMODIFIED code") && g.contains("pass GREEN"),
        "characterization tests must pass GREEN on the unmodified code"
    );
    assert!(
        g.contains("fails on master means you misunderstood"),
        "a failing characterization test signals a misunderstanding, not RED"
    );
}

#[test]
fn characterize_step_bans_source_text_assertions() {
    let config = read_native_config();
    let g = guidance(&config, "characterize");
    // #1290: behavioural tests still must not grep private implementation
    // detail, but architecture/repository-policy/docs-lockstep/build-policy/
    // migration-invariant checks may use source/document text as the contract.
    assert!(
        g.contains(
            "architecture, repository-policy, docs-lockstep, build-policy, or migration-invariant"
        ) && g.contains("source/document text may be the observable contract"),
        "characterization must allow source/document text for architecture and policy invariants"
    );
    assert!(
        g.contains("Behavioural product tests must NOT read production source")
            && g.contains("private helper names")
            && g.contains("verifies the diff, not the behaviour"),
        "behavioural source-text assertions on incidental implementation detail remain banned"
    );
    assert!(
        !g.contains("HARD BAN"),
        "the absolute HARD BAN wording must not remain after #1290"
    );
    assert!(
        g.contains("step body MUST perform that action"),
        "step bodies must perform the actions their titles claim"
    );
    assert!(
        g.contains("render harness"),
        "TUI behaviour must be pinned at the render-harness runtime class"
    );
}

// --- Mutation gate (replaces RED) --------------------------------------------

#[test]
fn mutation_step_replaces_red_with_mutation_evidence() {
    let config = read_native_config();
    let g = guidance(&config, "mutation");
    assert_eq!(step(&config, "mutation")["phase"], "red");
    for token in [
        "per assertion, not per test target",
        "confirm the test FAILS",
        "revert the mutation",
        "NO MUTATION RESIDUE",
        "no production-code changes",
        "mutation log",
        "hollow",
    ] {
        assert!(
            g.contains(token),
            "mutation guidance should contain `{token}`"
        );
    }
}

// --- Freeze review ------------------------------------------------------------

#[test]
fn freeze_review_dispatches_read_only_finders_then_freezes() {
    let config = read_native_config();
    let g = guidance(&config, "freeze_review");
    for token in [
        "read_only: true",
        "workflow_spec",
        "Falsifiability",
        "Coverage",
        "Gherkin discipline",
        "FREEZE",
        "hash",
        "READ-ONLY",
        "gate violations",
    ] {
        assert!(
            g.contains(token),
            "freeze_review guidance should contain `{token}`"
        );
    }
    assert!(
        step_index(&config, "freeze_review") < step_index(&config, "restructure"),
        "the suite must be reviewed and frozen before the refactor begins"
    );
}

// --- Restructure ---------------------------------------------------------------

#[test]
fn restructure_step_extends_abstractions_and_keeps_deletion_ledger() {
    let config = read_native_config();
    let g = guidance(&config, "restructure");
    assert_eq!(step(&config, "restructure")["phase"], "refactor");
    assert!(
        g.contains("EXTEND the abstraction") && g.contains("shoehorning"),
        "restructure must mandate extend-don't-shoehorn"
    );
    assert!(
        g.contains("DELETION LEDGER"),
        "restructure must keep a deletion ledger"
    );
    assert!(
        g.contains("pass UNMODIFIED"),
        "the frozen suite is the oracle and must pass unmodified"
    );
    // PR #1036 review retro: the ledger only covered production lines, so a
    // deleted test's coverage (the C1-control case) vanished silently.
    assert!(
        g.contains("deleted TESTS") && g.contains("port the assertion"),
        "the deletion ledger must cover deleted test assertions: name where the behaviour stays pinned or port it"
    );
    // Parity quirks belong inside the shared mechanism, not caller-side state
    // (the legacy_csi_tail pattern).
    assert!(
        g.contains("INSIDE the shared mechanism") && g.contains("caller-side state"),
        "restructure must place parity quirks inside the shared mechanism, never as caller-side state"
    );
    // Consolidation completeness: introducing a shared helper obliges a sweep
    // for the remaining hand-rolled copies.
    assert!(
        g.contains("CONSOLIDATION COMPLETENESS") && g.contains("grep the crate"),
        "restructure must mandate a consolidation-completeness sweep for remaining duplicates"
    );
    // The freeze must not incentivize contorting production code: no cfg(test)
    // forks/shims to keep frozen tests compiling.
    assert!(
        g.contains("cfg(test)") && g.contains("gate violation"),
        "restructure must forbid production cfg(test) forks/shims added to avoid touching frozen tests"
    );
    // No speculative API surface.
    assert!(
        g.contains("outside its own module"),
        "restructure must require every new pub item to have a consumer outside its own module"
    );
}

// --- Parity evidence ------------------------------------------------------------

#[test]
fn parity_step_requires_all_four_evidence_classes() {
    let config = read_native_config();
    let g = guidance(&config, "parity");
    for token in [
        "Behavioural",
        "Visual",
        "Performance",
        "Quantitative",
        "ZERO modifications",
        "freeze manifest",
        "never the frozen tests",
    ] {
        assert!(
            g.contains(token),
            "parity guidance should contain `{token}`"
        );
    }
}

// --- Reviewers --------------------------------------------------------------------

#[test]
fn reviewers_step_uses_refactor_weighted_angles_and_single_post() {
    let config = read_native_config();
    let g = guidance(&config, "reviewers");
    for token in [
        "DELETION LEDGER",
        "Equivalence tracer",
        "Test falsifiability",
        "source-text assertions",
        "SINGLE parallel batch",
        "SUBMIT the review",
        "submittedAt != null",
        // PR #1036 review retro: the refactor wave dropped the feature wave's
        // reuse/altitude angle — exactly the angle that catches unfinished
        // consolidation — and no angle owned architecture placement.
        "Reuse + altitude",
        "consolidation completeness",
        "Clean architecture",
        "caller-side state",
        "outside its own module",
    ] {
        assert!(
            g.contains(token),
            "reviewers guidance should contain `{token}`"
        );
    }
}

// --- Conformance --------------------------------------------------------------------

#[test]
fn conformance_verifies_structural_goals_and_greps_for_source_text_tests() {
    let config = read_native_config();
    let g = guidance(&config, "conformance");
    assert!(
        g.contains("verified HERE by code inspection"),
        "structural goals are verified at conformance, not by tests"
    );
    // #1290: source-text assertions fail only when inappropriate for behavioural
    // tests or incidental; architecture/policy/docs/build/migration invariants OK.
    assert!(
        g.contains("FAIL only when they are inappropriate behavioural tests")
            && g.contains("incidental implementation details"),
        "conformance must fail only inappropriate behavioural/incidental source-text assertions"
    );
    assert!(
        g.contains(
            "architecture, repository-policy, docs-lockstep, build-policy, and migration-invariant"
        ) && g.contains("text is the observable contract"),
        "conformance must allow source/document text where it is the contract"
    );
    assert!(
        !g.contains("NO test asserts on production source text"),
        "the absolute production-source-text ban must not remain after #1290"
    );
    assert!(
        g.contains("grep the test tree")
            && g.contains("treat behavioural/incidental hits as a FAIL"),
        "conformance must mandate the mechanical grep audit with the nuanced fail rule"
    );
    // PR #1036 review retro: speculative pub API and test-only production
    // paths are verified mechanically at conformance.
    assert!(
        g.contains("outside its own module"),
        "conformance must audit that every new pub item has a consumer outside its own module"
    );
    assert!(
        g.contains("cfg(test)"),
        "conformance must grep the diff for production cfg(test) additions"
    );
    assert!(
        g.contains("CONFORMANCE: PASS") && g.contains("CONFORMANCE: FAIL"),
        "conformance must emit a machine-checkable verdict"
    );
}

// --- Guards -----------------------------------------------------------------------

#[test]
fn guards_block_commit_before_parity_and_forbid_merge() {
    let config = read_native_config();
    let guards = template(&config, "refactor")["guards"]
        .as_array()
        .expect("refactor guards should be an array");
    let commit_guard = guards
        .iter()
        .find(|g| {
            g["commands"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x == "git commit"))
        })
        .expect("a git commit guard should exist");
    let commit_commands: Vec<&str> = commit_guard["commands"]
        .as_array()
        .expect("commit guard commands")
        .iter()
        .filter_map(|c| c.as_str())
        .collect();
    assert_eq!(
        commit_commands,
        ["git commit", "git push"],
        "both commit AND push must be guarded before parity evidence is complete"
    );
    assert_eq!(
        commit_guard["before_step_key"], "version_bump",
        "commit/push are blocked until parity evidence is complete"
    );
    let merge_guard = guards
        .iter()
        .find(|g| {
            g["commands"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x == "gh pr merge"))
        })
        .expect("a merge guard should exist");
    let merge_commands: Vec<&str> = merge_guard["commands"]
        .as_array()
        .expect("merge guard commands")
        .iter()
        .filter_map(|c| c.as_str())
        .collect();
    assert_eq!(
        merge_commands,
        ["git merge", "gh pr merge"],
        "both local git merge AND gh pr merge must be forbidden"
    );
    assert_eq!(merge_guard["before_step_key"], "cleanup");
    assert!(
        merge_guard["message"]
            .as_str()
            .expect("merge guard message")
            .contains("does NOT merge"),
        "the workflow never merges"
    );
}

// --- Runtime parity ----------------------------------------------------------------

#[test]
fn runtime_default_templates_are_empty_after_template_globalization() {
    // Production no longer bundles repo-local workflow defaults; template
    // behavior tests above exercise explicit canonical fixtures instead.
    assert!(quecto::domain::workflow::default_templates().is_empty());
}
