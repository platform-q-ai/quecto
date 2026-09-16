use super::*;
use crate::application::sessions::dto::{RawExportReceipt, ReportPreview};
use crate::domain::ids::MessageId;
use crate::domain::message::Message;
use crate::interface::cli::uds::dispatch_session_roster_tests::{
    composed_sessions_for, seeded_read_handles,
};
use crate::interface::cli::uds_busy_get_message::BusyCommandCtx;
use crate::interface::cli::uds_ext_protocol;

#[test]
fn a_truncated_report_presents_its_preview_and_recovery_ref() {
    let message = Message::assistant("界".repeat(10_000), vec![]);
    let preview = ReportPreview::of(&message);
    let data = report_json(&SessionReport {
        report: Some(preview),
        raw_export: None,
    });
    assert_eq!(data["report"]["messageId"], message.id().to_string());
    assert_eq!(data["report"]["content"].as_str().unwrap().len(), 8190);
    assert_eq!(data["report"]["contentTruncated"], true);
    assert_eq!(data["report"]["fullLengthBytes"], 30_000);
    assert_eq!(data["recovery"]["command"], "get_message");
    assert_eq!(data["recovery"]["messageId"], message.id().to_string());
    assert_eq!(data["recovery"]["offset"], 8190);
    assert_eq!(data["snapshot"], true);
    assert!(data.get("rawExport").is_none());
}

#[test]
fn a_whole_report_presents_a_null_recovery_and_no_report_omits_it() {
    let data = report_json(&SessionReport {
        report: Some(ReportPreview {
            message_id: MessageId::from("m-1"),
            content: "done".into(),
            content_truncated: false,
            full_length_bytes: 4,
        }),
        raw_export: None,
    });
    assert_eq!(data["report"]["contentTruncated"], false);
    assert!(data["recovery"].is_null());
    assert!(data.as_object().unwrap().contains_key("recovery"));
    let none = report_json(&SessionReport {
        report: None,
        raw_export: None,
    });
    assert_eq!(none, serde_json::json!({"report":null,"snapshot":true}));
}

#[test]
fn a_raw_export_receipt_is_presented_with_its_retained_snapshot_scope() {
    let data = report_json(&SessionReport {
        report: None,
        raw_export: Some(RawExportReceipt {
            records_path: "/x/records.jsonl".into(),
            manifest_path: "/x/manifest.json".into(),
            sha256: "ab".into(),
            bytes: 12,
        }),
    });
    assert_eq!(
        data["rawExport"],
        serde_json::json!({"path":"/x/records.jsonl","manifest":"/x/manifest.json","sha256":"ab","bytes":12,"scope":"retained_snapshot"})
    );
    let refused = report_event(Some("r-1"), Err(ReportError::EpochChanged));
    let line: Value = serde_json::from_str(&refused.to_json_line()).unwrap();
    assert_eq!(line["id"], "r-1");
    assert_eq!(line["command"], "get_report");
    assert_eq!(line["success"], false);
    assert_eq!(
        line["error"],
        "session changed during export; retry against the new epoch"
    );
}

#[tokio::test]
async fn raw_report_export_preserves_content_without_consuming_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let message = Message::assistant("raw".repeat(10000), vec![]);
    let session = seeded_read_handles(
        directory.path(),
        "cli:report",
        None,
        std::slice::from_ref(&message),
    );
    let (epoch, rev) = {
        let state = session.active_session.read().await;
        (state.conversation().epoch(), state.conversation().rev())
    };
    let result = report_json(&session.export_report.report(true).await.unwrap());
    assert!(result.to_string().len() < 10000);
    let path = result["rawExport"]["path"].as_str().unwrap();
    assert!(path.contains("artifacts/session-exports"));
    let raw = std::fs::read_to_string(path).unwrap();
    assert!(raw.contains(&message.content));
    assert_eq!(
        session.active_session.read().await.conversation().epoch(),
        epoch
    );
    assert_eq!(
        session.active_session.read().await.conversation().rev(),
        rev
    );
}

mod export_control_tests {
    use super::*;
    use crate::application::sessions::ports::{ContextSpillStore, SpillIndexList};
    use crate::domain::{error::DomainError, session::SpillEntry};
    use std::{future::Future, pin::Pin, sync::Arc};
    struct SlowExportStore;
    impl ContextSpillStore for SlowExportStore {
        fn append(
            &self,
            _: &crate::domain::session_identity::SessionIdentity,
            _: &SpillEntry,
        ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
        fn recall(
            &self,
            _: &crate::domain::session_identity::SessionIdentity,
            _: &crate::domain::session_identity::SpillId,
        ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>>
        {
            Box::pin(async { Ok(None) })
        }
        fn list_entries(
            &self,
            _: &crate::domain::session_identity::SessionIdentity,
        ) -> SpillIndexList<'_> {
            // Slow but finite and cancellable, so the export slot taken by
            // the detached task is released and never leaks across tests.
            Box::pin(async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                Ok(Arc::new(Vec::new()))
            })
        }
        fn clear(
            &self,
            _: &crate::domain::session_identity::SessionIdentity,
        ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
    }
    #[tokio::test]
    async fn raw_export_releases_reader_before_spill_io_completes() {
        let directory = tempfile::tempdir().unwrap();
        let session = composed_sessions_for(
            directory.path(),
            "cli:slow",
            Some(Arc::new(SlowExportStore)),
        )
        .read_handles();
        let registry = uds_ext_protocol::new_client_tool_registry();
        let ctx = BusyCommandCtx {
            line: r#"{"type":"get_report","id":"slow-export","export_raw":true}"#,
            session: &session,
            registry: &registry,
            client_id: 1,
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), intercept(&ctx))
                .await
                .is_ok(),
            "the same reader must remain available for pause/abort"
        );
    }

    #[tokio::test]
    async fn a_third_concurrent_busy_export_is_refused_with_a_correlated_error() {
        let directory = tempfile::tempdir().unwrap();
        let session = composed_sessions_for(
            directory.path(),
            "cli:bound",
            Some(Arc::new(SlowExportStore)),
        )
        .read_handles();
        let registry = uds_ext_protocol::new_client_tool_registry();
        let (writer, mut replies) = tokio::sync::mpsc::channel(4);
        uds_ext_protocol::register_client_writer(&registry, 7, writer);
        for id in ["one", "two", "three"] {
            let line = format!(r#"{{"type":"get_report","id":"{id}","export_raw":true}}"#);
            assert!(
                intercept(&BusyCommandCtx {
                    line: &line,
                    session: &session,
                    registry: &registry,
                    client_id: 7,
                })
                .await
            );
        }
        let refusal: Value = serde_json::from_str(
            &tokio::time::timeout(std::time::Duration::from_millis(100), replies.recv())
                .await
                .expect("the refusal answers before the admitted exports")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(refusal["id"], "three");
        assert_eq!(refusal["success"], false);
        assert_eq!(
            refusal["error"],
            "two raw exports are already running; retry after completion"
        );
        for _ in 0..2 {
            let admitted: Value = serde_json::from_str(
                &tokio::time::timeout(std::time::Duration::from_secs(3), replies.recv())
                    .await
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(admitted["success"], true, "{admitted}");
            assert!(admitted["data"]["rawExport"]["path"].is_string());
        }
    }
}

#[tokio::test]
async fn asynchronous_export_returns_correlated_artifact_or_storage_error() {
    for writable in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        if !writable {
            std::fs::create_dir_all(directory.path().join("artifacts")).unwrap();
            std::fs::write(
                directory.path().join("artifacts/session-exports"),
                "occupied",
            )
            .unwrap();
        }
        let session = seeded_read_handles(
            directory.path(),
            "cli:async",
            None,
            &[Message::assistant("report", vec![])],
        );
        let registry = uds_ext_protocol::new_client_tool_registry();
        let (writer, mut replies) = tokio::sync::mpsc::channel(2);
        uds_ext_protocol::register_client_writer(&registry, 1, writer);
        assert!(
            intercept(&BusyCommandCtx {
                line: r#"{"type":"get_report","id":"export-id","export_raw":true}"#,
                session: &session,
                registry: &registry,
                client_id: 1
            })
            .await
        );
        let line = tokio::time::timeout(std::time::Duration::from_secs(3), replies.recv())
            .await
            .unwrap()
            .unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], "export-id");
        assert_eq!(reply["success"], writable);
        if writable {
            let path = reply["data"]["rawExport"]["path"].as_str().unwrap();
            assert!(std::fs::read_to_string(path).unwrap().contains("report"));
        } else {
            assert!(reply["error"].as_str().unwrap().contains("session export"));
        }
    }
}

#[tokio::test]
async fn a_busy_report_only_request_is_answered_inline_and_is_not_forwarded() {
    let directory = tempfile::tempdir().unwrap();
    let session = seeded_read_handles(
        directory.path(),
        "cli:inline",
        None,
        &[Message::assistant("inline report", vec![])],
    );
    let registry = uds_ext_protocol::new_client_tool_registry();
    let (writer, mut replies) = tokio::sync::mpsc::channel(2);
    uds_ext_protocol::register_client_writer(&registry, 3, writer);
    assert!(
        intercept(&BusyCommandCtx {
            line: r#"{"type":"get_report","id":"inline"}"#,
            session: &session,
            registry: &registry,
            client_id: 3
        })
        .await
    );
    let reply: Value = serde_json::from_str(&replies.try_recv().unwrap()).unwrap();
    assert_eq!(reply["data"]["report"]["content"], "inline report");
    assert_eq!(reply["data"]["snapshot"], true);
    assert!(reply["data"].get("rawExport").is_none());
    assert!(
        !intercept(&BusyCommandCtx {
            line: r#"{"type":"get_report","id":"child","agent_id":"child-1"}"#,
            session: &session,
            registry: &registry,
            client_id: 3
        })
        .await,
        "a child-targeted report is forwarded, never served from the parent"
    );
    assert!(
        std::fs::read_dir(directory.path().join("artifacts")).is_err(),
        "no export directory without export_raw"
    );
}

/// The report resolves the chosen assistant message through the ledger so a
/// context-collapsed stub is never returned as the full text.
#[tokio::test]
async fn report_prefers_the_ledger_copy_over_a_collapsed_stub() {
    let directory = tempfile::tempdir().unwrap();
    let full = Message::assistant("the complete final report", vec![]);
    let id = full.id().to_string();
    let session = seeded_read_handles(
        directory.path(),
        "cli:stub",
        None,
        std::slice::from_ref(&full),
    );
    let mut stub = full.clone();
    stub.content = "recall(spilled)".to_string();
    session.active_session.write().await.publish(&[stub]);
    let data = report_json(&session.export_report.report(false).await.unwrap());
    assert_eq!(data["report"]["messageId"], id);
    assert_eq!(data["report"]["content"], "the complete final report");
    assert_eq!(data["report"]["contentTruncated"], false);
}
