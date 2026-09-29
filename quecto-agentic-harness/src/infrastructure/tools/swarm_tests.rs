//! The swarm tool's own refusals (#2282): telemetry per refused op.
use super::SwarmTool;
use crate::application::tools::ports::Tool;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<String>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(bytes));
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedLog {
    type Writer = Self;
    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

const SECRET: &str = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// Each refused call records one event on the board's telemetry target with
/// its refusal kind, and whether it named a removed workbench op, and,
/// while the event log is on, one `swarm_op` record: op `unknown`, refused
/// as `calling`, by the caller's redacted ref. The member's text (an op
/// name or a code string holding a secret) is never recorded.
#[tokio::test]
async fn each_refused_op_records_its_kind_and_never_the_members_text() {
    use crate::infrastructure::persistence::audit_log::AuditLog;
    let base = tempfile::tempdir().unwrap();
    let event_log = AuditLog::open_sync(base.path(), "cli:refusals").unwrap();
    let board = crate::composition::swarm::swarm_board();
    assert!(
        board.record_in_session(true, &event_log),
        "the event log is on"
    );
    let directory = tempfile::tempdir().unwrap();
    let tool: SwarmTool =
        super::super::swarm_test_support::tool(Arc::new(directory.path().to_path_buf()), board);
    let before = swarm_ops(base.path()).len();
    let captured = CapturedLog::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    let requests = [
        format!(r#"{{"op":"run","code":"print('{SECRET}')"}}"#),
        format!(r#"{{"op":"{SECRET}"}}"#),
        format!(r#"{{"code":"print('{SECRET}')"}}"#),
        format!(r#"{{"op":3,"code":"print('{SECRET}')"}}"#),
    ];
    for request in &requests {
        let result = tool.execute(request).await.unwrap();
        assert!(result.is_error, "{request}: {}", result.content);
    }
    drop(guard);
    let log = captured.0.lock().unwrap().clone();
    let refusals: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("swarm op refused"))
        .collect();
    assert_eq!(refusals.len(), requests.len(), "{log}");
    for line in &refusals {
        assert!(line.contains("quecto::swarm_board"), "{line}");
    }
    assert!(refusals[0].contains("refusal=\"unknown_op\""), "{log}");
    assert!(refusals[0].contains("removed_workbench_op=true"), "{log}");
    assert!(refusals[1].contains("refusal=\"unknown_op\""), "{log}");
    assert!(refusals[1].contains("removed_workbench_op=false"), "{log}");
    assert!(refusals[2].contains("refusal=\"op_required\""), "{log}");
    assert!(refusals[2].contains("removed_workbench_op=false"), "{log}");
    assert!(refusals[3].contains("refusal=\"op_not_a_string\""), "{log}");
    assert!(refusals[3].contains("removed_workbench_op=false"), "{log}");
    assert!(!log.contains("sk-ant"), "a secret reached the log: {log}");
    assert!(
        !log.contains("print("),
        "member text reached the log: {log}"
    );
    let recorded = swarm_ops(base.path());
    let refused = &recorded[before..];
    assert_eq!(refused.len(), requests.len(), "{recorded:?}");
    for record in refused {
        assert_eq!(record["op"], "unknown", "{record}");
        assert_eq!(record["outcome"], "refused", "{record}");
        assert_eq!(record["kind"], "calling", "{record}");
        assert_eq!(record["actor_ref"], "coordinator", "{record}");
    }
    let written =
        std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:refusals")).unwrap();
    assert!(
        !written.contains("sk-ant"),
        "a secret reached the event log: {written}"
    );
    assert!(
        !written.contains("print("),
        "member text reached the event log: {written}"
    );
}

/// The `swarm_op` records in the event log under `base`.
fn swarm_ops(base: &std::path::Path) -> Vec<serde_json::Value> {
    let path =
        crate::infrastructure::persistence::audit_log::AuditLog::file_path(base, "cli:refusals");
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|line| line["event"] == "swarm_op")
        .collect()
}
