//! #2167: a command that leaves a background job holding its output returns
//! once the shell exits, and long output keeps its true end.
use super::*;

fn exec_with_capture(max_capture_bytes: usize) -> (ExecTool, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()));
    let options = ExecOptions {
        max_capture_bytes,
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(sandbox),
        options,
    );
    (tool, tmp)
}

/// A background job that inherits the output no longer holds the call
/// until it exits: the shell's own output comes back, with a note.
#[tokio::test]
async fn a_background_job_holding_the_output_does_not_hold_the_call() {
    let (tool, _tmp) = exec_with_capture(MAX_CAPTURE_BYTES);
    let started = std::time::Instant::now();
    let result = tool
        .execute(r#"{"command": "sleep 5 & echo started"}"#)
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(3), "held for {elapsed:?}");
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("started"), "{}", result.content);
    assert!(
        result.content.contains("background"),
        "the note says why output stopped: {}",
        result.content
    );
}

/// Output a background job writes after the shell exits is not waited for,
/// but everything written before is kept, stderr included.
#[tokio::test]
async fn output_written_before_the_shell_exits_is_kept() {
    let (tool, _tmp) = exec_with_capture(MAX_CAPTURE_BYTES);
    let result = tool
        .execute(r#"{"command": "(sleep 5; echo late) & echo early; echo warned >&2"}"#)
        .await
        .unwrap();
    assert!(result.content.contains("early"), "{}", result.content);
    assert!(result.content.contains("warned"), "{}", result.content);
    assert!(!result.content.contains("late"), "{}", result.content);
}

/// A command whose output closes when the shell exits gets no note.
#[tokio::test]
async fn an_ordinary_command_gets_no_background_note() {
    let (tool, _tmp) = exec_with_capture(MAX_CAPTURE_BYTES);
    let result = tool
        .execute(r#"{"command": "sleep 1 & wait; echo done"}"#)
        .await
        .unwrap();
    assert_eq!(result.content.trim(), "done");
}

/// Output past the capture cap keeps its true end: the last line printed is
/// the last line shown, and the dropped middle is named.
#[tokio::test]
async fn output_past_the_capture_cap_keeps_its_true_end() {
    let (tool, _tmp) = exec_with_capture(4096);
    let result = tool.execute(r#"{"command": "seq 1 5000"}"#).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    let last = result.content.trim_end().lines().last().unwrap_or_default();
    assert_eq!(last, "5000", "{}", result.content);
    assert!(result.content.starts_with("1\n2\n"), "{}", result.content);
    assert!(result.content.contains("omitted"), "{}", result.content);
}

/// The saved file of a capture that dropped its middle is not called the
/// full output; one that dropped nothing still is.
#[tokio::test]
async fn a_cut_capture_is_not_saved_as_the_full_output() {
    let (tool, _tmp) = exec_with_capture(100_000);
    let cut = tool.execute(r#"{"command": "seq 1 60000"}"#).await.unwrap();
    assert!(cut.content.contains("middle omitted"), "{}", cut.content);
    assert!(!cut.content.contains("Full output"), "{}", cut.content);
    let (tool, _tmp) = exec_with_capture(MAX_CAPTURE_BYTES);
    let whole = tool.execute(r#"{"command": "seq 1 60000"}"#).await.unwrap();
    assert!(whole.content.contains("Full output"), "{}", whole.content);
}

/// A command that times out still returns what it printed first, even when
/// a descendant outside the killed group holds the output open.
#[tokio::test]
async fn a_timed_out_command_keeps_what_it_printed() {
    let (tool, _tmp) = exec_with_capture(MAX_CAPTURE_BYTES);
    let result = tool
        .execute(r#"{"command": "echo before; setsid sleep 5 & sleep 30", "timeout": 1}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("timed out"), "{}", result.content);
    assert!(result.content.contains("before"), "{}", result.content);
}

// --- the bounded capture (in-memory AsyncRead, no real pipes) ---

#[tokio::test]
async fn test_read_stream_limited_keeps_valid_utf8_under_cap() {
    let data = b"hello world".to_vec();
    let (content, truncated) = read_stream_limited(&data[..], 1024).await;
    assert_eq!(content, "hello world");
    assert!(!truncated);
}

#[tokio::test]
async fn test_read_stream_limited_replaces_invalid_utf8_under_cap() {
    let data = [b'o', b'k', 0xFF, b'!'];
    let (content, truncated) = read_stream_limited(&data[..], 1024).await;
    assert_eq!(content, "ok�!");
    assert!(!truncated);
}

#[tokio::test]
async fn test_read_stream_limited_allows_exact_cap() {
    let data = [b'x'; 100];
    let (content, truncated) = read_stream_limited(&data[..], 100).await;
    assert_eq!(content.len(), 100);
    assert!(!truncated, "exactly at the cap should not truncate");
}

/// Past the cap, the first quarter and the last three quarters are kept and
/// the dropped middle is named (#2167).
#[tokio::test]
async fn test_read_stream_limited_keeps_start_and_end_over_cap() {
    let data: Vec<u8> = (0..1000u32)
        .flat_map(|i| format!("{i:04}\n").into_bytes())
        .collect();
    let (content, truncated) = read_stream_limited(&data[..], 100).await;
    assert!(truncated, "exceeding the cap should set truncated=true");
    assert!(content.starts_with("0000\n0001\n"), "{content}");
    assert!(content.ends_with("0998\n0999\n"), "{content}");
    let omitted = data.len() - 100;
    assert!(
        content.contains(&format!("[... {omitted} bytes of output omitted ...]")),
        "{content}"
    );
}

/// Every chunk shape keeps the same head and tail as one push would.
#[test]
fn a_capture_is_the_same_however_the_stream_is_chunked() {
    let data: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
    let mut whole = capture::Capture::new(1000);
    whole.push(&data);
    for size in [1, 7, 249, 250, 251, 999, 1000, 1001, 4999] {
        let mut chunked = capture::Capture::new(1000);
        for chunk in data.chunks(size) {
            chunked.push(chunk);
        }
        assert_eq!(chunked.render(), whole.render(), "chunk size {size}");
    }
}

#[tokio::test]
async fn test_read_stream_limited_empty_input() {
    let data: Vec<u8> = Vec::new();
    let (content, truncated) = read_stream_limited(&data[..], 100).await;
    assert!(content.is_empty());
    assert!(!truncated);
}

// --- awaiting a stream ---

#[tokio::test]
async fn test_await_stream_output_none() {
    let (s, t) = await_stream_output(None).await;
    assert!(s.is_empty());
    assert!(!t);
}

#[tokio::test]
async fn test_await_stream_output_some() {
    let handle = tokio::spawn(async { ("captured".to_string(), false) });
    let (s, t) = await_stream_output(Some(handle.into())).await;
    assert_eq!(s, "captured");
    assert!(!t);
}

#[tokio::test]
async fn test_await_stream_output_with_timeout_none() {
    let ((s, t), open) = await_stream_output_with_timeout(None, Duration::from_millis(50)).await;
    assert!(s.is_empty());
    assert!(!t);
    assert!(!open);
}

#[tokio::test]
async fn test_await_stream_output_with_timeout_completes() {
    let handle = tokio::spawn(async { ("done".to_string(), false) });
    let ((s, _), open) =
        await_stream_output_with_timeout(Some(handle.into()), Duration::from_secs(5)).await;
    assert_eq!(s, "done");
    assert!(!open);
}

/// A stream still open when the wait runs out gives what it captured so
/// far, and says it was still open.
#[tokio::test]
async fn a_stream_still_open_gives_what_arrived_so_far() {
    let (mut writer, reader) = tokio::io::duplex(64);
    tokio::io::AsyncWriteExt::write_all(&mut writer, b"partial")
        .await
        .unwrap();
    let reader = capture::StreamReader::spawn(reader, 1024);
    let ((s, cut), open) =
        await_stream_output_with_timeout(Some(reader), Duration::from_millis(100)).await;
    assert_eq!(s, "partial");
    assert!(!cut);
    assert!(open, "the writer is still held");
    drop(writer);
}
