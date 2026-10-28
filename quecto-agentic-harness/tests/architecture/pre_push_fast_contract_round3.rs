//! Regressions for inert job text and inherited workflow shell semantics.
use super::{ci_contract_valid, pre_push_synthetic_ci};

#[test]
fn pre_push_ci_round3_rejects_required_jobs_inside_inert_run_name() {
    let source = pre_push_synthetic_ci(true);
    let (trigger, required_job_text) = source
        .split_once("jobs:\n")
        .expect("synthetic workflow has a jobs mapping");
    assert!(required_job_text.lines().all(|line| line.starts_with("  ")));
    // The two-space job-looking lines belong to the run-name scalar, not jobs.
    // Only the column-zero jobs key ends the scalar and introduces real jobs.
    let invalid = format!(
        "{trigger}run-name: |\n{required_job_text}jobs:\n  echo-only:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo harmless\n"
    );
    assert_eq!(invalid.lines().filter(|line| *line == "jobs:").count(), 1);
    assert!(invalid.contains("run-name: |\n  static-quality:"));
    assert!(invalid.contains("jobs:\n  echo-only:"));
    assert!(
        !ci_contract_valid(&invalid),
        "inert run-name required-job decoys accepted despite real jobs containing only echo-only"
    );
}

#[test]
fn pre_push_ci_round3_rejects_inherited_shell_that_skips_required_commands() {
    let source =
        pre_push_synthetic_ci(true).replace("    steps:", "    runs-on: ubuntu-latest\n    steps:");
    assert!(
        ci_contract_valid(&source),
        "ordinary executable jobs are valid"
    );
    assert_eq!(source.lines().filter(|line| *line == "jobs:").count(), 1);
    // Merely inspect this YAML; never execute the shell template. bash exits
    // successfully before reading the generated required-step script at {0}.
    let invalid = source.replacen(
        "jobs:\n",
        "defaults:\n  run:\n    shell: bash -c 'exit 0' {0}\njobs:\n",
        1,
    );
    assert_eq!(
        invalid
            .lines()
            .filter(|line| line.trim_start().starts_with("shell:"))
            .collect::<Vec<_>>(),
        ["    shell: bash -c 'exit 0' {0}"]
    );
    assert!(
        !ci_contract_valid(&invalid),
        "workflow defaults.run.shell bypass accepted with no required-step shell overrides"
    );
}
