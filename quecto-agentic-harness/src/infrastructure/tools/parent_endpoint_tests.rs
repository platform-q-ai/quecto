use super::parent_endpoint::*;
use crate::domain::agent_launch_backend::ParentEndpoint;
use std::path::PathBuf;
use std::time::Duration;

async fn one_response_listener(path: PathBuf) -> tokio::task::JoinHandle<String> {
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = tokio::io::BufReader::new(stream);
        let payload =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .unwrap()
                .unwrap();
        let text = String::from_utf8(payload).unwrap();
        let sent: serde_json::Value = serde_json::from_str(&text).unwrap();
        let ack = serde_json::json!({"type":"response","id":sent["id"],"success":true}).to_string();
        quecto_line_io::write_frame(
            reader.get_mut(),
            ack.as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
        text
    })
}

#[tokio::test]
async fn direct_endpoint_connects_to_socket_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let socket = dir.path().join("direct.sock");
    let seen = one_response_listener(socket.clone()).await;
    let response = send_command_with_timeout(
        &ParentEndpoint::DirectUds(socket),
        r#"{"type":"get_state"}"#,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert!(response.contains("success"));
    assert!(seen.await.unwrap().contains("get_state"));
}

#[tokio::test]
async fn proxy_endpoint_connects_to_proxy_not_requested_uds() {
    let dir = tempfile::TempDir::new().unwrap();
    let proxy = dir.path().join("proxy.sock");
    let missing_requested_uds = dir.path().join("missing-child.sock");
    let seen = one_response_listener(proxy.clone()).await;
    let response = send_command_with_timeout(
        &ParentEndpoint::Proxy(format!("unix:{}", proxy.display())),
        r#"{"type":"get_state"}"#,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert!(response.contains("success"));
    assert!(!missing_requested_uds.exists());
    assert!(seen.await.unwrap().contains("get_state"));
}
