//! Reason-bearing contract: failures identify the rejected boundary, not just false.
use super::{ADMISSION, ci_contract_validation, pre_push_synthetic_ci};

#[test]
fn pre_push_ci_diagnostics_identify_invalid_boundaries() {
    let source = pre_push_synthetic_ci(true);
    for (label, invalid, expected) in [
        ("empty", String::new(), "jobs"),
        (
            "duplicate jobs",
            source.replace("jobs:", "jobs:\njobs:"),
            "jobs",
        ),
        (
            "root env",
            source.replace("jobs:", "env:\n  RUSTFLAGS: unsafe\njobs:"),
            "RUSTFLAGS",
        ),
        (
            "root defaults",
            source.replace("jobs:", "defaults:\n  run:\n    shell: bash\njobs:"),
            "defaults",
        ),
        (
            "admission",
            source.replace(ADMISSION, "always()"),
            "static-quality",
        ),
        (
            "wrong name",
            source.replace("name: Strict workspace Clippy", "name: 'Other'"),
            "Strict workspace Clippy",
        ),
        (
            "malformed name",
            source.replace(
                "name: Strict workspace Clippy",
                "name: 'Strict workspace Clippy",
            ),
            "Strict workspace Clippy",
        ),
        (
            "step env",
            source.replace(
                "        run:",
                "        env:\n          RUSTFLAGS: unsafe\n        run:",
            ),
            "Strict workspace Clippy",
        ),
    ] {
        let error = ci_contract_validation(&invalid).expect_err(label);
        assert!(
            error.contains(expected),
            "{label}: expected {expected:?} in diagnostic, got {error:?}"
        );
        println!("diagnostic {label}: {error}");
    }
}

#[test]
fn pre_push_ci_diagnostics_accept_safe_root_metadata_and_names() {
    let source = pre_push_synthetic_ci(true)
        .replace(
            "jobs:",
            "permissions:\n  contents: read\nenv:\n  PROJECT_DISPLAY_LABEL: quecto\njobs:",
        )
        .replace(
            "name: Strict workspace Clippy",
            "name: 'Strict workspace Clippy'",
        );
    assert_eq!(ci_contract_validation(&source), Ok(()));
}
