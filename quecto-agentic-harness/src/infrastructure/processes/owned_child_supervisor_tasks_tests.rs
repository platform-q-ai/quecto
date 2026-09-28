use std::sync::Arc;
use std::time::Duration;

use super::OwnedChildSupervisor;
use crate::infrastructure::processes::child_line_pipes::{LineLimits, StdoutLine};
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildExit, ProcessGroup, ProtocolOutcome, TerminationBudget,
};

const BOUND: Duration = Duration::from_secs(10);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn line_pumps_outlive_the_runtime_that_started_them() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let caller = runtime();
    // Started from inside the caller's runtime, as a launcher would be.
    let (handle, stdin, stdout) = caller.block_on(async {
        let mut cat = tokio::process::Command::new("cat");
        cat.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let spawned = supervisor.spawn(cat, ProcessGroup::Own).await.unwrap();
        let limits = LineLimits {
            line_cap: 64,
            buffer_bytes: 4096,
        };
        (
            spawned.handle,
            supervisor.pump_stdin_lines(spawned.stdin.unwrap(), 2),
            supervisor.pump_stdout_lines(spawned.stdout.unwrap(), limits),
        )
    });
    drop(caller);
    let mut stdout = stdout;
    runtime().block_on(async {
        tokio::time::timeout(BOUND, async {
            stdin.write_line("hello\n".into()).await.unwrap();
            assert_eq!(
                stdout.next().await,
                Some(StdoutLine::Line(b"hello\n".to_vec()))
            );
            stdin
                .write_line(format!("{}\n", "x".repeat(100)))
                .await
                .unwrap();
            assert_eq!(
                stdout.next().await,
                Some(StdoutLine::OverCap { bytes: 101 })
            );
            assert!(stdin.close(), "the first close closes");
            assert!(!stdin.close(), "a close is idempotent");
            assert_eq!(stdout.next().await, None, "cat ends at its input's end");
            assert_eq!(supervisor.wait_exit(handle).await, Some(ChildExit::Code(0)));
        })
        .await
        .expect("the pumps answer on the supervisor's runtime");
    });
    supervisor.retire(handle);
}

/// Everything a `tracing` subscriber writes, for asserting what reached it.
#[derive(Clone, Default)]
struct CapturedLog(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
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
}

const QUICK: TerminationBudget = TerminationBudget {
    exit_after_ack: Duration::from_millis(100),
    term_grace: Duration::from_secs(2),
    kill_grace: Duration::from_secs(2),
};

/// A plain [`OwnedChildSupervisor::request_termination`] (bash, container
/// and proxy callers) runs as it always has, under the supervisor's own
/// `tracing` dispatch: only an observed termination is routed through its
/// requester's subscriber (#2286 review round 3).
#[test]
fn a_plain_termination_request_keeps_the_supervisors_own_subscriber() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let logs = CapturedLog::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let handle = runtime().block_on(async {
        let mut sleep = tokio::process::Command::new("sleep");
        sleep
            .arg("30")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let spawned = supervisor.spawn(sleep, ProcessGroup::Own).await.unwrap();
        supervisor.request_termination(
            spawned.handle,
            Box::pin(async { ProtocolOutcome::Acknowledged }),
            QUICK,
        );
        let exit = tokio::time::timeout(BOUND, supervisor.wait_exit(spawned.handle))
            .await
            .expect("the fallback ends the child");
        assert_eq!(exit, Some(ChildExit::Signal(libc::SIGTERM)));
        // The termination logs once it has observed the reap.
        tokio::time::sleep(Duration::from_millis(300)).await;
        spawned.handle
    });
    let text = logs.text();
    assert!(
        !text.contains("owned child termination"),
        "a plain termination is not logged under its requester's subscriber: {text}"
    );
    supervisor.retire(handle);
}
