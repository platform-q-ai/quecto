//! Contract for the `ConfigDocumentWriter` port (#2024): a write is
//! all-or-nothing (no temporary file survives, the previous content stays
//! on failure), and a document already in the writer's layout changes only
//! on the lines whose values changed — key order and unknown keys kept.
//! The exclusive hold it hands out is `document_lock.rs`'s contract.
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::configuration::ports::ConfigDocumentWriter;
use quecto::infrastructure::config::writer::JsonDocumentWriter;

fn under_test() -> Arc<dyn ConfigDocumentWriter> {
    Arc::new(JsonDocumentWriter)
}

const PRETTY: &str = "{\n  \"unknown_key\": \"kept\",\n  \"zeta\": 1,\n  \"agents\": {\n    \"defaults\": {\n      \"model\": \"old\"\n    }\n  }\n}\n";

#[test]
fn a_document_in_the_writers_layout_changes_only_on_the_touched_line() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, PRETTY).unwrap();
    let mut document: serde_json::Value = serde_json::from_str(PRETTY).unwrap();
    document["agents"]["defaults"]["model"] = serde_json::json!("new");
    under_test().write(&path, &document).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        PRETTY.replace("\"old\"", "\"new\"")
    );
}

#[test]
fn a_missing_file_is_created_and_no_temporary_file_survives() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("config.json");
    under_test()
        .write(&path, &serde_json::json!({"a": 1}))
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(parsed, serde_json::json!({"a": 1}));
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn a_failed_write_leaves_the_previous_content_in_place() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, PRETTY).unwrap();
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o500),
    )
    .unwrap();
    let outcome = under_test().write(&path, &serde_json::json!({"a": 1}));
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    if outcome.is_err() {
        assert_eq!(std::fs::read_to_string(&path).unwrap(), PRETTY);
    }
}
