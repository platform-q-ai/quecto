//! #1460: one socket-path validator governs every connect, and command-send
//! failures are tagged with the connection they occurred on.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn unique_socket_path(dir: &std::path::Path) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dir.join(format!(
        "quecto-tui-test-{}-{nanos}.sock",
        std::process::id()
    ))
}

/// The roster/feed gate must apply the same allowed-roots policy as the CLI's
/// `validate_socket_path` (/tmp, $TMPDIR, $XDG_RUNTIME_DIR, $HOME): a real
/// socket outside those roots is rejected.
#[test]
fn usable_socket_path_rejects_sockets_outside_allowed_roots() {
    let path = unique_socket_path(std::path::Path::new("/dev/shm"));
    let _listener =
        std::os::unix::net::UnixListener::bind(&path).expect("bind test socket in /dev/shm");

    let usable = crate::shell::socket_path::usable_socket_path(Some(path.to_str().unwrap()));

    let _ = std::fs::remove_file(&path);
    assert!(
        !usable,
        "usable_socket_path must enforce the shared allowed-roots policy; \
         it accepted a socket outside /tmp, $TMPDIR, $XDG_RUNTIME_DIR and $HOME"
    );
}

/// The shared validator must not be reject-everything: a real socket inside
/// an allowed root passes the same function that rejects outside ones.
#[test]
fn usable_socket_path_accepts_socket_inside_allowed_root() {
    let path = unique_socket_path(&std::env::temp_dir());
    let _listener =
        std::os::unix::net::UnixListener::bind(&path).expect("bind test socket in temp dir");

    let usable = crate::shell::socket_path::usable_socket_path(Some(path.to_str().unwrap()));

    let _ = std::fs::remove_file(&path);
    assert!(
        usable,
        "usable_socket_path must accept a live socket under an allowed root"
    );
}

/// There must be exactly one socket-path validator. No file under
/// `src/agents/` may define its own `usable_socket_path`; the roster and feed
/// gates must reference the shared `shell::socket_path` module instead.
#[test]
fn agents_tree_has_no_private_socket_path_validator() {
    let agents = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agents");
    for entry in std::fs::read_dir(&agents).expect("read src/agents") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read agents source file");
        assert!(
            !src.contains("fn usable_socket_path"),
            "{} must use the shared socket-path validator, not define its own",
            path.display()
        );
        if src.contains("usable_socket_path") {
            assert!(
                src.contains("socket_path::usable_socket_path"),
                "{} must delegate to shell::socket_path::usable_socket_path",
                path.display()
            );
        }
    }
}

/// A failure constructed with an explicit non-default connection id must
/// surface that id — proving the notice reads the field, not a literal.
#[tokio::test]
async fn command_send_failure_notification_names_the_failing_connection() {
    let mut h = harness().await;
    let a = h.app_mut();

    a.handle_command_send_failure(CommandSendFailure {
        command: Command::NewSession { id: None },
        error: "channel full".into(),
        connection: "tab-2".into(),
    });

    let messages = a.notifications.messages().join("\n");
    assert!(
        messages.contains("connection"),
        "send-failure notice must attribute the failure to a connection, got: {messages:?}"
    );
    assert!(
        messages.contains("tab-2"),
        "the notice must carry the failure's own connection id, got: {messages:?}"
    );
}

/// With today's single connection, a send failure produced by the real
/// `send_command` path is attributed to the master connection id.
#[tokio::test]
async fn command_send_failure_defaults_to_the_master_connection() {
    let mut h = harness().await;
    h.disconnect_master_commands();
    let a = h.app_mut();

    // The real production path: try_send fails, the failure is tagged with
    // the (single) master connection and routed back to the handler.
    assert!(!a.send_command(Command::NewSession { id: None }));
    let failure = a
        .command_send_failure_rx
        .try_recv()
        .expect("send failure must be reported");
    a.handle_command_send_failure(failure);

    let messages = a.notifications.messages().join("\n");
    assert!(
        messages.contains(MASTER_CONNECTION_ID),
        "with a single connection the failure must default to the master \
         connection id, got: {messages:?}"
    );
}

#[tokio::test]
async fn failed_resume_messages_enqueue_clears_pending_resume_id() {
    let mut h = harness().await;
    h.disconnect_master_commands();
    let a = h.app_mut();

    a.handle_response(
        Some("resume".into()),
        "resume_session".into(),
        true,
        Some(serde_json::json!({"session": "chat-1"})),
        None,
    );

    assert_eq!(
        a.test_pending_resume_messages_id(),
        None,
        "failed enqueue of the solicited resume load must not leave an owned pending id"
    );
}

#[tokio::test]
async fn rollback_from_non_master_connection_does_not_clear_master_resume_pending() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.test_arm_resume_messages("resume-messages-owned");

    a.handle_command_send_failure(CommandSendFailure {
        command: Command::GetMessages {
            agent_id: None,
            id: Some("resume-messages-owned".into()),
            before: None,
        },
        error: "tab channel full".into(),
        connection: "tab-2".into(),
    });

    assert_eq!(
        a.test_pending_resume_messages_id(),
        Some("resume-messages-owned"),
        "a non-master connection failure must not roll back master-owned pending resume state"
    );
}
