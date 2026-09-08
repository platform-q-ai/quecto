//! Latest published assistant report, independent of unread-history delivery cursors.
use crate::domain::message::{Message, Role};
use serde_json::{Value, json};

pub(super) fn latest_report(messages: &[Message]) -> Value {
    let Some(message) = messages.iter().rev().find(|message| {
        message.role == Role::Assistant
            && message.tool_calls.is_empty()
            && !message.content.trim().is_empty()
    }) else {
        return json!({"report":null,"snapshot":true});
    };
    let mut end = message.content.len().min(8192);
    while !message.content.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < message.content.len();
    json!({"report":{"messageId":message.id().to_string(),"content": &message.content[..end],
        "contentTruncated":truncated,"fullLengthBytes":message.content.len()},
        "recovery": truncated.then(|| json!({"command":"get_message","messageId":message.id().to_string(),"offset":end})),
        "snapshot":true})
}

pub(super) async fn intercept(ctx: &super::uds_busy_get_message::BusyCommandCtx<'_>) -> bool {
    let Ok(super::protocol::AgentCommand::GetReport {
        id,
        agent_id: None,
        export_raw,
    }) = serde_json::from_str(ctx.line)
    else {
        return false;
    };
    if export_raw {
        static EXPORTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
        match EXPORTS.try_acquire() {
            Ok(permit) => {
                let snapshot = ctx.snapshot.clone();
                let registry = ctx.registry.clone();
                let client_id = ctx.client_id;
                tokio::spawn(async move {
                    let event = match report(&snapshot, true).await {
                        Ok(data) => {
                            super::protocol::AgentEvent::ok(id.as_deref(), "get_report", Some(data))
                        }
                        Err(error) => {
                            super::protocol::AgentEvent::err(id.as_deref(), "get_report", error)
                        }
                    };
                    if let Some(writer) =
                        super::uds_ext_protocol::client_writer_tx(&registry, client_id)
                    {
                        let _ = writer.send(event.to_json_line() + "\n").await;
                    }
                    drop(permit);
                });
                return true;
            }
            Err(_) => {
                let event = super::protocol::AgentEvent::err(
                    id.as_deref(),
                    "get_report",
                    "two raw exports are already running; retry after completion",
                );
                if let Some(writer) =
                    super::uds_ext_protocol::client_writer_tx(ctx.registry, ctx.client_id)
                {
                    let _ = writer.send(event.to_json_line() + "\n").await;
                }
                return true;
            }
        }
    }
    let event = match report(ctx.snapshot, false).await {
        Ok(data) => super::protocol::AgentEvent::ok(id.as_deref(), "get_report", Some(data)),
        Err(error) => super::protocol::AgentEvent::err(id.as_deref(), "get_report", error),
    };
    if let Some(writer) = super::uds_ext_protocol::client_writer_tx(ctx.registry, ctx.client_id) {
        let _ = writer.send(event.to_json_line() + "\n").await;
    }
    true
}

#[cfg(test)]
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

pub(super) async fn report(
    snapshot: &super::uds_multi::ConversationSnapshot,
    export_raw: bool,
) -> Result<Value, String> {
    let (mut data, export) = {
        let state = snapshot.read().await;
        let export = if export_raw {
            let root = state
                .export_root
                .clone()
                .ok_or("session export directory unavailable")?;
            Some((
                root,
                state.epoch,
                state.rev,
                state.export_messages(),
                state.export_spill_source(),
            ))
        } else {
            None
        };
        (latest_report(&state.messages), export)
    };
    if let Some((root, epoch, revision, messages, (spill_store, session_key))) = export {
        let mut records: Vec<Value> = messages.into_iter().map(|message| json!({"kind":"message","message":super::uds_session::message_to_json(&message)})).collect();
        let mut spill_count = 0;
        if let Some(store) = spill_store {
            for entry in store
                .list_entries(&session_key)
                .await
                .map_err(|error| error.to_string())?
                .iter()
            {
                let spill = store
                    .recall(&session_key, &entry.id)
                    .await
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("spill disappeared during export: {}", entry.id))?;
                records.push(json!({"kind":"spill","id":spill.id,"tool":spill.tool,"input_preview":spill.input_preview,"tokens":spill.tokens,"content":spill.content}));
                spill_count += 1;
            }
        }
        if snapshot.read().await.epoch == epoch {
            let metadata = json!({"format":1,"epoch":epoch,"revision":revision,"recordCount":records.len(),"spillCount":spill_count,
                "scope":"retained live and full-message ledger plus available spill entries; previously evicted or cleared data is not reconstructed",
                "spillConsistency":"entries read after the message snapshot; concurrent appends may be absent"});
            data["rawExport"] = tokio::task::spawn_blocking(move || {
                crate::infrastructure::session_export::write(&root, records, metadata)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        } else {
            return Err("session changed during export; retry against the new epoch".into());
        }
    }
    Ok(data)
}

#[cfg(test)]
#[tokio::test]
async fn raw_report_export_preserves_content_without_consuming_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let message = Message::assistant("raw".repeat(10000), vec![]);
    let mut state =
        super::uds_snapshots::ConversationSnapshotData::from_messages(vec![message.clone()]);
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

#[cfg(test)]
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
        let mut state = super::super::uds_snapshots::ConversationSnapshotData::default();
        state.export_root = Some(directory.path().to_path_buf());
        state.set_spill_store(Some(Arc::new(SlowExportStore)), "test".into());
        let snapshot = Arc::new(tokio::sync::RwLock::new(state));
        let registry = super::super::uds_ext_protocol::new_client_tool_registry();
        let ctx = super::super::uds_busy_get_message::BusyCommandCtx {
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
