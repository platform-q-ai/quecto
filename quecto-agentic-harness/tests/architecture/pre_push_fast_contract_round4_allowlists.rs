//! Ownership and unsupported-shape coverage for the bounded round4 allowlists.
use super::{COMPILED_NAME, ci_contract_valid, pre_push_synthetic_ci};

#[test]
fn pre_push_ci_round4_branch_allowlist_rejects_unsupported_shapes() {
    let source = pre_push_synthetic_ci(true);
    assert!(ci_contract_valid(&source));
    let marker = "    types: [labeled]";
    assert_eq!(source.matches(marker).count(), 1);
    for field in [
        "branches: []",
        "branches: master",
        "branches: ['master']",
        "branches: [master, release]",
        "branches: [master] # comment",
        "branches:\n      - master",
        "branches: [master]\n    branches: [master]",
        "\"branches\": [master]",
        "branches : [master]",
        "branches-ignore: [release]",
    ] {
        let invalid = source.replace(marker, &format!("    {field}\n{marker}"));
        assert!(
            !ci_contract_valid(&invalid),
            "unsupported pull_request branch syntax accepted: {field}"
        );
        println!("rejected additional branch shape: {field}");
    }
}

#[test]
fn pre_push_ci_round4_required_steps_allow_no_environment_shapes() {
    let source = pre_push_synthetic_ci(true);
    assert!(ci_contract_valid(&source));
    for name in [
        "Strict workspace Clippy",
        "Strict standalone quecto-tui Clippy",
        "Strict standalone quecto-agentic-harness Clippy",
        COMPILED_NAME,
    ] {
        let marker = format!("      - name: {name}\n");
        assert_eq!(source.matches(&marker).count(), 1);
        for field in [
            "env:\n          RUSTFLAGS: --cap-lints allow",
            "env:\n          GH_TOKEN: ${{ github.token }}",
            "env:\n          GH_REPO: ${{ github.repository }}",
            "env:\n          ARBITRARY: value",
            "env: {}",
            "env: {RUSTFLAGS: --cap-lints allow}",
            "env: ${{ matrix.env }}",
            "env:",
            "\"env\": {}",
            "env : {}",
        ] {
            let invalid = source.replace(&marker, &format!("{marker}        {field}\n"));
            assert!(
                !ci_contract_valid(&invalid),
                "unsupported required-step env accepted: {name}: {field}"
            );
            println!("rejected additional required-step env: {name}: {field}");
        }
    }
}

#[test]
fn pre_push_ci_round4_root_and_required_job_environment_remain_unsupported() {
    let source = pre_push_synthetic_ci(true);
    assert!(ci_contract_valid(&source));
    for (marker, indent) in [
        ("jobs:", ""),
        ("    name: Static Quality", "    "),
        ("    name: Workspace Tests", "    "),
    ] {
        assert_eq!(source.matches(marker).count(), 1);
        for field in [
            "env: {}".to_owned(),
            format!("env:\n{indent}  RUSTFLAGS: --cap-lints allow"),
        ] {
            let invalid = source.replace(marker, &format!("{indent}{field}\n{marker}"));
            assert!(
                !ci_contract_valid(&invalid),
                "unsupported owning-scope env accepted: {marker}: {field}"
            );
            println!("rejected additional owning-scope env: {marker}: {field}");
        }
    }
}

#[test]
fn pre_push_ci_round4_unrelated_steps_and_jobs_keep_their_own_environment() {
    let source = pre_push_synthetic_ci(true);
    assert!(ci_contract_valid(&source));
    let unrelated_step = "      - name: Require all PR review threads resolved\n        env:\n          GH_TOKEN: ${{ github.token }}\n          GH_REPO: ${{ github.repository }}\n        run: echo unrelated\n";
    // Even within a required job, environment belongs to its own step only.
    let adjacent = source.replace(
        "  workspace-tests:",
        &format!("{unrelated_step}  workspace-tests:"),
    );
    assert!(ci_contract_valid(&adjacent));
    let unrelated_job = format!(
        "{source}  review-threads:\n    name: Review threads\n    env:\n      GH_REPO: ${{{{ github.repository }}}}\n    steps:\n{unrelated_step}"
    );
    assert!(ci_contract_valid(&unrelated_job));
    println!("accepted additional env ownership positives: adjacent step and unrelated job");
}
