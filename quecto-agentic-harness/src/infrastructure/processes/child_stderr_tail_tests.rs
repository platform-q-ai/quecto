use std::time::Duration;

use super::{STDERR_TAIL_CAPACITY, StderrTail};

fn sh(script: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("sh");
    command.arg("-c").arg(script);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::piped());
    command
}

async fn tail_of(script: &str) -> StderrTail {
    let mut child = sh(script).spawn().unwrap();
    let stderr = child.stderr.take().unwrap();
    let tail = StderrTail::pump(&tokio::runtime::Handle::current(), stderr);
    tail.wait_eof(Duration::from_secs(5)).await;
    let _ = child.wait().await;
    tail
}

#[tokio::test]
async fn retains_a_short_refusal_verbatim() {
    let tail =
        tail_of("echo 'agent: --persist is refused with --parent-control' >&2; exit 1").await;
    assert_eq!(
        tail.snapshot(),
        "agent: --persist is refused with --parent-control"
    );
}

#[tokio::test]
async fn keeps_only_the_last_capacity_bytes() {
    let tail = tail_of("head -c 20000 /dev/zero | tr '\\0' a >&2; echo END >&2").await;
    let text = tail.snapshot();
    assert!(text.len() <= STDERR_TAIL_CAPACITY, "{}", text.len());
    assert!(text.ends_with("END"), "the newest bytes survive");
    assert!(text.starts_with('a'), "the oldest bytes were dropped first");
}

#[tokio::test]
async fn silent_child_yields_an_empty_snapshot_and_wait_eof_is_bounded() {
    let tail = tail_of("exit 0").await;
    assert!(tail.snapshot().is_empty());
    // A pipe still held open cannot stall a report beyond the bound.
    let mut child = sh("sleep 5").spawn().unwrap();
    let open = StderrTail::pump(
        &tokio::runtime::Handle::current(),
        child.stderr.take().unwrap(),
    );
    let started = std::time::Instant::now();
    open.wait_eof(Duration::from_millis(100)).await;
    assert!(started.elapsed() < Duration::from_secs(2));
    let _ = child.kill().await;
}
