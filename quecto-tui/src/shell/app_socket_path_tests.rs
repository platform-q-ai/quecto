//! #1460 RED tests: one socket-path validator governs every connect, and
//! command-send failures are tagged with the connection they occurred on.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

/// The subagent-feed/roster gate (`usable_socket_path`) must apply the same
/// allowed-roots policy as the CLI's `validate_socket_path` (/tmp, $TMPDIR,
/// $XDG_RUNTIME_DIR, $HOME). A real socket outside those roots must be
/// rejected by every validator, not only the CLI one.
#[test]
fn usable_socket_path_rejects_sockets_outside_allowed_roots() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::path::PathBuf::from(format!(
        "/dev/shm/quecto-tui-test-{}-{nanos}.sock",
        std::process::id()
    ));
    let _listener =
        std::os::unix::net::UnixListener::bind(&path).expect("bind test socket in /dev/shm");

    let usable = super::app_subagents::usable_socket_path(Some(path.to_str().unwrap()));

    let _ = std::fs::remove_file(&path);
    assert!(
        !usable,
        "usable_socket_path must enforce the shared allowed-roots policy; \
         it accepted a socket outside /tmp, $TMPDIR, $XDG_RUNTIME_DIR and $HOME"
    );
}

/// There must be exactly one socket-path validator: the copy private to
/// `controller_subagent_feed.rs` has to delegate to the shared one instead of
/// duplicating the predicate (the duplicate is missing the allowed-roots
/// check today, and any future fix would land in only one of the two).
#[test]
fn subagent_feed_does_not_define_its_own_socket_path_validator() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let feed = std::fs::read_to_string(root.join("src/agents/controller_subagent_feed.rs"))
        .expect("read controller_subagent_feed.rs");
    assert!(
        !feed.contains("fn usable_socket_path"),
        "controller_subagent_feed.rs must use the shared socket-path validator, \
         not define a private duplicate"
    );
}

/// A failed command send must be attributed to the connection it happened on
/// (defaulted to the single master connection today) so that with N per-tab
/// connections the rollback/notice cannot be misrouted cross-tab.
#[tokio::test]
async fn command_send_failure_notification_names_the_connection() {
    let mut h = harness().await;
    let a = h.app_mut();

    a.handle_command_send_failure(CommandSendFailure {
        command: Command::NewSession { id: None },
        error: "channel full".into(),
    });

    let messages = a.notifications.messages().join("\n");
    assert!(
        messages.contains("connection"),
        "send-failure notice must attribute the failure to a connection, got: {messages:?}"
    );
    assert!(
        messages.contains("master"),
        "with a single connection the failure must default to the master \
         connection id, got: {messages:?}"
    );
}
