use super::*;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn oversized_line_read_keeps_buffer_bounded_and_resumes_at_next_line() {
    let oversized = vec![b'x'; MAX_LINE_BYTES + 65_536];
    let input = [
        oversized.as_slice(),
        b"\n{\"type\":\"token\",\"token\":\"after\"}\n".as_slice(),
    ]
    .concat();
    let mut reader = tokio::io::BufReader::new(input.as_slice());
    let mut line = String::new();

    let bytes_read = read_bounded_line(&mut reader, &mut line).await.unwrap();
    assert!(
        bytes_read > MAX_LINE_BYTES,
        "oversized frame should be consumed"
    );
    let capacity_after_oversized = line.capacity();

    assert!(
        capacity_after_oversized <= MAX_LINE_BYTES + 4096,
        "oversized frame must not inflate the line buffer beyond the protocol cap plus a small constant; capacity was {capacity_after_oversized}"
    );

    let bytes_read = read_bounded_line(&mut reader, &mut line).await.unwrap();
    assert!(
        bytes_read > 0,
        "reader should resume at the next framed event"
    );
    let event: Event = serde_json::from_str(line.trim()).unwrap();
    match event {
        Event::Token { token } => assert_eq!(token, "after"),
        other => panic!("unexpected event after oversized line: {other:?}"),
    }
}

#[tokio::test]
async fn client_connect_handles_line_just_under_cap() {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-under-cap-client-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    let token_prefix = r#"{"type":"token","token":""#;
    let token_suffix = r#""}"#;
    let token_len = MAX_LINE_BYTES - token_prefix.len() - token_suffix.len() - 1;
    let mut frame = String::with_capacity(MAX_LINE_BYTES);
    frame.push_str(token_prefix);
    frame.push_str(&"a".repeat(token_len));
    frame.push_str(token_suffix);
    assert_eq!(frame.len(), MAX_LINE_BYTES - 1);

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
        Some(Event::Token { token }) => assert_eq!(token.len(), token_len),
        other => panic!("line just under cap should be handled normally, got {other:?}"),
    }

    server.await.unwrap();
}
