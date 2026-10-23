//! Canonical external cfg(test) ownership must survive both production scans.
//! Use the real board tree and existing AST ownership, not a filename exception
//! or a fixture helper that implements the acceptance being requested.
use super::{
    assert_interface_dependencies_allowlisted, collect_rs_files, dependency_scan,
    find_composition_delegation_allowed,
};
use std::path::Path;

const BOARD: &str = "src/interface/cli/controllers/board/mod.rs";
const TESTS: &str = "src/interface/cli/controllers/board/tests";
const TEST_MODULE: &str = "src/interface/cli/controllers/board/tests/mod.rs";
const FORBIDDEN: &str =
    "fn build() { crate::composition::bootstrap::project_board::cli_board_recording(); }";

#[test]
fn find_scan_excludes_canonical_external_cfg_test_board_module() {
    assert!(dependency_scan::production_file(BOARD));
    assert!(dependency_scan::test_only_file(TEST_MODULE));
    let mut files = Vec::new();
    collect_rs_files(Path::new(TESTS), &mut files);
    // As in find_inward_boundaries_are_typed_and_effect_free, every source
    // returned by the production scan must pass the real delegation policy.
    for file in files {
        let (path, source) = file.split_once(":\n").expect("collected source");
        assert!(
            find_composition_delegation_allowed(path, source),
            "test-only module leaked into find production classification: {path}"
        );
    }
}

#[test]
fn web_fetch_scan_excludes_canonical_external_cfg_test_board_module() {
    assert!(dependency_scan::production_file(BOARD));
    assert!(dependency_scan::test_only_file(TEST_MODULE));
    let mut files = Vec::new();
    collect_rs_files(Path::new(TESTS), &mut files);
    // The actual web-fetch interface policy, with no test-local exemption.
    for file in files {
        let (path, source) = file.split_once(":\n").expect("collected source");
        assert_interface_dependencies_allowlisted(path, source);
    }
}

#[test]
fn find_rejects_direct_forbidden_production_composition() {
    assert!(dependency_scan::production_file(BOARD));
    assert_eq!(find_composition_delegation_allowed(BOARD, FORBIDDEN), false);
}

#[test]
fn web_fetch_rejects_direct_forbidden_production_composition() {
    assert!(dependency_scan::production_file(BOARD));
    let rejected = std::panic::catch_unwind(|| {
        assert_interface_dependencies_allowlisted(BOARD, FORBIDDEN);
    });
    assert!(rejected.is_err(), "production composition must be rejected");
}

#[test]
fn delegation_and_interface_policies_remain_distinct() {
    // Delegation intentionally permits this web-fetch caller; the interface
    // boundary remains narrower. Ownership repair must not merge the policies.
    let registry = "src/interface/cli/controllers/agent/agent_tool_registry.rs";
    let web_fetch = "fn build() { crate::composition::web_fetch::build(); }";
    assert!(dependency_scan::production_file(registry));
    assert!(find_composition_delegation_allowed(registry, web_fetch));
    assert!(
        std::panic::catch_unwind(|| {
            assert_interface_dependencies_allowlisted(registry, web_fetch);
        })
        .is_err()
    );
}
