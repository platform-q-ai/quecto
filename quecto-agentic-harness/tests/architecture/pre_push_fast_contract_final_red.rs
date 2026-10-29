//! FINAL false-positive regressions: safe metadata must preserve the CI contract.
//! These use the existing bool API; diagnostic reasons belong to the successor.
use super::{COMPILED_NAME, ci_contract_valid, pre_push_synthetic_ci};

const PINNED_STEPS: [&str; 4] = [
    "Strict workspace Clippy",
    "Strict standalone quecto-tui Clippy",
    "Strict standalone quecto-agentic-harness Clippy",
    COMPILED_NAME,
];

fn source_with_root_field(source: &str, field: &str) -> String {
    assert!(
        ci_contract_valid(source),
        "unmodified synthetic CI is valid"
    );
    assert_eq!(source.matches("jobs:\n").count(), 1);
    source.replacen("jobs:\n", &format!("{field}\njobs:\n"), 1)
}

#[test]
fn pre_push_ci_final_safe_root_permissions_are_valid() {
    for name_first in [true, false] {
        let source = pre_push_synthetic_ci(name_first);
        let safe = source_with_root_field(&source, "permissions:\n  contents: read");
        assert!(
            ci_contract_valid(&safe),
            "safe root permissions contents: read must remain valid; name_first={name_first}"
        );
    }
}

#[test]
fn pre_push_ci_final_benign_root_environment_is_valid() {
    for name_first in [true, false] {
        let source = pre_push_synthetic_ci(name_first);
        // A non-execution metadata variable: no PATH, compiler, shell or flags override.
        let safe = source_with_root_field(&source, "env:\n  PROJECT_DISPLAY_LABEL: quecto");
        assert!(
            ci_contract_valid(&safe),
            "benign root environment must remain valid; name_first={name_first}"
        );
    }
}

#[test]
fn pre_push_ci_final_quoted_pinned_step_names_are_valid() {
    let mut rejected = Vec::new();
    for name_first in [true, false] {
        let source = pre_push_synthetic_ci(name_first);
        assert!(ci_contract_valid(&source), "unquoted control is valid");
        for name in PINNED_STEPS {
            let marker = format!("name: {name}\n");
            assert_eq!(source.matches(&marker).count(), 1);
            for quote in ['\'', '"'] {
                // Replace only the display scalar, never commands or admission.
                let quoted = source.replacen(&marker, &format!("name: {quote}{name}{quote}\n"), 1);
                let valid = ci_contract_valid(&quoted);
                println!(
                    "quoted pinned step: name_first={name_first}; quote={quote}; name={name}; valid={valid}"
                );
                match valid {
                    true => {}
                    false => {
                        rejected.push(format!("name_first={name_first}; {quote}{name}{quote}"))
                    }
                }
            }
        }
    }
    assert!(
        rejected.is_empty(),
        "quoted scalar pinned step names must remain valid; rejected={rejected:?}"
    );
}

#[test]
fn pre_push_ci_final_execution_affecting_root_environment_is_invalid() {
    let source = pre_push_synthetic_ci(true);
    for field in [
        "env:\n  RUSTFLAGS: --cap-lints allow",
        "env:\n  CARGO_ENCODED_RUSTFLAGS: --cap-lints=allow",
        "env:\n  RUSTC: /tmp/alternate-rustc",
        "env:\n  RUSTC_WRAPPER: /tmp/wrapper",
        "env:\n  PATH: /tmp/tools",
        "env:\n  BASH_ENV: /tmp/shell-init",
    ] {
        let invalid = source_with_root_field(&source, field);
        assert!(
            !ci_contract_valid(&invalid),
            "execution-affecting root environment accepted: {field}"
        );
        println!("rejected execution-affecting root environment: {field}");
    }
}

#[test]
fn pre_push_ci_final_malformed_and_invalid_inputs_are_rejected() {
    let source = pre_push_synthetic_ci(true);
    assert!(ci_contract_valid(&source), "valid control");
    for (label, invalid) in [
        ("empty workflow", String::new()),
        ("no jobs mapping", source.replace("jobs:\n", "")),
        ("duplicate jobs mapping", format!("{source}jobs:\n")),
        (
            "unterminated quoted pinned name",
            source.replacen(
                "name: Strict workspace Clippy",
                "name: 'Strict workspace Clippy",
                1,
            ),
        ),
        (
            "quoted wrong identity",
            source.replacen(
                "name: Strict workspace Clippy",
                "name: 'Unrelated Clippy'",
                1,
            ),
        ),
        (
            "missing required command",
            source.replacen(super::COMPILED_COMMAND, "echo omitted", 1),
        ),
        (
            "invalid admission",
            source.replace(super::ADMISSION, "always()"),
        ),
    ] {
        assert!(
            !ci_contract_valid(&invalid),
            "invalid input accepted: {label}"
        );
        println!("rejected FINAL invalid input: {label}");
    }
}

fn table_cells(line: &str) -> Vec<&str> {
    match line
        .trim()
        .strip_prefix('|')
        .and_then(|row| row.strip_suffix('|'))
    {
        Some(row) => row.split('|').map(str::trim).collect(),
        None => Vec::new(),
    }
}

#[test]
fn pre_push_ci_final_contributions_table_matches_pinned_job_step_columns() {
    let markdown = include_str!("../../../CONTRIBUTIONS.md");
    let lines: Vec<_> = markdown.lines().collect();
    let headers: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            table_cells(line) == ["Relocated check", "Job ID (check name)", "Step name"]
        })
        .map(|(index, _)| index)
        .collect();
    let [header] = headers.as_slice() else {
        panic!(
            "expected one relocated-check table, found {}",
            headers.len()
        );
    };
    // Read the actual rendered table's cells, after its header and delimiter row.
    let actual: Vec<_> = lines[header + 2..]
        .iter()
        .take_while(|line| line.trim().starts_with('|'))
        .map(|line| {
            let cells = table_cells(line);
            assert_eq!(
                cells.len(),
                3,
                "relocated-check table must have three columns"
            );
            (cells[0], cells[1], cells[2])
        })
        .collect();
    let static_job = "`static-quality` (Static Quality)";
    let workspace_job = "`workspace-tests` (Workspace Tests)";
    let expected = vec![
        (
            "Strict workspace Clippy (all targets, test-support, complexity lints)",
            static_job,
            PINNED_STEPS[0],
        ),
        (PINNED_STEPS[1], static_job, PINNED_STEPS[1]),
        (PINNED_STEPS[2], static_job, PINNED_STEPS[2]),
        ("Architecture tests", workspace_job, COMPILED_NAME),
        ("Port contract tests", workspace_job, COMPILED_NAME),
        ("Docs tests", workspace_job, COMPILED_NAME),
    ];
    assert_eq!(
        actual, expected,
        "documented check/job/step mapping must equal pinned identities"
    );
    println!(
        "actual CONTRIBUTIONS.md table matches all {} pinned check/job/step rows",
        actual.len()
    );
}
