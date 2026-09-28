use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use super::{LineLimits, LineWriteError, MIN_LINE_COST, StdinLines, StdoutLine};
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildExit, ChildHandleId, OwnedChildSupervisor, ProcessGroup, ProtocolOutcome,
    TerminationBudget,
};

const BOUND: Duration = Duration::from_secs(20);

#[test]
fn limits_are_valid_only_when_a_line_fits_the_budget() {
    let valid = |line_cap, buffer_bytes| {
        LineLimits {
            line_cap,
            buffer_bytes,
        }
        .valid()
    };
    assert!(valid(1024, 4096));
    assert!(valid(MIN_LINE_COST, MIN_LINE_COST));
    assert!(!valid(0, 4096), "a line holds at least one byte");
    assert!(
        !valid(8192, 4096),
        "a line longer than the budget never fits"
    );
    assert!(!valid(16, 128), "the budget holds at least one entry");
    assert!(
        !valid(1024, usize::try_from(u64::from(u32::MAX) + 1).unwrap()),
        "the budget is one acquisition"
    );
}

#[test]
fn each_line_costs_its_bytes_at_least_the_minimum_at_most_the_budget() {
    let limits = LineLimits {
        line_cap: 4096,
        buffer_bytes: 4096,
    };
    let cost = |line: StdoutLine| line.cost(limits) as usize;
    assert_eq!(cost(StdoutLine::OverCap { bytes: 1 << 30 }), MIN_LINE_COST);
    assert_eq!(cost(StdoutLine::Line(Vec::new())), MIN_LINE_COST);
    assert_eq!(cost(StdoutLine::Line(Vec::with_capacity(2000))), 2000);
    assert_eq!(cost(StdoutLine::Line(Vec::with_capacity(9000))), 4096);
}

/// A child running `script` under `sh` (with `$0`, `$1`, … set to `args`)
/// whose stdin is pumped `queue` lines ahead.
async fn stdin_of(
    supervisor: &Arc<OwnedChildSupervisor>,
    script: &str,
    args: &[&Path],
    queue: usize,
) -> (ChildHandleId, StdinLines) {
    let mut sh = tokio::process::Command::new("sh");
    sh.arg("-c")
        .arg(script)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let spawned = supervisor.spawn(sh, ProcessGroup::Own).await.unwrap();
    let stdin = supervisor.pump_stdin_lines(spawned.stdin.unwrap(), queue);
    (spawned.handle, stdin)
}

/// Whether `future` is still pending after a short while (it is polled,
/// and so started, meanwhile).
async fn still_pending<F: std::future::Future + Unpin>(future: &mut F) -> bool {
    tokio::time::timeout(Duration::from_millis(100), future)
        .await
        .is_err()
}

/// Shell that holds the child, reading nothing, until the test creates
/// the marker file `$0`.
const HELD_UNTIL_MARKER: &str = "while [ ! -e \"$0\" ]; do sleep 0.01; done";

/// Let a child held by [`HELD_UNTIL_MARKER`] go on.
fn release(marker: &Path) {
    std::fs::write(marker, b"").expect("the marker is written");
}

/// A line far larger than a pipe holds: its write blocks until the child
/// reads.
fn big_line() -> String {
    format!("{}\n", "x".repeat(1 << 20))
}

/// A write that failed ends the input: a sender whose line was queued
/// behind it learns of the failure even when the input is closed before
/// it looks (#2286 review round 3).
#[tokio::test]
async fn a_line_lost_to_a_failed_write_is_failed_even_after_a_close() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("release");
    // Held until released, the child then closes its stdin without
    // reading: the stuck write fails.
    let (handle, stdin) = stdin_of(
        &supervisor,
        &format!("{HELD_UNTIL_MARKER}; exec 0<&-; exec sleep 30"),
        &[&marker],
        2,
    )
    .await;
    let mut taken = Box::pin(stdin.write_line(big_line()));
    let mut queued = Box::pin(stdin.write_line("queued\n".into()));
    assert!(still_pending(&mut taken).await, "the big line is stuck");
    assert!(still_pending(&mut queued).await, "the small line is queued");
    release(&marker);
    let failed = tokio::time::timeout(BOUND, taken).await.expect("bounded");
    assert!(
        matches!(failed, Err(LineWriteError::Failed(_))),
        "{failed:?}"
    );
    assert!(stdin.close(), "the input closes after the failure");
    let lost = tokio::time::timeout(BOUND, queued).await.expect("bounded");
    assert!(
        matches!(lost, Err(LineWriteError::Failed(_))),
        "the queued line was lost to the failed write, not to the close: {lost:?}"
    );
    let outcome = supervisor
        .terminate(
            handle,
            async { ProtocolOutcome::Negative("test over".into()) },
            TerminationBudget::DEFAULT,
        )
        .await;
    assert!(supervisor.wait_exit(handle).await.is_some(), "{outcome:?}");
    supervisor.retire(handle);
}

/// Closing never waits on a write: a sender waiting for queue space is
/// answered `Closed` at once, the line already taken is written whole, and
/// the queued line is answered `Closed` and never written.
#[tokio::test]
async fn a_close_answers_waiting_and_queued_senders_and_writes_the_taken_line_whole() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("release");
    let out = root.path().join("read");
    // Held, reading nothing, until released after the close is answered.
    let (handle, stdin) = stdin_of(
        &supervisor,
        &format!("{HELD_UNTIL_MARKER}; exec cat > \"$1\""),
        &[&marker, &out],
        1,
    )
    .await;
    let big = big_line();
    let mut taken = Box::pin(stdin.write_line(big.clone()));
    let mut queued = Box::pin(stdin.write_line("queued\n".into()));
    let mut waiting = Box::pin(stdin.write_line("waiting\n".into()));
    assert!(still_pending(&mut taken).await, "the big line is stuck");
    assert!(
        still_pending(&mut queued).await,
        "the queue's one slot is used"
    );
    assert!(still_pending(&mut waiting).await, "no queue space is left");
    assert!(stdin.close(), "the first close closes");
    let answered = tokio::time::timeout(Duration::from_millis(300), waiting)
        .await
        .expect("a sender waiting for space is answered at once");
    assert_eq!(answered, Err(LineWriteError::Closed));
    assert!(
        still_pending(&mut taken).await,
        "the taken line is still stuck behind the held child"
    );
    release(&marker);
    let written = tokio::time::timeout(BOUND, taken).await.expect("bounded");
    assert_eq!(written, Ok(()), "the taken line is written whole");
    let dropped = tokio::time::timeout(BOUND, queued).await.expect("bounded");
    assert_eq!(dropped, Err(LineWriteError::Closed));
    let exit = tokio::time::timeout(BOUND, supervisor.wait_exit(handle))
        .await
        .expect("the child ends at its input's end");
    assert_eq!(exit, Some(ChildExit::Code(0)));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), big);
    supervisor.retire(handle);
}

/// Senders racing a close: each is answered, the child reads exactly the
/// lines answered `Ok`, each whole, and none answered `Closed`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn senders_racing_a_close_are_each_answered_and_only_their_ok_lines_are_read() {
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("read");
    let (handle, stdin) = stdin_of(&supervisor, "exec cat > \"$0\"", &[&out], 4).await;
    let stdin = Arc::new(stdin);
    let senders: Vec<_> = (0..64)
        .map(|n| {
            let stdin = Arc::clone(&stdin);
            let line = format!("line-{n}-{}\n", "y".repeat(4096));
            tokio::spawn(async move { (line.clone(), stdin.write_line(line).await) })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(2)).await;
    stdin.close();
    let mut answered_ok = Vec::new();
    for sender in senders {
        let (line, answer) = tokio::time::timeout(BOUND, sender)
            .await
            .expect("every sender is answered")
            .unwrap();
        match answer {
            Ok(()) => answered_ok.push(line),
            Err(LineWriteError::Closed) => {}
            Err(failed) => panic!("no write fails here: {failed:?}"),
        }
    }
    let exit = tokio::time::timeout(BOUND, supervisor.wait_exit(handle))
        .await
        .expect("the child ends at its input's end");
    assert_eq!(exit, Some(ChildExit::Code(0)));
    let read = std::fs::read_to_string(&out).unwrap();
    let mut read: Vec<String> = read.split_inclusive('\n').map(str::to_string).collect();
    read.sort();
    answered_ok.sort();
    assert_eq!(
        read, answered_ok,
        "the child read exactly the lines answered Ok"
    );
    supervisor.retire(handle);
}
