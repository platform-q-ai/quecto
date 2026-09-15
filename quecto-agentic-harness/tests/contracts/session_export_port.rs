//! Contract tests for the `SessionExportPort` port (#1859, #1974).
//!
//! Drives `FileSessionExport` through the trait object. The contract is:
//! an export lands as a fresh directory under the root holding
//! `records.jsonl` (one JSON object per record, in order) and
//! `manifest.json` (the use case's statements plus checksum, size and
//! runtime), the receipt names both files and the checksum of the bytes
//! written, two exports never share a directory, and a failed export
//! leaves no directory behind.

use quecto::application::sessions::dto::{ExportManifest, ExportRecord};
use quecto::application::sessions::ports::export::SessionExportPort;
use quecto::domain::message::Message;
use quecto::domain::session::SpillEntry;
use quecto::infrastructure::session_export::FileSessionExport;
use sha2::{Digest, Sha256};
use std::sync::Arc;

fn under_test(root: std::path::PathBuf) -> Arc<dyn SessionExportPort> {
    Arc::new(FileSessionExport::new(root))
}

fn manifest(record_count: usize, spill_count: usize) -> ExportManifest {
    ExportManifest {
        epoch: 4,
        revision: 11,
        record_count,
        spill_count,
    }
}

fn records() -> Vec<ExportRecord> {
    vec![
        ExportRecord::Message(Box::new(Message::user("hello"))),
        ExportRecord::Message(Box::new(Message::assistant("the report", vec![]))),
        ExportRecord::Spill(SpillEntry {
            id: "spill-1".into(),
            tool: "bash".into(),
            input_preview: "ls".into(),
            tokens: 2,
            content: "listing".into(),
        }),
    ]
}

#[tokio::test]
async fn an_export_lands_under_the_root_with_records_in_order_and_a_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("exports");
    let exporter = under_test(root.clone());
    let receipt = exporter
        .write_export(records(), manifest(3, 1))
        .await
        .unwrap();
    assert!(
        receipt
            .records_path
            .starts_with(root.canonicalize().unwrap())
    );
    assert_eq!(receipt.records_path.file_name().unwrap(), "records.jsonl");
    assert_eq!(receipt.manifest_path.file_name().unwrap(), "manifest.json");
    assert_eq!(
        receipt.records_path.parent(),
        receipt.manifest_path.parent()
    );
    let raw = std::fs::read_to_string(&receipt.records_path).unwrap();
    let lines: Vec<serde_json::Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["kind"], "message");
    assert_eq!(lines[0]["message"]["role"], "user");
    assert_eq!(lines[1]["message"]["content"], "the report");
    assert_eq!(lines[2]["kind"], "spill");
    assert_eq!(lines[2]["content"], "listing");
    let mut hash = Sha256::new();
    hash.update(raw.as_bytes());
    assert_eq!(receipt.sha256, format!("{:x}", hash.finalize()));
    assert_eq!(receipt.bytes, raw.len() as u64);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["format"], 1);
    assert_eq!(manifest["epoch"], 4);
    assert_eq!(manifest["revision"], 11);
    assert_eq!(manifest["recordCount"], 3);
    assert_eq!(manifest["spillCount"], 1);
    assert_eq!(manifest["sha256"], receipt.sha256);
    assert_eq!(manifest["bytes"], receipt.bytes);
    assert!(manifest["runtime"].is_object());
}

#[tokio::test]
async fn consecutive_exports_never_share_a_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let exporter = under_test(tmp.path().to_path_buf());
    let records = records();
    let first = exporter
        .write_export(records.clone(), manifest(3, 1))
        .await
        .unwrap();
    let second = exporter
        .write_export(records, manifest(3, 1))
        .await
        .unwrap();
    assert_ne!(first.records_path, second.records_path);
    assert_eq!(first.sha256, second.sha256, "same bytes, same checksum");
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn an_empty_export_is_a_valid_artifact() {
    let tmp = tempfile::tempdir().unwrap();
    let receipt = under_test(tmp.path().to_path_buf())
        .write_export(Vec::new(), manifest(0, 0))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&receipt.records_path).unwrap(), b"");
    assert_eq!(receipt.bytes, 0);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["recordCount"], 0);
}

#[tokio::test]
async fn an_unusable_root_fails_without_an_artifact() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("exports");
    std::fs::write(&root, "a file where the root should be").unwrap();
    let error = under_test(root.clone())
        .write_export(records(), manifest(3, 1))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("session export"),
        "the failure names the export: {error}"
    );
    assert!(root.is_file(), "nothing replaced the occupant");
}
