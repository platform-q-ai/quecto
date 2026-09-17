//! The composed hook persists through the config file it was built over
//! and reports an unreadable one instead of overwriting it.

use super::*;
use crate::domain::tool::{ToolPolicyApplyMode, ToolPolicyReconciliation};

fn empty_reconciliation() -> ToolPolicyReconciliation {
    ToolPolicyReconciliation {
        mode: ToolPolicyApplyMode::ImmediateIfIdle,
        results: vec![],
        correlation_id: None,
    }
}

#[test]
fn the_composed_hook_rewrites_the_config_file_it_was_built_over() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, "{}").unwrap();
    let persist = build_tool_policy_persistence(&config_path);
    persist(&empty_reconciliation()).unwrap();
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert!(
        written.get("tools").is_some(),
        "the hook serialises the full config back into its own file: {written}"
    );
}

#[test]
fn the_composed_hook_reports_a_malformed_config_file_and_leaves_it_alone() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, "{ not json").unwrap();
    let persist = build_tool_policy_persistence(&config_path);
    let error = persist(&empty_reconciliation()).unwrap_err();
    assert!(error.contains("tool policy persistence"), "{error}");
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), "{ not json");
}
