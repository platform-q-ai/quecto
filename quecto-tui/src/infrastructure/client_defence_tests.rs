use super::*;
use tokio::io::AsyncWriteExt;

/// Guard that removes a test's temp dir (and its socket) on drop, so reruns
/// under a recycled PID never hit a stale socket file.
struct TempDirGuard(std::path::PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bind a fresh Unix socket under a per-test temp dir. Returns the listener,
/// the socket path, and a guard that cleans the dir up on drop.
fn bind_test_socket(name: &str) -> (tokio::net::UnixListener, std::path::PathBuf, TempDirGuard) {
    let dir = std::env::temp_dir().join(format!("quecto-tui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    (listener, socket_path, TempDirGuard(dir))
}

/// Build a well-formed token event frame (without trailing newline) whose
/// total length is exactly `frame_len` bytes.
fn token_frame_of_len(frame_len: usize) -> (String, String) {
    let token_prefix = r#"{"type":"token","token":""#;
    let token_suffix = r#""}"#;
    let token_len = frame_len - token_prefix.len() - token_suffix.len();
    let token: String = (0..token_len)
        .map(|idx| char::from(b'a' + (idx % 26) as u8))
        .collect();
    let mut frame = String::with_capacity(frame_len);
    frame.push_str(token_prefix);
    frame.push_str(&token);
    frame.push_str(token_suffix);
    assert_eq!(frame.len(), frame_len);
    (frame, token)
}

#[tokio::test]
async fn oversized_line_read_keeps_buffer_bounded_and_resumes_at_next_line() {
    let oversized = vec![b'x'; MAX_LINE_BYTES + 65_536];
    let input = [
        oversized.as_slice(),
        b"\n{\"type\":\"token\",\"token\":\"after\"}\n".as_slice(),
    ]
    .concat();
    let mut reader = tokio::io::BufReader::new(input.as_slice());
    let mut line = Vec::new();

    let read = quecto_line_io::read_bounded_line_into(&mut reader, &mut line, MAX_LINE_BYTES)
        .await
        .unwrap()
        .expect("oversized line");
    assert!(
        read.bytes_read > MAX_LINE_BYTES,
        "oversized frame should be consumed"
    );
    let capacity_after_oversized = line.capacity();

    assert!(
        capacity_after_oversized <= MAX_LINE_BYTES + 4096,
        "oversized frame must not inflate the line buffer beyond the protocol cap plus a small constant; capacity was {capacity_after_oversized}"
    );

    let read = quecto_line_io::read_bounded_line_into(&mut reader, &mut line, MAX_LINE_BYTES)
        .await
        .unwrap()
        .expect("next line");
    assert!(
        read.bytes_read > 0,
        "reader should resume at the next framed event"
    );
    assert!(
        line.capacity() <= 64 * 1024,
        "the next read must reclaim the buffer inflated by the oversized frame; capacity was {}",
        line.capacity()
    );
    let event: Event = serde_json::from_str(std::str::from_utf8(&line).unwrap().trim()).unwrap();
    match event {
        Event::Token { token } => assert_eq!(token, "after"),
        other => panic!("unexpected event after oversized line: {other:?}"),
    }
}

#[tokio::test]
async fn client_connect_drops_invalid_utf8_instead_of_normalizing_it() {
    let (listener, socket_path, _dir) = bind_test_socket("invalid-utf8-client-test");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream
            .write_all(b"{\"type\":\"token\",\"token\":\"")
            .await
            .unwrap();
        stream.write_all(&[0xff]).await.unwrap();
        stream.write_all(b"\"}\n").await.unwrap();
        stream
            .write_all(b"{\"type\":\"token\",\"token\":\"after\"}\n")
            .await
            .unwrap();
        stream.flush().await.unwrap();
    });

    let mut client = Client::connect(&socket_path).await.unwrap();
    match tokio::time::timeout(std::time::Duration::from_secs(2), client.recv())
        .await
        .unwrap()
    {
        Some(Event::Token { token }) => assert_eq!(token, "after"),
        other => panic!("invalid UTF-8 frame should be dropped before later event, got {other:?}"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn client_connect_handles_line_just_under_cap() {
    let (listener, socket_path, _dir) = bind_test_socket("under-cap-client-test");

    // Content of MAX-1 bytes; with the newline the line is exactly MAX bytes —
    // the largest frame the client accepts.
    let (frame, expected_token) = token_frame_of_len(MAX_LINE_BYTES - 1);

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(frame.as_bytes()).await.unwrap();
        stream.write_all(b"\n").await.unwrap();
        stream.flush().await.unwrap();
    });

    let mut client = Client::connect(&socket_path).await.unwrap();
    match tokio::time::timeout(std::time::Duration::from_secs(2), client.recv())
        .await
        .unwrap()
    {
        Some(Event::Token { token }) => assert_eq!(token, expected_token),
        other => panic!("line just under cap should be handled normally, got {other:?}"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn client_connect_drops_line_exactly_at_cap() {
    let (listener, socket_path, _dir) = bind_test_socket("at-cap-client-test");

    // Content of exactly MAX bytes: with the newline the line is one byte over
    // the cap — the first frame that must be dropped. Pins the flip point so
    // an off-by-one in read_bounded_line_into's capacity/newline handling cannot
    // land silently between the accepted MAX-1 case and the +64KiB case.
    let (frame, _) = token_frame_of_len(MAX_LINE_BYTES);

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(frame.as_bytes()).await.unwrap();
        stream.write_all(b"\n").await.unwrap();
        stream
            .write_all(b"{\"type\":\"token\",\"token\":\"after\"}\n")
            .await
            .unwrap();
        stream.flush().await.unwrap();
    });

    let mut client = Client::connect(&socket_path).await.unwrap();
    match tokio::time::timeout(std::time::Duration::from_secs(2), client.recv())
        .await
        .unwrap()
    {
        Some(Event::Token { token }) => {
            assert_eq!(
                token, "after",
                "at-cap frame must be dropped, not delivered"
            );
        }
        other => panic!("expected the event after the at-cap frame, got {other:?}"),
    }

    server.await.unwrap();
}

/// #1047 AC4: dropped oversized event lines must be COUNTED so the UI can
/// surface the loss — near a full context window `turn_end`/`agent_end` can
/// exceed the cap, and a silent drop makes the session look frozen.
#[tokio::test]
async fn oversized_event_drop_is_recorded_for_ui_surfacing() {
    let (listener, socket_path, _dir) = bind_test_socket("oversized-drop-counted-test");

    let (frame, _) = token_frame_of_len(MAX_LINE_BYTES + 65_536);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(frame.as_bytes()).await.unwrap();
        stream.write_all(b"\n").await.unwrap();
        stream
            .write_all(b"{\"type\":\"token\",\"token\":\"after\"}\n")
            .await
            .unwrap();
        stream.flush().await.unwrap();
    });

    let mut client = Client::connect(&socket_path).await.unwrap();
    match tokio::time::timeout(std::time::Duration::from_secs(2), client.recv())
        .await
        .unwrap()
    {
        Some(Event::Token { token }) => assert_eq!(token, "after"),
        other => panic!("expected the event after the oversized frame, got {other:?}"),
    }

    assert_eq!(
        client.dropped_oversized_events(),
        1,
        "the client must record the dropped oversized event line so the UI can surface it (#1047)"
    );
    server.await.unwrap();
}
