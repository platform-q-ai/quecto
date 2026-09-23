//! #2098: executable boundaries for the first four architecture scenarios.
use std::{fs, path::Path};

const APPLICATION_IO: &[&str] = &[
    "std::fs::",
    "tokio::fs::",
    "std::env::",
    "dirs::",
    ".exists(",
];

fn application_line_allowed(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("//") || APPLICATION_IO.iter().all(|pattern| !line.contains(pattern))
}

fn production_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .take_while(|line| line.trim() != "#[cfg(test)]")
}

fn visit_rs(dir: &Path, check: &mut impl FnMut(&Path)) {
    for entry in fs::read_dir(dir).expect("architecture source directory") {
        let path = entry.expect("architecture source entry").path();
        if path.is_dir() {
            visit_rs(&path, check);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            check(&path);
        }
    }
}

fn ci_workspace_lint(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("- run: cargo clippy ")
        && line.split_whitespace().any(|word| word == "--workspace")
}

fn ci_mock_suite(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("- run: ")
        && line.contains("scripts/run-bdd-shards.sh")
        && line.contains("--suite mock-llm-bdd")
        && line.contains("--tag mock-llm")
}

fn pre_push_paid_lane_allowed(source: &str) -> bool {
    ["OPENAI_API_KEY", "REAL_LLM_STATE"]
        .iter()
        .all(|probe| !source.contains(probe))
}

#[test]
fn application_traversal_checks_production_tests_named_module() {
    let dir = std::env::temp_dir().join(format!("quecto-2098-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("fixture directory");
    let fixture = dir.join("production_tests.rs");
    fs::write(
        &fixture,
        "std::fs::read(path)\n#[cfg(test)]\nstd::fs::read(path)\n",
    )
    .expect("fixture source");
    let mut violations = Vec::new();
    visit_rs(&dir, &mut |path| {
        let source = fs::read_to_string(path).expect("fixture source");
        for line in production_lines(&source) {
            if !application_line_allowed(line) {
                violations.push(line.to_owned());
            }
        }
    });
    fs::remove_dir_all(&dir).expect("remove fixture directory");
    assert_eq!(violations, ["std::fs::read(path)"]);
}

#[test]
fn application_runtime_io_is_absent_from_production() {
    let mut count = 0;
    visit_rs(Path::new("src/application"), &mut |path| {
        count += 1;
        let source = fs::read_to_string(path).expect("application source");
        for (index, line) in production_lines(&source).enumerate() {
            assert!(
                application_line_allowed(line),
                "{}:{} contains runtime I/O: {line}",
                path.display(),
                index + 1
            );
        }
    });
    assert!(count > 10, "application inventory must not be empty");
}

#[test]
fn authoritative_ci_runs_workspace_lint_and_mocked_e2e() {
    let ci = fs::read_to_string("../.github/workflows/ci.yml").expect("authoritative CI workflow");
    assert!(
        ci.lines().any(ci_workspace_lint),
        "CI must run workspace clippy"
    );
    assert!(
        ci.lines().any(ci_mock_suite),
        "CI must run mocked e2e shards"
    );
}

#[test]
fn pre_push_does_not_probe_provider_credentials_to_start_paid_suite() {
    let hook = fs::read_to_string("../scripts/pre-push.sh").expect("pre-push hook");
    assert!(
        pre_push_paid_lane_allowed(&hook),
        "provider credentials must not auto-enable paid tests"
    );
}

#[test]
fn core_guards_reject_deliberate_negative_fixtures() {
    assert!(application_line_allowed(
        "// std::fs::read is forbidden in production"
    ));
    for fixture in [
        "std::fs::read(path)",
        "tokio::fs::read(path)",
        "std::env::var(key)",
        "dirs::home_dir()",
        "path.exists()",
    ] {
        assert!(
            !application_line_allowed(fixture),
            "accepted I/O fixture {fixture}"
        );
    }
    assert_eq!(
        production_lines("safe\n#[cfg(test)]\nstd::fs::read(path)").collect::<Vec<_>>(),
        vec!["safe"]
    );
    assert!(ci_workspace_lint(
        "- run: cargo clippy --workspace --all-targets"
    ));
    for fixture in [
        "# - run: cargo clippy --workspace",
        "- run: cargo clippy -p harness",
    ] {
        assert!(
            !ci_workspace_lint(fixture),
            "accepted lint fixture {fixture}"
        );
    }
    assert!(ci_mock_suite(
        "- run: bash scripts/run-bdd-shards.sh --suite mock-llm-bdd --tag mock-llm"
    ));
    for fixture in [
        "# scripts/run-bdd-shards.sh --suite mock-llm-bdd --tag mock-llm",
        "- run: bash scripts/run-bdd-shards.sh --suite real-llm-bdd --tag real-llm",
    ] {
        assert!(
            !ci_mock_suite(fixture),
            "accepted mock suite fixture {fixture}"
        );
    }
    assert!(pre_push_paid_lane_allowed("QUECTO_RUN_REAL_LLM=1"));
    assert!(!pre_push_paid_lane_allowed(
        "if test -n \"$OPENAI_API_KEY\"; then REAL_LLM_STATE=run; fi"
    ));
}
