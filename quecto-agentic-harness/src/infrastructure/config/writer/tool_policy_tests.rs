//! The hook patches only `tools.policy.entries` of the config file it was
//! built over and reports an unreadable one instead of overwriting it.

use super::*;
use crate::domain::tool::{
    ToolPolicyApplyMode, ToolPolicyMutationResult, ToolPolicyMutationStatus,
    ToolPolicyReconciliation,
};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

fn empty_reconciliation() -> ToolPolicyReconciliation {
    ToolPolicyReconciliation {
        mode: ToolPolicyApplyMode::ImmediateIfIdle,
        results: vec![],
        correlation_id: None,
    }
}

fn applied(stable_id: &str, scope: ProfileAvailabilityScope) -> ToolPolicyReconciliation {
    use crate::domain::tool_descriptor::{
        ToolAvailability, ToolCatalogueEntry, ToolHealth, ToolLifecycleKind, ToolSource,
    };
    let entry = ToolCatalogueEntry {
        stable_id: stable_id.to_string().into(),
        name: stable_id.to_string().into(),
        label: stable_id.to_string().into(),
        description: "test".into(),
        input_schema: "{}".into(),
        source: ToolSource::Runtime,
        owner: "test".into(),
        provider_id: "test".into(),
        version: None,
        lifecycle: ToolLifecycleKind::RuntimeLoadable,
        configurable: true,
        default_enabled: true,
        configured_enabled: None,
        profile_enabled: Some(scope.is_enabled()),
        profile_scope: Some(scope),
        session_enabled: None,
        explicit_restriction: None,
        runtime_availability: ToolAvailability::Enabled,
        effective_enabled: scope.is_enabled(),
        effective_scope: scope,
        effective_parent_enabled: scope.allows_parent(),
        effective_child_enabled: scope.allows_child(),
        health: ToolHealth::Ok,
    };
    ToolPolicyReconciliation {
        mode: ToolPolicyApplyMode::ImmediateIfIdle,
        results: vec![ToolPolicyMutationResult {
            name: stable_id.to_string(),
            requested_identifier: None,
            requested_availability: ToolAvailability::Enabled,
            requested_scope: scope,
            status: ToolPolicyMutationStatus::Applied,
            before: None,
            after: Some(entry),
            reason: "test".into(),
        }],
        correlation_id: None,
    }
}

#[test]
fn an_empty_reconciliation_writes_nothing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, "{}").unwrap();
    tool_policy_persistence_for(config_path.clone(), None)(&empty_reconciliation()).unwrap();
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), "{}");
}

#[test]
fn the_composed_hook_patches_only_the_policy_entries_of_its_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    let original = "{\n  \"custom\": \"kept\",\n  \"tools\": {\n    \"policy\": {\n      \"entries\": {\n        \"native:docs\": {\n          \"scope\": \"parent\"\n        }\n      }\n    }\n  }\n}\n";
    std::fs::write(&config_path, original).unwrap();
    let persist = tool_policy_persistence_for(config_path.clone(), None);
    persist(&applied("native:bash", ProfileAvailabilityScope::Child)).unwrap();
    let written = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        written.starts_with("{\n  \"custom\": \"kept\",\n"),
        "{written}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(
        parsed["tools"]["policy"]["entries"]["native:docs"]["scope"],
        "parent"
    );
    assert_eq!(
        parsed["tools"]["policy"]["entries"]["native:bash"]["scope"],
        "child"
    );
    assert!(
        parsed.get("agents").is_none(),
        "defaults are not expanded into the file"
    );
}

#[test]
fn the_composed_hook_creates_a_missing_file_with_only_the_entries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    let persist = tool_policy_persistence_for(config_path.clone(), None);
    persist(&applied("native:bash", ProfileAvailabilityScope::Both)).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({"tools":{"policy":{"entries":{"native:bash":{"scope":"both"}}}}})
    );
}

#[test]
fn the_composed_hook_reports_a_malformed_config_file_and_leaves_it_alone() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, "{ not json").unwrap();
    let persist = tool_policy_persistence_for(config_path.clone(), None);
    let error = persist(&applied("native:bash", ProfileAvailabilityScope::Both)).unwrap_err();
    assert!(error.contains("tool policy persistence"), "{error}");
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), "{ not json");
}

#[test]
fn a_non_object_on_the_way_to_the_entries_is_refused() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, r#"{"tools":{"policy":"junk"}}"#).unwrap();
    let error = persist_tool_policy_results(
        &config_path,
        None,
        &applied("native:bash", ProfileAvailabilityScope::Both),
    )
    .unwrap_err();
    assert!(error.contains("not a JSON object"), "{error}");
    assert!(
        error.contains(&config_path.display().to_string()),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        r#"{"tools":{"policy":"junk"}}"#
    );
    assert!(
        persist_tool_policy_results(
            Path::new("/nonexistent/dir/config.json"),
            None,
            &applied("native:bash", ProfileAvailabilityScope::Both)
        )
        .is_err()
    );
}

#[test]
fn an_unreadable_config_entry_is_reported() {
    let tmp = tempfile::TempDir::new().unwrap();
    let directory = tmp.path().join("config.json");
    std::fs::create_dir(&directory).unwrap();
    let error = persist_tool_policy_results(
        &directory,
        None,
        &applied("native:bash", ProfileAvailabilityScope::Both),
    )
    .unwrap_err();
    assert!(error.contains("tool policy persistence"), "{error}");
}

#[test]
fn an_entry_the_overlay_defines_is_refused_naming_the_overlay_and_the_remedy() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, "{}").unwrap();
    let overlay = tmp.path().join("repo").join(".quecto").join("config.json");
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    std::fs::write(
        &overlay,
        r#"{"tools":{"policy":{"entries":{"native:bash":{"scope":"parent"}}}}}"#,
    )
    .unwrap();
    let persist = tool_policy_persistence_for(config_path.clone(), Some(overlay.clone()));
    let error = persist(&applied("native:bash", ProfileAvailabilityScope::Both)).unwrap_err();
    assert!(error.contains(&overlay.display().to_string()), "{error}");
    assert!(
        error.contains("quecto config set tools.policy.entries.native:bash"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        "{}",
        "nothing written"
    );

    persist(&applied("native:docs", ProfileAvailabilityScope::Both)).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(
        parsed["tools"]["policy"]["entries"]["native:docs"]["scope"],
        "both"
    );

    std::fs::remove_file(&overlay).unwrap();
    persist(&applied("native:bash", ProfileAvailabilityScope::Both)).unwrap();
}
