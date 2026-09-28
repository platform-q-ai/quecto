//! A claude member process's telemetry (#2286): structured `tracing`
//! events under [`super::TELEMETRY_TARGET`] at launch, start, input close,
//! skipped line, termination and exit — and never a credential, a proxy
//! value, an inline JSON argument or stderr's content.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::test_rig::{BOUND, FALLBACK_BOUND, RESULT_LINE, Rig, finish, until_retired};
use crate::application::external_agent::dto::ExternalAgentExit;

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = CapturedLog;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl CapturedLog {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    /// The captured lines of `target` naming `message`.
    fn lines_of(&self, target: &str, message: &str) -> Vec<String> {
        self.text()
            .lines()
            .filter(|line| line.contains(target) && line.contains(message))
            .map(str::to_string)
            .collect()
    }

    /// The first line of the external-agent target naming `message`.
    async fn until_logged(&self, message: &str) -> String {
        self.until_logged_under(super::TELEMETRY_TARGET, message)
            .await
    }

    async fn until_logged_under(&self, target: &str, message: &str) -> String {
        tokio::time::timeout(BOUND, async {
            loop {
                if let Some(line) = self.lines_of(target, message).into_iter().next() {
                    return line;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{message:?} is logged under {target}: {}", self.text()))
    }

    /// The supervisor's record of the member's termination: logged by the
    /// supervisor itself from the launcher's data-only observation, never
    /// by a closure of the launcher's run on the supervisor's thread.
    async fn termination(&self) -> String {
        let line = self
            .until_logged_under(SUPERVISOR_TARGET, "owned child termination observed")
            .await;
        for expected in ["owner=claude member", "label=m1", "elapsed_ms="] {
            assert!(line.contains(expected), "{expected}: {line}");
        }
        line
    }
}

/// The `tracing` target of the supervisor's own telemetry.
const SUPERVISOR_TARGET: &str = "quecto::owned_child";

fn capture() -> (CapturedLog, tracing::subscriber::DefaultGuard) {
    let logs = CapturedLog::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (logs, guard)
}

const PROXY: &str = "http://proxyuser:proxy-secret@proxy.invalid:3128";

/// Values that must never reach a log line.
const NEVER_LOGGED: &[&str] = &[
    "member-token",
    "proxy-secret",
    "proxyuser",
    "mcpServers",
    "hooks",
    "stderr-words",
];

fn assert_nothing_secret(logs: &CapturedLog) {
    let text = logs.text();
    for value in NEVER_LOGGED {
        assert!(!text.contains(value), "{value:?} was logged: {text}");
    }
}

#[tokio::test]
async fn a_member_run_is_logged_from_launch_to_exit_without_secrets() {
    let (logs, _guard) = capture();
    let long = format!("{{\"type\": \"system\", \"pad\": \"{}\"}}", "x".repeat(400));
    let scenario = format!("@stderr stderr-words\n{long}\n{RESULT_LINE}");
    let rig = Rig::build(
        scenario.as_bytes(),
        &[("HTTPS_PROXY", PROXY), ("ALL_PROXY", PROXY), ("TZ", "UTC")],
        |launcher| launcher.with_stream_limits(256, 4096),
    );
    let process = rig.start().await;
    process.send_user_turn("go").await.unwrap();
    while let Some(event) = tokio::time::timeout(BOUND, process.next_event())
        .await
        .expect("an event is bounded")
    {
        if matches!(
            event,
            crate::domain::external_agent::stream::ExternalAgentEvent::Result(_)
        ) {
            break;
        }
    }
    assert_eq!(finish(process.as_ref()).await, ExternalAgentExit::Code(0));
    drop(process);
    let launch = logs.until_logged("claude member launch").await;
    for expected in [
        "member=m1",
        "--permission-mode",
        "--mcp-config",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "claude-config",
    ] {
        assert!(launch.contains(expected), "{expected}: {launch}");
    }
    let pid = rig.mock.recorded("pid").remove(0);
    let started = logs.until_logged("claude member process started").await;
    assert!(started.contains(&format!("pid={pid}")), "{started}");
    assert!(started.contains(&format!("pgid={pid}")), "{started}");
    logs.until_logged("claude member stdin closed").await;
    let skipped = logs.until_logged("claude member line skipped").await;
    assert!(skipped.contains("OverCap"), "{skipped}");
    assert!(
        skipped.contains(&format!("bytes={}", long.len() + 1)),
        "{skipped}"
    );
    let exit = logs.until_logged("claude member exit").await;
    for expected in ["status=0", "wall_ms=", "stderr_tail_bytes="] {
        assert!(exit.contains(expected), "{expected}: {exit}");
    }
    logs.termination().await;
    until_retired(&rig.supervisor, FALLBACK_BOUND).await;
    assert_nothing_secret(&logs);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_member_ended_by_kill_logs_the_signal() {
    let (logs, _guard) = capture();
    let rig = Rig::build(b"@stubborn\n", &[("HTTP_PROXY", PROXY)], |launcher| {
        launcher
    });
    let process = rig.start().await;
    drop(process);
    until_retired(&rig.supervisor, FALLBACK_BOUND).await;
    let termination = logs.termination().await;
    assert!(termination.contains("signal=KILL"), "{termination}");
    assert_nothing_secret(&logs);
}

/// Output discarded by `exited_discarding_output` is counted in the exit's
/// telemetry: the decoded events never returned as well as the lines never
/// decoded (#2286 review round 3).
#[tokio::test]
async fn the_exit_counts_the_decoded_events_and_the_lines_it_discarded() {
    let (logs, _guard) = capture();
    let blocks = (0..3)
        .map(|n| format!("{{\"type\": \"text\", \"text\": \"block {n}\"}}"))
        .collect::<Vec<_>>()
        .join(", ");
    let scenario = format!(
        "{{\"type\": \"assistant\", \"message\": {{\"id\": \"msg-1\", \"content\": [{blocks}]}}}}\n{RESULT_LINE}"
    );
    let rig = Rig::new(&scenario);
    let process = rig.start().await;
    process.send_user_turn("go").await.unwrap();
    let first = tokio::time::timeout(BOUND, process.next_event())
        .await
        .expect("an event is bounded");
    assert!(
        matches!(
            first,
            Some(crate::domain::external_agent::stream::ExternalAgentEvent::AssistantBlock { .. })
        ),
        "{first:?}"
    );
    // Two of the line's three blocks are decoded and pending; the result
    // line is not read yet. The input closes once the discard is under way.
    let (exit, ()) = tokio::join!(
        tokio::time::timeout(BOUND, process.exited_discarding_output()),
        async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            process.close_input().await;
        }
    );
    assert_eq!(exit.expect("exit is bounded"), ExternalAgentExit::Code(0));
    let exit = logs.until_logged("claude member exit").await;
    for expected in ["discarded_events=2", "discarded_lines=1"] {
        assert!(exit.contains(expected), "{expected}: {exit}");
    }
}
