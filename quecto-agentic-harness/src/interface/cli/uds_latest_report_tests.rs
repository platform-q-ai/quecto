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
    let mut state =
        crate::interface::cli::uds_snapshots::ConversationSnapshotData::from_messages(vec![
            message.clone(),
        ]);
    state.export_root = Some(directory.path().to_path_buf());
    let epoch = state.epoch;
    let rev = state.rev;
    let snapshot = std::sync::Arc::new(tokio::sync::RwLock::new(state));
    let result = report(&snapshot, true).await.unwrap();
    assert!(result.to_string().len() < 10000);
    let path = result["rawExport"]["path"].as_str().unwrap();
    let raw = std::fs::read_to_string(path).unwrap();
    assert!(raw.contains(&message.content));
    assert_eq!(snapshot.read().await.epoch, epoch);
    assert_eq!(snapshot.read().await.rev, rev);
}

mod export_control_tests {
    use super::*;
    use crate::domain::{
        error::DomainError,
        session::{ContextSpillStore, SpillEntry, SpillIndexList},
    };
    use std::{future::Future, pin::Pin, sync::Arc};
    struct SlowExportStore;
    impl ContextSpillStore for SlowExportStore {
        fn append(
            &self,
            _: &str,
            _: &SpillEntry,
        ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
        fn recall(
            &self,
            _: &str,
            _: &str,
        ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>>
        {
            Box::pin(async { Ok(None) })
        }
        fn list_entries(&self, _: &str) -> SpillIndexList<'_> {
            Box::pin(std::future::pending())
        }
        fn clear(
            &self,
            _: &str,
        ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }
    }
    #[tokio::test]
    async fn raw_export_releases_reader_before_spill_io_completes() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = crate::interface::cli::uds_snapshots::ConversationSnapshotData::default();
        state.export_root = Some(directory.path().to_path_buf());
        state.set_spill_store(Some(Arc::new(SlowExportStore)), "test".into());
        let snapshot = Arc::new(tokio::sync::RwLock::new(state));
        let registry = crate::interface::cli::uds_ext_protocol::new_client_tool_registry();
        let ctx = crate::interface::cli::uds_busy_get_message::BusyCommandCtx {
            line: r#"{"type":"get_report","id":"slow-export","export_raw":true}"#,
            snapshot: &snapshot,
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
    use crate::interface::cli::{
        uds_busy_get_message::BusyCommandCtx, uds_ext_protocol,
        uds_snapshots::ConversationSnapshotData,
    };
    for writable in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("export");
        if !writable {
            std::fs::write(&root, "occupied").unwrap();
        }
        let mut state =
            ConversationSnapshotData::from_messages(vec![Message::assistant("report", vec![])]);
        state.export_root = Some(root);
        let snapshot = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        let registry = uds_ext_protocol::new_client_tool_registry();
        let (writer, mut replies) = tokio::sync::mpsc::channel(2);
        uds_ext_protocol::register_client_writer(&registry, 1, writer);
        assert!(
            intercept(&BusyCommandCtx {
                line: r#"{"type":"get_report","id":"export-id","export_raw":true}"#,
                snapshot: &snapshot,
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
    use crate::domain::{
        error::DomainError,
        session::{ContextSpillStore, SpillEntry, SpillIndex, SpillIndexList},
    };
    use std::{future::Future, pin::Pin, sync::Arc};
    struct Store(&'static str);
    impl ContextSpillStore for Store {
        fn append(
            &self,
            _: &str,
            _: &SpillEntry,
        ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
            Box::pin(async { Err(DomainError::Tool("read-only fixture".into())) })
        }
        fn clear(
            &self,
            _: &str,
        ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
            Box::pin(async { Err(DomainError::Tool("read-only fixture".into())) })
        }
        fn list_entries(&self, _: &str) -> SpillIndexList<'_> {
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
            _: &str,
            _: &str,
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
            let mut state =
                crate::interface::cli::uds_snapshots::ConversationSnapshotData::default();
            state.export_root = Some(directory.path().to_path_buf());
            state.set_spill_store(Some(Arc::new(Store(reason))), "test".into());
            let snapshot = Arc::new(tokio::sync::RwLock::new(state));
            assert!(report(&snapshot, true).await.unwrap_err().contains(reason));
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }
}
