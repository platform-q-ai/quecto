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
    let mut child = sh("exec sleep 5").spawn().unwrap();
    let open = StderrTail::pump(
        &tokio::runtime::Handle::current(),
        child.stderr.take().unwrap(),
    );
    let started = std::time::Instant::now();
    open.wait_eof(Duration::from_millis(100)).await;
    assert!(started.elapsed() < Duration::from_secs(2));
    let _ = child.kill().await;
}

#[tokio::test]
async fn a_lossy_snapshot_stays_within_the_capacity_in_bytes() {
    // Each byte that is not UTF-8 becomes U+FFFD, three bytes: 64 of them
    // would be 192 bytes of text from a 64-byte tail.
    let mut child = sh("i=0; while [ $i -lt 100 ]; do printf '\\377' >&2; i=$((i+1)); done")
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let tail = StderrTail::pump_within(&tokio::runtime::Handle::current(), stderr, 64);
    tail.wait_eof(Duration::from_secs(5)).await;
    let _ = child.wait().await;
    let snapshot = tail.snapshot();
    assert!(!snapshot.is_empty());
    assert!(snapshot.len() <= 64, "{} bytes", snapshot.len());
    assert!(snapshot.chars().all(|c| c == char::REPLACEMENT_CHARACTER));
}

#[test]
fn tail_text_keeps_the_last_whole_characters_within_the_capacity() {
    assert_eq!(super::tail_text("abc".as_bytes(), 8), "abc");
    assert_eq!(
        super::tail_text(" \u{e9}\u{e9}\u{e9} ".as_bytes(), 5),
        "\u{e9}\u{e9}"
    );
    assert_eq!(super::tail_text(b"\xff\xff", 4), "\u{fffd}");
    assert_eq!(super::tail_text(b"", 4), "");
}
