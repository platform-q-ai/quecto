//! Step definitions for the D4 (#1974) report/export scenarios of
//! `uds_paged_history.feature`: the latest substantive assistant report
//! with its bounded preview and recovery ref, the null report of a session
//! without one, and the raw export under the artifacts directory — all
//! driven over the REAL UDS server of a seeded persisted session (the
//! plumbing of `uds_paged_history_steps` and `uds_history_recovery_steps`).

use super::uds_history_recovery_steps::{attach, request, response, seed_session};
use super::*;
use quecto::domain::message::{Message, ToolCall};
use sha2::{Digest, Sha256};

const LONG_REPORT_CHARS: usize = 4000;

fn long_report() -> String {
    "界".repeat(LONG_REPORT_CHARS)
}

// ── Given ───────────────────────────────────────────────────────────────────

#[given("a persisted UDS session whose latest assistant report is longer than the report preview")]
fn given_long_report_session(world: &mut QuectoWorld) {
    seed_session(
        world,
        vec![
            Message::user("please summarise"),
            Message::assistant("an earlier, shorter report", vec![]),
            Message::assistant(long_report(), vec![]),
            Message::assistant(
                "",
                vec![ToolCall {
                    id: "call-after".into(),
                    name: "bash".into(),
                    arguments: "{\"command\":\"true\"}".into(),
                }],
            ),
            Message::user("a pending instruction"),
        ],
    );
}

// ── When ────────────────────────────────────────────────────────────────────

#[when("a client requests the latest report")]
fn when_latest_report(world: &mut QuectoWorld) {
    attach(world);
    request(
        world,
        "get_report",
        "report-1",
        serde_json::json!({"type": "get_report", "id": "report-1"}),
    );
}

#[when("a client requests the latest report with a raw export")]
fn when_latest_report_with_export(world: &mut QuectoWorld) {
    attach(world);
    request(
        world,
        "get_report",
        "report-raw",
        serde_json::json!({"type": "get_report", "id": "report-raw", "export_raw": true}),
    );
}

// ── Then ────────────────────────────────────────────────────────────────────

#[then(
    "the report should be the latest substantive assistant message bounded to the preview with a recovery reference"
)]
fn then_bounded_report(world: &mut QuectoWorld) {
    let response = response(world);
    assert_eq!(response["success"], true, "{response}");
    let data = &response["data"];
    let report = &data["report"];
    let content = report["content"].as_str().expect("report content");
    assert!(content.len() <= 8192, "bounded: {}", content.len());
    assert_eq!(content.len(), 8190, "8192 falls inside a 3-byte character");
    assert!(long_report().starts_with(content));
    assert_eq!(report["contentTruncated"], true, "{data}");
    assert_eq!(report["fullLengthBytes"], (LONG_REPORT_CHARS * 3) as u64);
    assert_eq!(data["recovery"]["command"], "get_message", "{data}");
    assert_eq!(data["recovery"]["messageId"], report["messageId"]);
    assert_eq!(data["recovery"]["offset"], 8190);
    assert_eq!(data["snapshot"], true);
    assert!(data.get("rawExport").is_none(), "no export unless asked");
}

#[then("the report should be null")]
fn then_null_report(world: &mut QuectoWorld) {
    let response = response(world);
    assert_eq!(response["success"], true, "{response}");
    let data = &response["data"];
    assert!(data["report"].is_null(), "{data}");
    assert!(data.get("recovery").is_none(), "{data}");
    assert_eq!(data["snapshot"], true);
}

#[then(
    "the raw export should record the retained messages under the artifacts directory with a checksum manifest"
)]
fn then_raw_export(world: &mut QuectoWorld) {
    let base = base_path(world);
    let response = response(world);
    assert_eq!(response["success"], true, "{response}");
    let data = &response["data"];
    assert_eq!(data["report"]["contentTruncated"], true, "{data}");
    let export = &data["rawExport"];
    let records_path = std::path::PathBuf::from(export["path"].as_str().expect("path"));
    let manifest_path = std::path::PathBuf::from(export["manifest"].as_str().expect("manifest"));
    let root = base
        .join("artifacts/session-exports")
        .canonicalize()
        .expect("the export root exists under the harness base");
    assert!(
        records_path.starts_with(&root),
        "{records_path:?} under {root:?}"
    );
    assert_eq!(records_path.parent(), manifest_path.parent());
    assert_eq!(export["scope"], "retained_snapshot");
    let raw = std::fs::read_to_string(&records_path).expect("records");
    let lines: Vec<serde_json::Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).expect("json record"))
        .collect();
    assert!(
        lines
            .iter()
            .any(|record| record["kind"] == "message"
                && record["message"]["content"] == long_report()),
        "the full report is exported, not the preview"
    );
    assert!(
        lines
            .iter()
            .any(|record| record["message"]["toolCalls"][0]["id"] == "call-after"),
        "tool-call steps are retained records"
    );
    let mut hash = Sha256::new();
    hash.update(raw.as_bytes());
    let sha256 = format!("{:x}", hash.finalize());
    assert_eq!(export["sha256"], sha256);
    assert_eq!(export["bytes"], raw.len() as u64);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("manifest")).unwrap();
    assert_eq!(manifest["format"], 1);
    assert_eq!(manifest["recordCount"], lines.len() as u64);
    assert_eq!(manifest["spillCount"], 0);
    assert_eq!(manifest["sha256"], sha256);
    assert_eq!(manifest["bytes"], raw.len() as u64);
    assert!(manifest["epoch"].is_u64() && manifest["revision"].is_u64());
}
