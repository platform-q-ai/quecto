use std::sync::Arc;
use std::time::Duration;

use super::OwnedChildSupervisor;
use crate::infrastructure::processes::child_line_pipes::{LineLimits, StdoutLine};
use crate::infrastructure::processes::owned_child_supervisor::{ChildExit, ProcessGroup};

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
