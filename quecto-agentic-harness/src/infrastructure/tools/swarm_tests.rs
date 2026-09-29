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
/// its refusal kind, and whether it named a removed workbench op; the
/// member's text (an op name or a code string holding a secret) is never
/// recorded.
#[tokio::test]
async fn each_refused_op_records_its_kind_and_never_the_members_text() {
    let directory = tempfile::tempdir().unwrap();
    let tool: SwarmTool = super::super::swarm_test_support::tool(
        Arc::new(directory.path().to_path_buf()),
        crate::composition::swarm::swarm_board(),
    );
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
    assert!(!log.contains("sk-ant"), "a secret reached the log: {log}");
    assert!(
        !log.contains("print("),
        "member text reached the log: {log}"
    );
}
