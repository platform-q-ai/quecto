//! `get_report` presentation (#1859, #1974): the wire shape of a session
//! report and its raw-export receipt, and the busy reader task's parse of
//! the parent-local command. Selection, preview, export and admission are
//! the composed report owner's (`SessionReadHandles::export_report`).
use super::protocol::AgentEvent;
use crate::application::sessions::dto::{ReportError, SessionReport};
use serde_json::{Value, json};

/// The `get_report` response data: the report (or `null`), its recovery
/// ref when the preview is truncated, the snapshot marker, and the raw
/// export receipt when one was written.
pub(super) fn report_json(report: &SessionReport) -> Value {
    let mut data = match &report.report {
        Some(preview) => {
            json!({"report":{"messageId":preview.message_id.as_str(),"content":preview.content,
            "contentTruncated":preview.content_truncated,"fullLengthBytes":preview.full_length_bytes},
            "recovery": preview.recovery().map(|recovery| json!({"command":"get_message",
                "messageId":recovery.message_id.as_str(),"offset":recovery.offset})),
            "snapshot":true})
        }
        None => json!({"report":null,"snapshot":true}),
    };
    if let Some(receipt) = &report.raw_export {
        data["rawExport"] = json!({"path":receipt.records_path,"manifest":receipt.manifest_path,
            "sha256":receipt.sha256,"bytes":receipt.bytes,"scope":"retained_snapshot"});
    }
    data
}

/// The `get_report` response event for `id`.
pub(super) fn report_event(
    id: Option<&str>,
    result: Result<SessionReport, ReportError>,
) -> AgentEvent {
    match result {
        Ok(report) => AgentEvent::ok(id, "get_report", Some(report_json(&report))),
        Err(error) => AgentEvent::err(id, "get_report", error.to_string()),
    }
}

/// Serve a parent-local `get_report` from the busy reader task: a
/// report-only request is answered inline; a raw export is admitted (or
/// refused) by the report owner and answered from a detached task when it
/// completes, so the reader stays available for pause/abort meanwhile.
pub(super) async fn intercept(ctx: &super::uds_busy_get_message::BusyCommandCtx<'_>) -> bool {
    let Ok(super::protocol::AgentCommand::GetReport {
        id,
        agent_id: None,
        export_raw,
    }) = serde_json::from_str(ctx.line)
    else {
        return false;
    };
    let event = if export_raw {
        match ctx.session.export_report.admit_raw_export() {
            Ok(admitted) => {
                let registry = ctx.registry.clone();
                let client_id = ctx.client_id;
                tokio::spawn(async move {
                    let event = report_event(id.as_deref(), admitted.await);
                    if let Some(writer) =
                        super::uds_ext_protocol::client_writer_tx(&registry, client_id)
                    {
                        let _ = writer.send(event.to_json_line() + "\n").await;
                    }
                });
                return true;
            }
            Err(refused) => report_event(id.as_deref(), Err(refused)),
        }
    } else {
        report_event(id.as_deref(), ctx.session.export_report.report(false).await)
    };
    if let Some(writer) = super::uds_ext_protocol::client_writer_tx(ctx.registry, ctx.client_id) {
        let _ = writer.send(event.to_json_line() + "\n").await;
    }
    true
}

#[cfg(test)]
#[path = "uds_latest_report_tests.rs"]
mod tests;
