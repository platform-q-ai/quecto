//! Behavioral RED seam for #2358. This deliberately does not enforce layout yet.
//! Counts will follow filenames: include mod.rs and cfg(test) support names,
//! exclude names ending in _tests.rs (application/swarm baseline: 23).

use std::path::Path;

#[derive(Debug)]
struct FlatBudget<'a> {
    path: &'a str,
    maximum: usize,
}

#[derive(Debug)]
struct MigratedCapability<'a> {
    path: &'a str,
    allowed_roles: &'a [&'a str],
}

#[derive(Debug, PartialEq, Eq)]
enum Violation {
    FlatBudgetExceeded {
        path: String,
        actual: usize,
        maximum: usize,
    },
    UnexpectedMigratedEntry {
        path: String,
    },
}

// Temporary test-only no-op. GREEN delivery replaces this seam with enforcement;
// there is no production hook, scanner or checked-in source-tree policy yet.
fn validate(
    root: &Path,
    budgets: &[FlatBudget<'_>],
    migrated: &[MigratedCapability<'_>],
) -> Vec<Violation> {
    // Keep the provisional policy types compile/Clippy-clean without enforcement.
    let _ = root;
    for budget in budgets {
        let _ = (budget.path, budget.maximum);
    }
    for capability in migrated {
        let _ = (capability.path, capability.allowed_roles);
    }
    Vec::new()
}

#[test]
fn adding_flat_file_exceeds_ratchet() {
    let tree = tempfile::tempdir().expect("create isolated layout fixture");
    let domain = tree.path().join("domain");
    std::fs::create_dir(&domain).expect("create domain layer");
    std::fs::write(domain.join("mod.rs"), "").expect("write counted module root");
    let budgets = [FlatBudget {
        path: "domain",
        maximum: 1,
    }];
    assert_eq!(validate(tree.path(), &budgets, &[]), Vec::new());

    std::fs::write(domain.join("extra.rs"), "").expect("add flat production file");
    assert_eq!(
        validate(tree.path(), &budgets, &[]),
        vec![Violation::FlatBudgetExceeded {
            path: "domain".into(),
            actual: 2,
            maximum: 1,
        }],
        "adding an immediate flat .rs file must exceed the unchanged budget"
    );
}

#[test]
fn stray_file_in_migrated_capability_fails_shape() {
    let tree = tempfile::tempdir().expect("create isolated layout fixture");
    let capability = tree.path().join("application/sessions");
    std::fs::create_dir_all(&capability).expect("create migrated capability");
    std::fs::write(capability.join("mod.rs"), "").expect("write permitted module root");
    for role in ["use_cases", "ports", "dto"] {
        std::fs::create_dir(capability.join(role)).expect("create permitted role");
    }
    let migrated = [MigratedCapability {
        path: "application/sessions",
        allowed_roles: &["use_cases", "ports", "dto"],
    }];
    assert_eq!(validate(tree.path(), &[], &migrated), Vec::new());

    std::fs::write(capability.join("stray.rs"), "").expect("add stray direct file");
    assert_eq!(
        validate(tree.path(), &[], &migrated),
        vec![Violation::UnexpectedMigratedEntry {
            path: "application/sessions/stray.rs".into(),
        }],
        "a migrated root permits only explicit role directories and mod.rs"
    );
}
