use super::*;
#[test]
fn latest_report_ignores_tool_backlog_and_bounds_unicode_content() {
    let text = "界".repeat(10000);
    let report = Message::assistant(text, vec![]);
    let id = report.id().to_string();
    let data = latest_report(&[report, Message::user("pending instruction")]);
    assert_eq!(data["report"]["messageId"], id);
    assert!(data["report"]["content"].as_str().unwrap().len() <= 8192);
    assert_eq!(data["report"]["contentTruncated"], true);
    assert_eq!(data["recovery"]["command"], "get_message");
}

#[tokio::test]
async fn raw_report_export_preserves_content_without_consuming_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let message = Message::assistant("raw".repeat(10000), vec![]);
    let snapshot =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(
            std::slice::from_ref(&message),
        )
        .active_session;
    let (epoch, rev) = {
        let state = snapshot.read().await;
        (state.conversation().epoch(), state.conversation().rev())
    };
    let result = report(&snapshot, Some(directory.path().to_path_buf()), true)
        .await
        .unwrap();
    assert!(result.to_string().len() < 10000);
    let path = result["rawExport"]["path"].as_str().unwrap();
    let raw = std::fs::read_to_string(path).unwrap();
    assert!(raw.contains(&message.content));
    assert_eq!(snapshot.read().await.conversation().epoch(), epoch);
    assert_eq!(snapshot.read().await.conversation().rev(), rev);
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
            // Slow but finite and cancellable, so the export permit taken by
            // the spawned task is released and never leaks across tests.
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
        let session = crate::interface::cli::uds::dispatch_session_roster_tests::read_handles_for(
            "test",
            Some(Arc::new(SlowExportStore)),
            &[],
        );
        let export_root: crate::interface::cli::uds_snapshots::ExportRootSlot =
            Arc::new(std::sync::Mutex::new(Some(directory.path().to_path_buf())));
        let registry = crate::interface::cli::uds_ext_protocol::new_client_tool_registry();
        let ctx = crate::interface::cli::uds_busy_get_message::BusyCommandCtx {
            line: r#"{"type":"get_report","id":"slow-export","export_raw":true}"#,
            session: &session,
            export_root: &export_root,
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
}

#[tokio::test]
async fn asynchronous_export_returns_correlated_artifact_or_storage_error() {
    use crate::interface::cli::{uds_busy_get_message::BusyCommandCtx, uds_ext_protocol};
    for writable in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("export");
        if !writable {
            std::fs::write(&root, "occupied").unwrap();
        }
        let session =
            crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[
                Message::assistant("report", vec![]),
            ]);
        let export_root: crate::interface::cli::uds_snapshots::ExportRootSlot =
            std::sync::Arc::new(std::sync::Mutex::new(Some(root)));
        let registry = uds_ext_protocol::new_client_tool_registry();
        let (writer, mut replies) = tokio::sync::mpsc::channel(2);
        uds_ext_protocol::register_client_writer(&registry, 1, writer);
        assert!(
            intercept(&BusyCommandCtx {
                line: r#"{"type":"get_report","id":"export-id","export_raw":true}"#,
                session: &session,
                export_root: &export_root,
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

mod unavailable_spill {
    use super::*;
    use crate::application::sessions::ports::{ContextSpillStore, SpillIndexList};
    use crate::domain::{
        error::DomainError,
        session::{SpillEntry, SpillIndex},
    };
    use std::{future::Future, pin::Pin, sync::Arc};
    struct Store(&'static str);
    impl ContextSpillStore for Store {
        fn append(
            &self,
            _: &crate::domain::session_identity::SessionIdentity,
            _: &SpillEntry,
        ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
            Box::pin(async { Err(DomainError::Tool("read-only fixture".into())) })
        }
        fn clear(
            &self,
            _: &crate::domain::session_identity::SessionIdentity,
        ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
            Box::pin(async { Err(DomainError::Tool("read-only fixture".into())) })
        }
        fn list_entries(
            &self,
            _: &crate::domain::session_identity::SessionIdentity,
        ) -> SpillIndexList<'_> {
            Box::pin(async move {
                match self.0 {
                    "index unavailable" => Err(DomainError::Tool(self.0.into())),
                    _ => Ok(Arc::new(vec![SpillIndex {
                        id: "spill-1".into(),
                        tool: "bash".into(),
                        input_preview: "test".into(),
                        tokens: 1,
                    }])),
                }
            })
        }
        fn recall(
            &self,
            _: &crate::domain::session_identity::SessionIdentity,
            _: &crate::domain::session_identity::SpillId,
        ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>>
        {
            Box::pin(async move {
                match self.0 {
                    "recall unavailable" => Err(DomainError::Tool(self.0.into())),
                    _ => Ok(None),
                }
            })
        }
    }
    #[tokio::test]
    async fn incomplete_spill_exports_fail_explicitly_without_partial_artifacts() {
        for reason in [
            "index unavailable",
            "recall unavailable",
            "spill disappeared",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let snapshot =
                crate::interface::cli::uds::dispatch_session_roster_tests::read_handles_for(
                    "test",
                    Some(Arc::new(Store(reason))),
                    &[],
                )
                .active_session;
            assert!(
                report(&snapshot, Some(directory.path().to_path_buf()), true)
                    .await
                    .unwrap_err()
                    .contains(reason)
            );
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }
}

/// The report resolves the chosen assistant message through the ledger so a
/// context-collapsed stub is never returned as the full text.
#[tokio::test]
async fn report_prefers_the_ledger_copy_over_a_collapsed_stub() {
    use crate::domain::message::Message;
    let full = Message::assistant("the complete final report", vec![]);
    let id = full.id().to_string();
    let snapshot =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(
            std::slice::from_ref(&full),
        )
        .active_session;
    let mut stub = full.clone();
    stub.content = "recall(spilled)".to_string();
    snapshot.write().await.publish(&[stub]);
    let data = report(&snapshot, None, false).await.unwrap();
    assert_eq!(data["report"]["messageId"], id);
    assert_eq!(data["report"]["content"], "the complete final report");
    assert_eq!(data["report"]["contentTruncated"], false);
}
