use super::*;
use crate::infrastructure::warn_capture::{
    install_warn_capture, oversized_outbound_warn_count, oversized_warn_count,
};
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_outbound_command_emits_warning_and_keeps_writer_alive() {
    let (listener, socket_path, _dir) = bind_test_socket("oversized-outbound-warn-test");

    let (warnings, _guard) = install_warn_capture();
    let oversized_message = "x".repeat(MAX_LINE_BYTES + 1);

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // Consume the framed-mode hello.
        let mut len = [0_u8; 4];
        tokio::io::AsyncReadExt::read_exact(&mut stream, &mut len)
            .await
            .unwrap();
        assert_eq!(u32::from_be_bytes(len), 0);

        tokio::io::AsyncReadExt::read_exact(&mut stream, &mut len)
            .await
            .unwrap();
        let len = u32::from_be_bytes(len) as usize;
        let mut payload = vec![0_u8; len];
        tokio::io::AsyncReadExt::read_exact(&mut stream, &mut payload)
            .await
            .unwrap();
        assert!(
            std::str::from_utf8(&payload)
                .unwrap()
                .contains(r#""type":"get_state""#),
            "writer must keep running after dropping the oversized command"
        );
    });

    let mut client = Client::connect(&socket_path).await.unwrap();
    client
        .send(&Command::Prompt {
            id: None,
            message: oversized_message,
            streaming_behavior: None,
        })
        .await
        .unwrap();
    client.send(&Command::GetState { id: None }).await.unwrap();

    server.await.unwrap();
    let captured = warnings.lock().unwrap();
    assert_eq!(
        oversized_outbound_warn_count(&captured),
        1,
        "dropping one oversized outbound command must emit exactly one tracing::warn! (#1125); \
         captured events: {captured:?}"
    );
}

#[tokio::test]
async fn client_connect_handles_line_just_under_cap() {
    let (listener, socket_path, _dir) = bind_test_socket("under-cap-client-test");

    let (warnings, _guard) = install_warn_capture();

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

    {
        let captured = warnings.lock().unwrap();
        assert!(
            captured.is_empty(),
            "an in-bounds frame must not emit any WARN-or-worse tracing event (#1112); \
             captured events: {captured:?}"
        );
    }

    server.await.unwrap();
}

#[tokio::test]
async fn client_connect_drops_line_exactly_at_cap() {
    let (listener, socket_path, _dir) = bind_test_socket("at-cap-client-test");

    let (warnings, _guard) = install_warn_capture();

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

    {
        let captured = warnings.lock().unwrap();
        // The smallest frame that must be dropped is also the smallest frame
        // that must warn (#1112).
        assert_eq!(
            oversized_warn_count(&captured),
            1,
            "the at-cap boundary drop must emit exactly one tracing::warn! (#1112); \
             captured events: {captured:?}"
        );
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

/// #1112: dropping an oversized inbound frame must emit `tracing::warn!` (in
/// addition to the #1047 counter), and the warning must reach a subscriber
/// that is only installed as the *thread-scoped* default on the connecting
/// thread. The reader runs in a spawned task on another worker thread, so
/// this passes only if the client propagates the connect-time dispatcher into
/// the reader task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_event_drop_emits_warning_through_propagated_dispatcher() {
    let (listener, socket_path, _dir) = bind_test_socket("oversized-drop-warned-test");

    // Thread-scoped only — deliberately NOT a global default, so a warning is
    // observable solely via dispatcher propagation into the spawned reader.
    let (warnings, _guard) = install_warn_capture();

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
    // Receiving the event queued after the oversized frame proves the reader
    // has already taken the oversized branch — no sleeps, no race.
    match tokio::time::timeout(std::time::Duration::from_secs(2), client.recv())
        .await
        .unwrap()
    {
        Some(Event::Token { token }) => assert_eq!(token, "after"),
        other => panic!("expected the event after the oversized frame, got {other:?}"),
    }

    {
        let captured = warnings.lock().unwrap();
        // Exactly one WARN for exactly one oversized frame: `count == 1`
        // rules out a warn-at-connect or warn-on-every-frame implementation,
        // and the exact-level match rules out an `error!` regression.
        assert_eq!(
            oversized_warn_count(&captured),
            1,
            "dropping one oversized inbound frame must emit exactly one tracing::warn! \
             visible to the connect-time dispatcher (#1112); captured events: {captured:?}"
        );
    }

    server.await.unwrap();
}

/// #1112: the normal v2 framed path must warn and recover too, not only the
/// legacy NDJSON compatibility path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_framed_event_warns_and_recovers() {
    let (listener, socket_path, _dir) = bind_test_socket("oversized-framed-warn-test");
    let (warnings, _guard) = install_warn_capture();
    let (oversized, _) = token_frame_of_len(MAX_LINE_BYTES + 1);
    let valid = br#"{"type":"token","token":"after"}"#;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream
            .write_all(&(oversized.len() as u32).to_be_bytes())
            .await
            .unwrap();
        stream.write_all(oversized.as_bytes()).await.unwrap();
        stream
            .write_all(&(valid.len() as u32).to_be_bytes())
            .await
            .unwrap();
        stream.write_all(valid).await.unwrap();
        stream.flush().await.unwrap();
    });

    let mut client = Client::connect(&socket_path).await.unwrap();
    match tokio::time::timeout(std::time::Duration::from_secs(2), client.recv())
        .await
        .unwrap()
    {
        Some(Event::Token { token }) => assert_eq!(token, "after"),
        other => panic!("expected framed event after oversized frame, got {other:?}"),
    }
    assert_eq!(oversized_warn_count(&warnings.lock().unwrap()), 1);
    server.await.unwrap();
}

/// #1112: the warning is per-drop (matching quecto-api's UDS client), not
/// once-per-connection — three oversized frames must produce three warnings.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_oversized_drops_emit_one_warning_each() {
    let (listener, socket_path, _dir) = bind_test_socket("oversized-drop-warn-each-test");

    let (warnings, _guard) = install_warn_capture();

    let (frame, _) = token_frame_of_len(MAX_LINE_BYTES + 65_536);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        for _ in 0..3 {
            stream.write_all(frame.as_bytes()).await.unwrap();
            stream.write_all(b"\n").await.unwrap();
        }
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
        other => panic!("expected the event after the oversized frames, got {other:?}"),
    }

    {
        let captured = warnings.lock().unwrap();
        assert_eq!(
            oversized_warn_count(&captured),
            3,
            "each dropped oversized frame must emit its own warning (#1112); \
             captured events: {captured:?}"
        );
    }

    server.await.unwrap();
}

/// #1112 no-stderr policy: the client must never install a tracing subscriber
/// itself — with no dispatcher installed by the embedder, driving an
/// oversized drop must leave the process's global dispatcher unset, so the
/// warn is a no-op on a raw-mode terminal instead of smearing stderr.
#[tokio::test]
async fn client_installs_no_global_subscriber() {
    let (listener, socket_path, _dir) = bind_test_socket("no-global-subscriber-test");

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

    // Observed from a fresh thread (which has no thread-scoped default), the
    // dispatcher falls back to the process-global default — which must still
    // be the no-op subscriber. (`dispatcher::has_been_set()` cannot be used
    // here: it also flips on thread-scoped `set_default`, which the client's
    // own dispatcher propagation performs.)
    let global_is_noop = std::thread::spawn(|| {
        tracing::dispatcher::get_default(|dispatch| {
            dispatch.is::<tracing::subscriber::NoSubscriber>()
        })
    })
    .join()
    .unwrap();
    assert!(
        global_is_noop,
        "the TUI client must never install a global tracing subscriber; the \
         oversized-drop warn must stay a no-op unless the embedder installs one (#1112)"
    );

    server.await.unwrap();
}
