use std::collections::BTreeMap;

use tempfile::TempDir;

use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::tools::inherited_tool_policy::{
    InheritedToolPolicySnapshot, load_validate_unlink, write_snapshot,
};

#[test]
fn snapshot_round_trip_validates_and_unlinks_private_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("policy.json");
    let snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([
        ("read".to_string(), ProfileAvailabilityScope::Both),
        ("bash".to_string(), ProfileAvailabilityScope::None),
    ]));

    write_snapshot(&path, &snapshot).unwrap();
    assert!(path.exists());

    let loaded = load_validate_unlink(&path).unwrap();
    assert_eq!(loaded, snapshot);
    assert!(!path.exists());
}

#[test]
fn load_rejects_unsupported_snapshot_version_and_unlinks() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("policy.json");
    std::fs::write(&path, r#"{"version":999,"tools":{"read":"both"}}"#).unwrap();

    let err = load_validate_unlink(&path).unwrap_err();

    assert!(err.contains("unsupported inherited tool policy snapshot version 999"));
    assert!(!path.exists());
}

#[test]
fn load_rejects_empty_tool_ids_and_unlinks() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("policy.json");
    std::fs::write(&path, r#"{"version":1,"tools":{" ":"both"}}"#).unwrap();

    let err = load_validate_unlink(&path).unwrap_err();

    assert!(err.contains("empty tool id"));
    assert!(!path.exists());
}

#[test]
fn load_reports_missing_snapshot_without_creating_or_unlinking() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("missing-policy.json");

    let err = load_validate_unlink(&path).unwrap_err();

    assert!(err.contains("read inherited tool policy snapshot"));
    assert!(!path.exists());
}

#[test]
fn load_reports_malformed_snapshot_and_unlinks() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("policy.json");
    std::fs::write(&path, b"not json").unwrap();

    let err = load_validate_unlink(&path).unwrap_err();

    assert!(err.contains("parse inherited tool policy snapshot"));
    assert!(!path.exists());
}
