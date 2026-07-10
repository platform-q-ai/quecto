//! Structural assertions for the native `feature` workflow template in
//! `workflow-config.json`. These encode issue #814 acceptance criteria:
//! a `conformance` gate, corrected gate/required-check facts, git-add safety,
//! per-step exit criteria, de-duplicated reviewer mechanics, and repo gotchas.

mod common;

use common::read_repo_file;
use serde_json::Value;

fn read_native_config() -> Value {
    serde_json::from_str(&read_repo_file("workflow-config.json"))
        .expect("workflow-config.json should parse as JSON")
}

fn feature_template(config: &Value) -> &Value {
    config["workflow"]["templates"]
        .as_array()
        .expect("workflow templates should be an array")
        .iter()
        .find(|template| template["id"] == "feature")
        .expect("feature workflow template should exist")
}

fn steps(config: &Value) -> &[Value] {
    feature_template(config)["steps"]
        .as_array()
        .expect("feature workflow steps should be an array")
}

fn guards(config: &Value) -> &[Value] {
    feature_template(config)["guards"]
        .as_array()
        .expect("feature workflow guards should be an array")
}

fn step<'a>(config: &'a Value, key: &str) -> &'a Value {
    steps(config)
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("feature template should have a `{key}` step"))
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
        .unwrap_or_else(|| panic!("feature template should have a `{key}` step"))
}

#[test]
fn config_is_valid_json() {
    // read_native_config panics if not valid JSON.
    let _ = read_native_config();
}

#[test]
fn conformance_step_present_before_merge() {
    let config = read_native_config();

    let conformance = step(&config, "conformance");
    assert_eq!(
        conformance["phase"], "review",
        "conformance step should be in the review phase"
    );

    let g = guidance(&config, "conformance");
    assert!(
        g.contains("CONFORMANCE: PASS"),
        "conformance guidance should describe the CONFORMANCE: PASS verdict"
    );
    assert!(
        g.contains("CONFORMANCE: FAIL"),
        "conformance guidance should describe the CONFORMANCE: FAIL verdict"
    );
    assert!(
        g.contains("file:line"),
        "conformance guidance should require file:line evidence"
    );

    // Positioned after resolve_threads, before pre_merge and merge.
    assert!(
        step_index(&config, "resolve_threads") < step_index(&config, "conformance"),
        "conformance should come after resolve_threads"
    );
    assert!(
        step_index(&config, "conformance") < step_index(&config, "pre_merge"),
        "conformance should come before pre_merge"
    );
}

#[test]
fn merge_guard_requires_conformance() {
    let config = read_native_config();

    // #886: the `merge` step is removed; the no-merge guard now gates `cleanup`.
    let merge_guard = guards(&config)
        .iter()
        .find(|g| g["before_step_key"] == "cleanup")
        .expect("there should be a no-merge guard before cleanup");
    let message = merge_guard["message"]
        .as_str()
        .expect("merge guard should have a message");
    assert!(
        message.to_lowercase().contains("conformance"),
        "merge guard message should reference conformance, got: {message}"
    );
}

#[test]
fn merge_blocks_on_errored_phase_or_bypassed_gate() {
    // Hardening after the #818 incident: the terminal report step must refuse to
    // hand off a clean PR when an upstream review/fix/conformance phase errored,
    // and must reject a push that bypassed the local gate with --no-verify.
    // #886: this guidance now lives on `pre_merge` (the `merge` step was removed).
    let config = read_native_config();
    let merge_guidance = step(&config, "pre_merge")["guidance"]
        .as_str()
        .expect("pre_merge step should have guidance")
        .to_lowercase();
    assert!(
        merge_guidance.contains("errored") || merge_guidance.contains("did not actually run"),
        "merge guidance should block merging when a review/fix/conformance step errored, got: {merge_guidance}"
    );
    assert!(
        merge_guidance.contains("--no-verify") || merge_guidance.contains("bypass"),
        "merge guidance should reject a gate bypassed with --no-verify, got: {merge_guidance}"
    );
}

#[test]
fn no_stale_strings_remain() {
    // The native config and its example mirror must both be free of the stale
    // required-check / opt-out facts, otherwise the acceptance criterion can
    // report GREEN while the strings still live in the mirror config.
    for path in ["workflow-config.json", "examples/config.json"] {
        let raw = read_repo_file(path);
        assert!(
            !raw.contains("Smoke Test"),
            "{path}: the Smoke Test required-check reference should be gone"
        );
        assert!(
            !raw.contains("QUECTO_SKIP_REAL_LLM"),
            "{path}: the obsolete QUECTO_SKIP_REAL_LLM opt-out should be gone"
        );
    }
}

#[test]
fn gate_facts_describe_mock_llm_default_and_opt_in() {
    let config = read_native_config();

    for key in ["push", "pre_merge"] {
        let g = guidance(&config, key);
        assert!(
            g.contains("@mock-llm"),
            "`{key}` guidance should describe the @mock-llm lane"
        );
        assert!(
            g.contains("QUECTO_RUN_REAL_LLM"),
            "`{key}` guidance should describe the QUECTO_RUN_REAL_LLM opt-in"
        );
    }
}

#[test]
fn required_checks_reference_unit_and_mock_e2e() {
    let config = read_native_config();

    for key in ["pr", "pre_merge"] {
        let g = guidance(&config, key);
        assert!(
            g.contains("Unit Tests"),
            "`{key}` guidance should reference the Unit Tests required check"
        );
        assert!(
            g.contains("Mock LLM E2E Tests"),
            "`{key}` guidance should reference the Mock LLM E2E Tests required check"
        );
    }
}

#[test]
fn commit_has_git_add_safety() {
    let config = read_native_config();
    let g = guidance(&config, "commit");
    assert!(
        g.contains("git add -A") && g.contains("git add ."),
        "commit guidance should warn against git add -A / git add ."
    );
}

#[test]
fn action_steps_carry_done_when_criteria() {
    let config = read_native_config();
    for key in [
        "hooks",
        "scenarios",
        "tests",
        "red",
        "green",
        "verify",
        "push",
        "pr",
        "fix_reviews",
        "resolve_threads",
        "pre_merge",
    ] {
        let g = guidance(&config, key);
        assert!(
            g.contains("Done when"),
            "`{key}` guidance should carry a `Done when` exit criterion"
        );
    }
}

#[test]
fn action_steps_document_failure_handling() {
    // Item 4 also asks for "If it fails …" guidance where useful; assert it on
    // the steps where a failure path is genuinely actionable.
    let config = read_native_config();
    for key in ["red", "verify", "push"] {
        let g = guidance(&config, key);
        assert!(
            g.contains("If it fails") || g.contains("isn't exercising"),
            "`{key}` guidance should describe what to do on failure"
        );
    }
}

#[test]
fn flaky_find_gotcha_documented() {
    let config = read_native_config();
    let documented = ["push", "pre_merge"].iter().any(|key| {
        let g = guidance(&config, key).to_lowercase();
        g.contains("find.feature") && (g.contains("re-run") || g.contains("rerun"))
    });
    assert!(
        documented,
        "the load-flaky find.feature scenario should be documented with a re-run instruction"
    );
}

#[test]
fn scenarios_step_ties_to_conformance() {
    let config = read_native_config();
    let g = guidance(&config, "scenarios").to_lowercase();
    assert!(
        g.contains("conformance"),
        "scenarios guidance should note its acceptance criteria are the conformance checklist"
    );
}

#[test]
fn pre_merge_confirms_inline_findings_and_resolved_threads() {
    let config = read_native_config();
    let g = guidance(&config, "pre_merge").to_lowercase();
    assert!(
        g.contains("inline"),
        "pre_merge should confirm reviewers actually posted inline findings"
    );
    assert!(
        g.contains("resolved") || g.contains("resolve"),
        "pre_merge should confirm review threads are resolved"
    );
}

#[test]
fn reviewers_step_describes_finder_waves() {
    // Issue #1004: the PR review fan-out is restructured from six broad
    // self-judging dimensions into narrow mechanical finder angles with
    // find -> verify -> single-post waves. The full token set lives in the
    // shared helper so this native copy, examples/config.json and
    // docs/workflow.md are pinned identically and none can silently drift.
    let config = read_native_config();
    let g = guidance(&config, "reviewers");
    common::assert_reviewer_finder_waves(g, "workflow-config.json");
}

#[test]
fn conformance_step_retains_acceptance_criteria_and_documentation_checks() {
    // Issue #1004: Conformance-to-AC leaves the reviewer wave (asserted by the
    // shared finder-waves helper); the standalone `conformance` step keeps that
    // responsibility. The deleted per-dimension reviewer used to pin that AC
    // conformance explicitly covers documentation/protocol updates — that check
    // must survive on the step that now owns it.
    let config = read_native_config();
    let g = guidance(&config, "conformance").to_lowercase();
    assert!(
        g.contains("acceptance criterion") || g.contains("acceptance criteria"),
        "conformance step must verify the issue acceptance criteria: {g}"
    );
    assert!(
        g.contains("documentation"),
        "conformance step must explicitly cover documentation/protocol updates: {g}"
    );
}

#[test]
fn reviewers_step_preserves_single_batch_and_submits_non_pending() {
    // #845: expanding the dimension set must NOT change wall-clock — reviewers
    // still go out as ONE parallel batch — and reviews must be SUBMITTED, never
    // left as a PENDING draft (verified via submittedAt, not the author-side view).
    let config = read_native_config();
    let g = guidance(&config, "reviewers");
    assert!(
        g.contains("SINGLE parallel batch") || g.contains("single parallel batch"),
        "reviewers guidance should keep the single parallel batch invariant"
    );
    let lower = g.to_lowercase();
    assert!(
        lower.contains("submit") && lower.contains("pending"),
        "reviewers guidance should require submitting the review (never leaving it PENDING)"
    );
    assert!(
        lower.contains("submittedat"),
        "reviewers guidance should verify submittedAt (not just the author-side view)"
    );
}

#[test]
fn reviewers_step_requires_pr_number_not_raw_diff_and_pr_precondition() {
    // #946: reviewers must review the PR, not an pasted raw diff blob, and must
    // not be dispatched before the PR exists.
    let config = read_native_config();
    let g = guidance(&config, "reviewers");
    let lower = g.to_lowercase();
    assert!(
        lower.contains("must not be dispatched before a pr exists")
            || lower.contains("must not dispatch reviewers before a pr exists"),
        "reviewers guidance should explicitly block dispatch before a PR exists: {g}"
    );
    assert!(
        lower.contains("forbid") && lower.contains("raw diff"),
        "reviewers guidance should explicitly forbid passing a raw diff: {g}"
    );
    assert!(
        g.contains("PR number") && g.contains("gh pr diff <PR>"),
        "reviewers guidance should require passing the PR number and fetching the diff with gh pr diff <PR>: {g}"
    );
    assert!(
        lower.contains("inline") && lower.contains("on the pr"),
        "reviewers guidance should require inline comments on the PR: {g}"
    );
}

#[test]
fn feature_js_reviewers_run_finder_waves() {
    // #862 established that `.claude/workflows/feature.js` is the only copy that
    // actually DRIVES execution under Claude Code, so it is guarded directly.
    // Issue #1004 (PR #1005 review finding): the executable PR-review block must
    // carry the same find -> verify -> single-post wave structure as the native
    // config — pinned via the SAME shared helper so it cannot silently revert to
    // the six broad self-posting dimensions with every test green.
    let js = read_repo_file("../.claude/workflows/feature.js");

    // The executable reviewer dispatch block: from the PR Review phase marker to
    // the fix phase.
    let reviewer_block = js
        .split_once("phase('PR Review')")
        .expect("feature.js should have a PR Review phase")
        .1
        .split_once("// ── Fix reviews")
        .expect("feature.js reviewer block should end before fix phase")
        .0;
    common::assert_reviewer_finder_waves(reviewer_block, "feature.js PR-review block");

    let lower = reviewer_block.to_lowercase();
    assert!(
        lower.contains("submittedat") && lower.contains("pending"),
        "feature.js Wave 3 prompt should require a submitted (non-PENDING) review"
    );
    assert!(
        lower.contains("must not be dispatched before a pr exists"),
        "feature.js finder prompt should forbid dispatch before a PR exists"
    );
    assert!(
        lower.contains("forbid")
            && lower.contains("raw diff")
            && reviewer_block.contains("PR number")
            && reviewer_block.contains("gh pr diff <PR>"),
        "feature.js finder prompt should forbid raw diffs and require PR number + gh pr diff <PR>"
    );
    assert!(
        lower.contains("inline") && lower.contains("on the pr"),
        "feature.js Wave 3 prompt should require inline comments on the PR"
    );
    // The ship step must return a SCHEMA-validated {pr_number, head_sha} — a
    // prose reply once slipped a BASE PR number past a regex guard and the
    // finders reviewed the wrong PR — and the script must still sanity-check
    // the integer before dispatching finders.
    assert!(
        js.contains("SHIP_RESULT")
            && js.contains("'pr_number'")
            && js.contains("'head_sha'")
            && js.contains("schema: SHIP_RESULT"),
        "feature.js ship step should force a structured {{pr_number, head_sha}} return via schema"
    );
    assert!(
        reviewer_block.contains("Number.isInteger(ship.pr_number)")
            && reviewer_block.contains("prNumber"),
        "feature.js should validate the schema-returned PR number before dispatching finders"
    );
    assert!(
        reviewer_block.contains("PR/context: #${prNumber}"),
        "feature.js should pass only the validated PR number to finder prompts"
    );

    // Conformance-to-AC left the reviewer wave; the executable conformance
    // phase keeps the acceptance-criteria + documentation responsibility.
    let conformance_block = js
        .split_once("phase('Conformance')")
        .expect("feature.js should have a Conformance phase")
        .1
        .to_lowercase();
    assert!(
        conformance_block.contains("acceptance") && conformance_block.contains("documentation"),
        "feature.js conformance prompt should verify acceptance criteria incl. documentation"
    );
}

#[test]
fn feature_js_red_requires_per_assertion_failure_evidence() {
    // Issue #1004 mirrored into the executable feature.js: RED evidence is per
    // new Then step / per new test assertion, not per test target.
    let js = read_repo_file("../.claude/workflows/feature.js");
    let red_block = js
        .split_once("phase('RED')")
        .expect("feature.js should have a RED phase")
        .1
        .split_once("phase('BDD Review')")
        .expect("feature.js RED phase should precede BDD Review")
        .0
        .to_lowercase();
    assert!(
        (red_block.contains("per new then step") || red_block.contains("every new then step"))
            && (red_block.contains("per new test assertion")
                || red_block.contains("every new assertion")),
        "feature.js RED step must require failure evidence per new Then step and per new assertion: {red_block}"
    );
    assert!(
        red_block.contains("individually be shown to fail"),
        "feature.js RED step must require each assertion to individually be shown to fail: {red_block}"
    );
}

#[test]
fn feature_workflow_guidance_is_self_contained() {
    let config = read_native_config();

    // Regression guard: the workflow guidance must carry every instruction an
    // agent needs INLINE — it must NOT tell the agent to go read an external
    // repo doc. Agents run from the repo root and a filesystem read of
    // `docs/…` (e.g. the deleted `agent-dev-quickstart.md`) throws
    // `read failed: No such file or directory` on the orientation step of
    // every feature run. The workflow file is the complete brief.
    for s in steps(&config) {
        let g = s["guidance"].as_str().unwrap_or("");
        assert!(
            !g.contains("agent-dev-quickstart"),
            "step `{}` guidance must be self-contained, not point to an external doc: {g}",
            s["key"]
        );
    }

    // The canonical commands the quickstart used to hold now live inline in the
    // steps that need them.
    let tests = guidance(&config, "tests");
    assert!(tests.contains("cargo test -p quecto-agentic-harness --lib <name_substring>"));
    assert!(tests.contains("cargo test -p quecto-tui --lib <name_substring>"));
    assert!(
        tests.contains("`-p` takes the PACKAGE name"),
        "guidance must explain the package-vs-lib-target naming trap"
    );
    assert!(
        guidance(&config, "scenarios").contains("gh issue view <N> --json title,body,comments")
    );
    assert!(guidance(&config, "red").contains("quick targeted"));
    assert!(guidance(&config, "verify").contains("Do not manually re-run the whole suite"));
    assert!(guidance(&config, "push").contains("scripts/pre-push.sh"));
    let reviewers = guidance(&config, "reviewers");
    assert!(reviewers.contains("gh pr diff <PR>"));
    assert!(reviewers.contains("addPullRequestReview"));
    assert!(guidance(&config, "hooks").contains("git commit"));
}

#[test]
fn bdd_section_teaches_best_practice_gherkin() {
    // #953 AC1: the behaviourally-led Gherkin + step-test best-practice checklist
    // must live INLINE (no external-doc pointer). Gherkin-structure guidance sits
    // on `scenarios`; step-test (RED/hollow) discipline sits on the `tests` step
    // where it belongs — the tokens are asserted against their proper step.
    let config = read_native_config();
    let scenarios = guidance(&config, "scenarios").to_lowercase();
    for token in [
        "declarative",
        "given-when-then",
        "one behaviour per scenario",
        "implementation detail",
        "ubiquitous",
        "every acceptance criterion maps to a scenario",
    ] {
        assert!(
            scenarios.contains(token),
            "scenarios guidance should teach best-practice Gherkin token `{token}`: {scenarios}"
        );
    }

    // Step-test discipline belongs to the RED `tests` step.
    let tests = guidance(&config, "tests").to_lowercase();
    for token in ["behavioural", "deterministic", "isolated", "hollow"] {
        assert!(
            tests.contains(token),
            "tests guidance should teach step-test discipline token `{token}`: {tests}"
        );
    }
}

#[test]
fn bdd_review_is_strict_and_fixes_all_valid_concerns() {
    // #953 AC2/AC3: strict reviewer that flags every genuine best-practice
    // deviation, and the implementer must address EVERY valid concern regardless
    // of severity (fix or documented decline) before GREEN.
    let config = read_native_config();
    let g = guidance(&config, "bdd_review").to_lowercase();
    assert!(
        g.contains("strict"),
        "bdd_review should instruct a strict reviewer: {g}"
    );
    assert!(
        g.contains("every genuine") || g.contains("every best-practice"),
        "bdd_review should flag every genuine best-practice deviation: {g}"
    );
    // The best-practice checklist the reviewer explicitly checks.
    assert!(
        g.contains("declarative") && g.contains("one behaviour per scenario"),
        "bdd_review should name the best-practice checklist it verifies: {g}"
    );
    // All-valid-concerns rule.
    assert!(
        g.contains("every valid") && g.contains("regardless of severity"),
        "bdd_review should require addressing every valid concern regardless of severity: {g}"
    );
    assert!(
        g.contains("decline"),
        "bdd_review should allow a documented decline for invalid concerns: {g}"
    );
    assert!(
        g.contains("green"),
        "bdd_review should require concerns resolved before GREEN: {g}"
    );
}

#[test]
fn bdd_review_dispatches_three_narrow_finders() {
    // Issue #1004: bdd_review becomes three narrow parallel finders whose
    // findings must quote the offending line and give a concrete fix.
    let config = read_native_config();
    let g = guidance(&config, "bdd_review");
    let lower = g.to_lowercase();

    assert!(
        g.contains("Gherkin discipline"),
        "bdd_review should name the Gherkin discipline finder: {g}"
    );
    assert!(
        g.contains("Falsifiability"),
        "bdd_review should name the Falsifiability finder: {g}"
    );
    assert!(
        g.contains("Coverage"),
        "bdd_review should name the Coverage finder: {g}"
    );
    // Findings must quote the offending line + concrete fix.
    assert!(
        lower.contains("quote the offending line"),
        "bdd_review findings must quote the offending line: {g}"
    );
    assert!(
        lower.contains("concrete fix"),
        "bdd_review findings must include a concrete fix: {g}"
    );
    // Falsifiability: per assertion, name the change that would fail it; flag
    // self-asserted state, constant comparisons, type-level facts.
    assert!(
        lower.contains("constant")
            && (lower.contains("self-asserted") || lower.contains("type-level")),
        "falsifiability finder should flag constant comparisons / self-asserted state / type-level facts: {g}"
    );
    // Coverage: AC<->scenario mapping + boundary pinning on both sides.
    assert!(
        lower.contains("boundary") && lower.contains("both sides"),
        "coverage finder should require both sides of every numeric/size limit tested: {g}"
    );
}

#[test]
fn red_step_requires_per_assertion_failure_evidence() {
    // Issue #1004: RED evidence is per new Then step and per new test assertion,
    // not per test target — every new assertion must individually be shown to
    // fail before implementation. A tautology cannot produce that evidence.
    let config = read_native_config();
    let g = guidance(&config, "red");
    let lower = g.to_lowercase();
    assert!(
        lower.contains("per new then step") || lower.contains("every new then step"),
        "red guidance should require failure evidence per new Then step: {g}"
    );
    assert!(
        lower.contains("per new test assertion") || lower.contains("every new assertion"),
        "red guidance should require failure evidence per new test assertion: {g}"
    );
    // "individually" alone is a common English word; pin the distinctive
    // phrase so an unrelated sentence cannot satisfy this.
    assert!(
        lower.contains("individually be shown to fail"),
        "red guidance should require each assertion to individually be shown to fail: {g}"
    );
}

#[test]
fn adr_records_review_restructure_decision() {
    // Issue #1004: an ADR records the move from six broad self-judging PR
    // reviewers to find -> verify -> single-post waves and the per-assertion
    // RED evidence gate, with the PR #1001 retrospective as context.
    let adr = read_repo_file(
        "docs/architecture-design-records/adr-0007-review-finder-waves-adversarial-verification.md",
    );
    let lower = adr.to_lowercase();
    for section in ["## Context", "## Decision", "## Consequences"] {
        assert!(
            adr.contains(section),
            "ADR should have a `{section}` section"
        );
    }
    assert!(
        adr.contains("#1001"),
        "ADR context should cite the PR #1001 retrospective"
    );
    assert!(
        lower.contains("tautolog"),
        "ADR context should record the tautological/vacuous assertion escape"
    );
    // "verify" && "find" alone are hollow ("find" matches "findings"
    // anywhere); pin the distinctive wave-structure phrasings instead.
    assert!(
        lower.contains("adversarial"),
        "ADR decision should describe the adversarial verify wave"
    );
    assert!(
        lower.contains("exactly one submitted") || lower.contains("single-post"),
        "ADR decision should record the single-post wave"
    );
    assert!(
        lower.contains("red evidence") || lower.contains("per-assertion"),
        "ADR decision should record the per-assertion RED evidence gate"
    );
}

#[test]
fn fix_reviews_addresses_all_valid_concerns() {
    // #953 AC3: fix_reviews must address EVERY valid concern regardless of
    // severity, not just high-priority ones.
    let config = read_native_config();
    let g = guidance(&config, "fix_reviews").to_lowercase();
    assert!(
        g.contains("every valid") && g.contains("regardless of severity"),
        "fix_reviews should require fixing every valid concern regardless of severity: {g}"
    );
    assert!(
        g.contains("decline"),
        "fix_reviews should allow a documented decline for invalid concerns: {g}"
    );
}

#[test]
fn version_bump_step_present_between_verify_and_commit() {
    // #950: a `version_bump` step must exist, positioned after `verify` and before
    // `commit` so the bump is committed and goes through the gate.
    let config = read_native_config();
    let vb = step(&config, "version_bump");
    assert_eq!(
        vb["phase"], "ci_cd",
        "version_bump should be a ci_cd-phase step"
    );
    assert!(
        step_index(&config, "verify") < step_index(&config, "version_bump"),
        "version_bump should come after verify"
    );
    assert!(
        step_index(&config, "version_bump") < step_index(&config, "commit"),
        "version_bump should come before commit"
    );

    let g = guidance(&config, "version_bump");
    let lower = g.to_lowercase();
    assert!(
        lower.contains("semver") && (lower.contains("patch") || lower.contains("minor")),
        "version_bump guidance should describe patch/minor semver bumps: {g}"
    );
    assert!(
        lower.contains("changed") && lower.contains("do not bump"),
        "version_bump guidance should bump only changed crates, not unchanged ones: {g}"
    );
    // Version docs kept in lockstep.
    assert!(
        g.contains("Current version:"),
        "version_bump guidance should update the README Current version line: {g}"
    );
    assert!(
        g.contains("repo_docs.feature"),
        "version_bump guidance should keep repo_docs.feature in lockstep: {g}"
    );
    assert!(
        g.contains("quecto-tui"),
        "version_bump guidance should cover the quecto-tui crate: {g}"
    );
    assert!(
        g.contains("Done when"),
        "version_bump guidance should carry a Done when exit criterion: {g}"
    );
}

#[test]
fn feature_js_bdd_review_is_strict() {
    // #953 mirrored into the executable feature.js. Anchor each assertion to the
    // specific BDD-review agent prompt block that runs, not a whole-file substring —
    // `"strict"` etc. can appear incidentally, so a whole-file `contains` could pass
    // even if the wording landed in the wrong place. (feature.js already carried a
    // doc-syncing Version phase before this change, so #950 adds nothing new to guard
    // here; the quecto template's new `version_bump` step is covered by
    // `version_bump_step_present_between_verify_and_commit`.)
    let js = read_repo_file("../.claude/workflows/feature.js");

    // The BDD-review agent prompt: from `phase('BDD Review')` up to `phase('GREEN')`.
    let bdd_block = js
        .split_once("phase('BDD Review')")
        .expect("feature.js should have a BDD Review phase")
        .1
        .split_once("phase('GREEN')")
        .expect("feature.js BDD Review phase should precede GREEN")
        .0
        .to_lowercase();
    assert!(
        bdd_block.contains("strict"),
        "feature.js BDD review prompt should instruct a strict reviewer: {bdd_block}"
    );
    assert!(
        bdd_block.contains("every valid") && bdd_block.contains("regardless of severity"),
        "feature.js BDD review prompt should require addressing every valid concern regardless of severity: {bdd_block}"
    );
    assert!(
        bdd_block.contains("one behaviour per scenario"),
        "feature.js BDD review prompt should carry the best-practice checklist token: {bdd_block}"
    );
    // Issue #1004 (PR #1005 review finding): the executable bdd_review runs the
    // same three narrow finders as the native config, with quoted-line findings.
    for token in ["gherkin discipline", "falsifiability", "coverage"] {
        assert!(
            bdd_block.contains(token),
            "feature.js BDD review should name the `{token}` finder: {bdd_block}"
        );
    }
    assert!(
        bdd_block.contains("quote the offending line") && bdd_block.contains("concrete fix"),
        "feature.js BDD review findings must quote the offending line with a concrete fix: {bdd_block}"
    );
    assert!(
        bdd_block.contains("both sides"),
        "feature.js BDD coverage finder must pin both sides of numeric/size limits: {bdd_block}"
    );
}

// NOTE: the former `reviewer_mechanic_deduplicated` guard was removed. It
// required the shared spawn/await/read mechanic to live in exactly one shared
// field (`shared_guidance`) and forbade the review steps from restating it — but
// `shared_guidance` is not a `WorkflowTemplate` field, so serde dropped it and
// the read-only instruction never reached a running agent. Correctness beats
// DRY here: each review step's `guidance` now carries the instruction inline,
// verified by `reviewer_spawns_are_read_only` (spec) and the runtime guard in
// `templates.rs`.

#[test]
fn reviewer_spawns_are_read_only() {
    // #957: both the `bdd_review` and PR `reviewers` spawns must launch reviewers
    // read-only — `write` and `edit` removed from the child registry so the model
    // never sees them (defense-in-depth against reviewers writing stray files).
    // The instruction MUST live in each review step's own `guidance` — a field
    // the runtime deserializes — NOT a `shared_guidance` field serde silently
    // drops (that phantom field made this instruction inert at runtime). See the
    // matching runtime guard in `templates.rs`.
    let config = read_native_config();
    for key in ["bdd_review", "reviewers"] {
        let g = guidance(&config, key);
        let lower = g.to_lowercase();
        assert!(
            lower.contains("read_only") || lower.contains("read-only"),
            "`{key}` guidance should launch reviewers read-only: {g}"
        );
        // The disabled set must be EXACTLY write + edit. Anchor to the canonical
        // quoted list (rather than free-floating `write`/`edit` substrings, which
        // also match "written"/"credit").
        assert!(
            g.contains(r#"["write", "edit"]"#) || g.contains(r#"["write","edit"]"#),
            "`{key}` guidance should disable exactly [\"write\", \"edit\"]: {g}"
        );
        // Reviewers keep their non-mutating toolset — name the retained tools so
        // criterion 3 (bash/read/grep/find/agent_cmd intact) is verified directly.
        for keep in ["bash", "read", "grep", "find", "agent_cmd"] {
            assert!(
                g.contains(keep),
                "`{key}` reviewers must retain `{keep}`; guidance should name it: {g}"
            );
        }
    }
}
