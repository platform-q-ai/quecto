//! #1059 deprecation-window interop tests for the legacy-NDJSON client path.
use super::*;
use tokio::io::AsyncWriteExt;

/// #1059 deprecation-window interop: a client that connected via
/// `connect_legacy` (agent announced no protocol v2) MUST write commands as
/// legacy NDJSON lines and MUST NOT emit the empty hello frame or
/// length-prefixed frames. The first byte on the wire is the JSON opener `{`
/// (an empty hello frame would begin with 0x00), and the command line ends
/// with a newline. Pins the `mode == Framed` guard on the hello + the legacy
/// write path, so misspeaking frames at a legacy agent is caught.
#[tokio::test]
async fn client_connect_legacy_writes_newline_commands_not_frames() {
    let dir = std::env::temp_dir().join(format!("quecto-tui-client-legacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        // One legacy NDJSON event, so the client's dual-accepting reader still
        // works over a legacy socket.
        write_half
            .write_all(br#"{"type":"token","token":"from-server"}"#)
            .await
            .unwrap();
        write_half.write_all(b"\n").await.unwrap();
        write_half.flush().await.unwrap();

        use tokio::io::{AsyncBufReadExt, AsyncReadExt};
        let mut reader = tokio::io::BufReader::new(read_half);
        let first = reader.read_u8().await.unwrap();
        assert_eq!(
            first, b'{',
            "legacy client must write NDJSON, not a length-prefixed frame (got byte {first:#04x})"
        );
        let mut rest = String::new();
        reader.read_line(&mut rest).await.unwrap();
        assert!(
            rest.ends_with('\n'),
            "legacy command must be newline-terminated"
        );
        format!("{{{rest}")
    });

    let mut client = Client::connect_legacy(&socket_path).await.unwrap();
    client
        .send(&Command::SetModel {
            id: Some("m-1".into()),
            model: Some("test-model".into()),
            provider: None,
            model_id: None,
        })
        .await
        .unwrap();

    match tokio::time::timeout(std::time::Duration::from_secs(1), client.recv())
        .await
        .unwrap()
    {
        Some(Event::Token { token }) => assert_eq!(token, "from-server"),
        other => panic!("unexpected event: {other:?}"),
    }

    let written = server.await.unwrap();
    assert!(written.contains(r#""type":"set_model""#));
    assert!(written.contains(r#""model":"test-model""#));
    let _ = std::fs::remove_dir_all(&dir);
}
