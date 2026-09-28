use super::parent_playbook::load;

const NAME: &str = "PARENT_PLAYBOOK.md";

#[test]
fn falls_back_to_packaged_playbook_only_when_absent() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        load(directory.path()).unwrap(),
        include_str!("../../../PARENT_PLAYBOOK.md")
    );
    std::fs::write(directory.path().join(NAME), "project policy\n").unwrap();
    assert_eq!(load(directory.path()).unwrap(), "project policy\n");
}

#[test]
fn rejects_invalid_and_unreadable_override() {
    let directory = tempfile::tempdir().unwrap();
    let override_path = directory.path().join(NAME);
    std::fs::write(&override_path, [0xff]).unwrap();
    assert!(load(directory.path()).unwrap_err().contains("UTF-8"));
    std::fs::remove_file(&override_path).unwrap();
    std::fs::create_dir(&override_path).unwrap();
    assert!(
        load(directory.path())
            .unwrap_err()
            .contains("failed to read")
    );
}

#[test]
fn never_searches_ancestors() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join(NAME), "ancestor").unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    assert_eq!(
        load(&child).unwrap(),
        include_str!("../../../PARENT_PLAYBOOK.md")
    );
}

/// These contract tests describe the bundled default, not project overrides.
fn bundled_policy() -> &'static str {
    include_str!("../../../PARENT_PLAYBOOK.md")
}

fn policy_has_all(context: &str, policy: &str, terms: &[&str]) {
    for term in terms {
        assert!(
            policy.contains(term),
            "{context}: missing {term:?} from bundled parent playbook"
        );
    }
}

#[test]
fn heavy_verification_is_ci_evidence_not_local_work() {
    let policy = bundled_policy();
    policy_has_all(
        "CI owns expensive verification",
        policy,
        &[
            "mutation",
            "sharded",
            "diff-scoped",
            "CI",
            "full",
            "BDD",
            "integration",
            "merge-requested",
        ],
    );
    policy_has_all(
        "local delivery and fixes use focused checks",
        policy,
        &[
            "Locally",
            "delivery",
            "fix",
            "targeted",
            "lint",
            "architecture",
            "docs",
            "contracts",
        ],
    );
    policy_has_all(
        "review and fixes consume CI mutant evidence",
        policy,
        &[
            "review",
            "MISSED",
            "TIMEOUT",
            "regression",
            "mutation tools",
        ],
    );
    assert!(
        policy.contains("not run mutation")
            || policy.contains("don't run mutation")
            || policy.contains("never run mutation"),
        "reviewers and fix swarms must not launch local mutation tools"
    );
    assert!(
        policy.contains("full suites run in CI")
            || policy.contains("full test suites run in CI")
            || policy.contains("full suites only in CI"),
        "full suites must run in CI instead of on the local host"
    );
}

#[test]
fn unsafe_probes_have_hard_deadlines_non_core_termination_and_cleanup() {
    let policy = bundled_policy();
    let bounded_probes = policy
        .split_once("### Bounded verification and unsafe probes")
        .expect("bounded verification section")
        .1
        .split_once("\n### ")
        .expect("bounded verification has next section")
        .0;
    let termination_rule = bounded_probes
        .split_once("Give every unsafe probe")
        .expect("affirmative unsafe-probe rule")
        .1
        .split_once('.')
        .expect("unsafe-probe rule has a sentence")
        .0;
    policy_has_all(
        "bounded probe deadline",
        termination_rule,
        &["hard timeout"],
    );
    assert_eq!(
        termination_rule
            .split_once(';')
            .expect("unsafe-probe rule has a termination clause")
            .1
            .trim(),
        "terminate via SIGKILL or `_exit`, never a core-dumping signal",
        "unsafe probes must end safely, not by a core-dumping signal"
    );
    policy_has_all(
        "unsafe probes",
        policy,
        &[
            "probe",
            "loop",
            "cycle",
            "retries",
            "lock",
            "hard timeout",
            "SIGKILL",
            "_exit",
            "core",
            "scratch",
            "temp",
            "cleanup",
        ],
    );
}

#[test]
fn planning_sizes_every_pr_independently_of_optional_spikes() {
    let policy = bundled_policy();
    let planning = policy
        .split_once("### Planning and PR sizing")
        .expect("unconditional planning section")
        .1
        .split_once("\n### ")
        .expect("planning has next section")
        .0;
    policy_has_all(
        "each planned PR remains independently deliverable and mergeable",
        planning,
        &[
            "Each planned PR",
            "self-contained",
            "independently testable acceptance behavior",
            "manageable red-test phase",
            "master buildable and green after each merge",
            "duplicate ownership",
        ],
    );
}

#[test]
fn optional_disposable_spike_right_sizes_independently_mergeable_prs() {
    let policy = bundled_policy();
    let spike = policy
        .split_once("### Optional pre-planning spike")
        .expect("optional pre-planning spike section")
        .1
        .split_once("\n### ")
        .expect("spike has next section")
        .0;
    policy_has_all(
        "spike informs PR sizing without entering delivery",
        spike,
        &[
            "optional",
            "disposable",
            "small",
            "complete",
            "independently",
            "PR",
            "red-test",
            "master",
            "green",
            "buildable",
            "duplicate ownership",
        ],
    );
    policy_has_all(
        "spike code is not reused",
        spike,
        &["never merge", "spike code"],
    );
    assert!(
        spike.contains("not a substitute") || spike.contains("does not replace"),
        "spike must not replace a separate red-test phase"
    );
}

#[test]
fn each_pr_requires_independent_red_tests_before_implementation() {
    let policy = bundled_policy();
    policy_has_all(
        "red-test swarm boundary and evidence",
        policy,
        &[
            "fresh",
            "red-test swarm",
            "each PR",
            "before implementation",
            "unit",
            "BDD",
            "acceptance",
            "non-testable",
            "review evidence",
            "delivery branch",
            "own commit",
            "fails",
            "compile error",
            "missing fixture",
            "pushed",
            "targeted",
            "CI",
        ],
    );
    policy_has_all(
        "parent decisions for exceptions",
        policy,
        &[
            "already passes",
            "baseline",
            "escalate",
            "weaken",
            "delete",
            "explicit parent",
            "recorded on the PR",
            "user's explicit instruction",
        ],
    );
    assert!(
        policy.contains("red-test swarm doesn't implement")
            || policy.contains("red-test swarm does not implement"),
        "red-test swarm must not implement acceptance behavior"
    );
    let planning = policy.find("### Optional pre-planning spike").unwrap();
    let red = policy
        .find("### Red-test")
        .expect("dedicated red-test phase");
    let review = policy.find("### PR-first review and remediation").unwrap();
    assert!(
        planning < red && red < review,
        "red-test phase belongs between spike/planning and PR-first review"
    );
    policy_has_all(
        "red-to-delivery handoff",
        &policy[red..review],
        &["planning", "delivery", "spike", "separate"],
    );
}
