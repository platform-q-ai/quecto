use super::*;
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn write_event_line_uses_legacy_by_default_and_strips_newline_for_frame() {
    let mut legacy = Vec::new();
    let mode = ConnectionWireMode::default();
    write_event_line(&mut legacy, "{\"ok\":true}\n", &mode)
        .await
        .expect("legacy write succeeds");
    assert_eq!(legacy, b"{\"ok\":true}\n");

    let mut framed = Vec::new();
    mode.record(quecto_line_io::WireMode::Framed);
    write_event_line(&mut framed, "{\"ok\":true}\n", &mode)
        .await
        .expect("framed write succeeds");
    assert_ne!(framed, b"{\"ok\":true}\n");

    let mut cursor = tokio::io::BufReader::new(std::io::Cursor::new(framed));
    let mut announced = Vec::new();
    cursor
        .read_to_end(&mut announced)
        .await
        .expect("frame bytes readable");
    assert!(announced.ends_with(b"{\"ok\":true}"));
    assert!(!announced.ends_with(b"\n"));
}

#[test]
fn socket_announcement_includes_protocol_and_socket_lines() {
    let announcement = socket_announcement(std::path::Path::new("/tmp/q.sock"));
    assert!(announcement.starts_with(PROTOCOL_ANNOUNCE_PREFIX));
    assert!(announcement.contains("quecto-agent-socket: /tmp/q.sock\n"));
    assert_eq!(announcement.lines().count(), 2);
}
