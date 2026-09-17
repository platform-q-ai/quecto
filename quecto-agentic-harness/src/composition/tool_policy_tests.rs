//! The composed hook goes through the configuration writer: it patches
//! only `tools.policy.entries` and reports an unreadable file instead of
//! overwriting it.

use super::*;
use crate::application::configuration::dto::ConfigSources;
use crate::domain::tool::{ToolPolicyApplyMode, ToolPolicyReconciliation};

fn empty_reconciliation() -> ToolPolicyReconciliation {
    ToolPolicyReconciliation {
        mode: ToolPolicyApplyMode::ImmediateIfIdle,
        results: vec![],
        correlation_id: None,
    }
}

#[test]
fn the_composed_hook_leaves_a_file_alone_when_nothing_was_applied() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, "{\"custom\": 1}").unwrap();
    let persist = build_tool_policy_persistence(
        tmp.path(),
        &ConfigSources {
            base: config_path.clone(),
            explicit: true,
            overlay: None,
            legacy_local: None,
        },
    );
    persist(&empty_reconciliation()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        "{\"custom\": 1}",
        "no whole-struct rewrite: the file is byte-identical"
    );
}
