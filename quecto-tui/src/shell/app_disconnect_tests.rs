//! Unit tests for agent-disconnect handling (#1047): a dying agent (e.g. a
//! panic-abort near a full context window) must not silently strip the UI of
//! context — the left panel stays visible and the disconnect notification
//! carries the child's exit diagnosis.

use super::tui_harness::TuiHarness;
use crate::components::chat::ChatEntry;
use crate::components::component::Component;
use crate::protocol::client::Command;

#[tokio::test]
async fn disconnected_chat_submit_does_not_stack_duplicate_refusal_toasts() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();
    app.ac_mut().agent_connected = false;

    app.handle_submit("first drafted message");
    app.handle_submit("second drafted message");

    let messages = h.notification_messages();
    let refusal_count = messages
        .iter()
        .filter(|message| message.as_str() == "Agent disconnected — commands are not being sent")
        .count();
    assert_eq!(
        refusal_count, 1,
        "disconnected chat submits must reuse the deduped refusal toast path; got {messages:?}"
    );
    assert!(
        !messages
            .iter()
            .any(|message| message.as_str() == "Agent disconnected — message not sent"),
        "per-submit disconnected errors must not stack outside the deduped refusal path; got {messages:?}"
    );
}

/// #1047 AC2: once connected, the persistent left panel must survive an agent
/// disconnect (shown in a "disconnected" state) rather than vanishing, so the
/// user keeps the session/sub-agent context needed to diagnose the failure.
#[tokio::test]
async fn left_panel_stays_visible_after_agent_disconnect() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();
    assert!(
        app.subagent_panel_visible(),
        "precondition: panel is visible while connected"
    );

    app.handle_agent_disconnected(None);

    assert!(
        !app.ac().agent_connected,
        "disconnect must still mark the agent as not connected"
    );
    assert!(
        app.subagent_panel_visible(),
        "left panel must remain visible after the agent disconnects (#1047)"
    );
}

/// #1047 AC1: when the TUI owns the agent child and it died, the disconnect
/// notification must include the exit diagnosis (status/signal), not just a
/// bare "Agent disconnected".
#[tokio::test]
async fn disconnect_notification_includes_agent_exit_detail() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();

    app.handle_agent_disconnected(Some(
        "agent process aborted: signal 6 (SIGABRT)".to_string(),
    ));

    let rendered = app.notifications.render(200).join("\n");
    assert!(
        rendered.contains("SIGABRT"),
        "disconnect notification must surface the child's exit detail (#1047): {rendered}"
    );
}

/// #1047 AC1 wiring: the PRODUCTION disconnect path (`handle_agent_stream_closed`)
/// must read the exit diagnosis from the child watcher's slot — for a real
/// spawned child killed by a real signal — not be hand-fed a string.
#[tokio::test]
async fn stream_closed_path_reports_real_child_exit_detail() {
    let mut h = TuiHarness::new().await;

    let child = tokio::process::Command::new("sh")
        .args(["-c", "kill -ABRT $$"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn aborting child");
    let watch = crate::shell::child_watch::watch_child(
        child,
        crate::shell::child_watch::StderrTail::default(),
    );
    h.agent_stream_closed_with_child_watch(watch).await;

    let rendered = h.app_mut().notifications.render(200).join("\n");
    assert!(
        rendered.contains("signal 6 (SIGABRT)"),
        "the stream-closed path must diagnose the real child's abort (#1047): {rendered}"
    );
}

/// #1047 (top-priority fix): the agent's post-startup stderr is drained into
/// the watcher's ring buffer, and the disconnect diagnostics include the tail
/// — under `panic = "abort"` the panic message lands on stderr right before
/// the process dies, and without this every recurrence is undiagnosable.
#[tokio::test]
async fn disconnect_diagnostics_include_drained_stderr_tail() {
    use crate::shell::child_watch::{StderrTail, watch_child};

    let mut h = TuiHarness::new().await;

    let mut child = tokio::process::Command::new("sh")
        .args([
            "-c",
            "echo \"thread 'main' panicked at boom-panic\" >&2; kill -ABRT $$",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn aborting child");
    let stderr = child.stderr.take().expect("child stderr");
    let tail = StderrTail::default();
    crate::shell::cli::spawn_stderr_drain(tokio::io::BufReader::new(stderr), tail.clone());
    let watch = watch_child(child, tail.clone());
    // Deliberately NO wait for the drain here: the disconnect path itself
    // must synchronize on drain completion, or the panic message is
    // nondeterministically missing from the diagnostics (#1051 final review).

    h.agent_stream_closed_with_child_watch(watch).await;

    let rendered = h.app_mut().notifications.render(200).join("\n");
    assert!(
        rendered.contains("boom-panic"),
        "disconnect notification must carry the last stderr line (#1047): {rendered}"
    );
    assert!(
        rendered.contains("SIGABRT"),
        "exit diagnosis must still be present alongside the stderr tail: {rendered}"
    );
}

/// #1047 AC4: a dropped oversized event must be SURFACED in the UI — a
/// counted-but-silent drop still leaves the session looking frozen.
#[tokio::test]
async fn oversized_event_drop_is_surfaced_as_notification() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();

    assert!(
        !app.surface_dropped_oversized_events(),
        "no drops recorded — nothing to surface"
    );

    app.ac().transport.record_dropped_oversized_for_tests(1);
    assert!(
        app.surface_dropped_oversized_events(),
        "a recorded drop must raise a notification"
    );
    let rendered = app.notifications.render(200).join("\n");
    assert!(
        rendered.contains("oversized agent event"),
        "the drop notification must name the oversized-event loss (#1047): {rendered}"
    );

    assert!(
        !app.surface_dropped_oversized_events(),
        "the same drop must not be surfaced twice"
    );
}

/// #1470 review (r3): once the master stream closes, `/new` and `/clear`
/// still clear the LOCAL transcript — a dead session must stay tidyable —
/// but must not flash a misleading "session started" success: the NewSession
/// command would vanish into the writer channel that outlives the closed
/// event stream, so the reset warns about what actually happened.
#[tokio::test]
async fn reset_session_clears_locally_and_warns_when_disconnected() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    a.ac_mut().master_session.chat.add_entry(ChatEntry::User {
        text: "clear me".into(),
    });
    a.ac_mut().agent_connected = false;

    a.reset_session("New session started");

    // The old transcript is gone; the only surviving entry is the
    // re-raised persistent refusal Status line (#1470 r6) so later refused
    // commands stay diagnosable in the fresh transcript.
    assert_eq!(
        a.ac().master_session.chat.entry_count(),
        1,
        "a disconnected reset clears the transcript except the refusal line (#1470 r3/r6)"
    );
    let msgs = a.notifications.messages();
    assert!(
        msgs.iter().any(|m| m.contains("disconnected")),
        "a disconnected reset must warn, got {msgs:?}"
    );
    assert!(
        !msgs.iter().any(|m| m.contains("New session started")),
        "a disconnected reset must not claim success, got {msgs:?}"
    );
}

/// #1470 review: the writer channel outlives the closed event stream, so a raw
/// `try_send` returns `Ok` against a dead socket. `send_command` must refuse up
/// front so post-disconnect commands (/session, /resume, /model, toggles)
/// cannot silently vanish into the dead connection.
#[tokio::test]
async fn send_command_refuses_when_disconnected() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    a.ac_mut().agent_connected = false;
    assert!(
        !a.send_command(Command::NewSession { id: None }),
        "a known-dead connection must refuse the enqueue"
    );
}

/// How many times the stderr-tail transcript header of a disconnect notice
/// (#1047) was written into the master transcript.
fn stderr_transcript_headers(app: &super::App) -> usize {
    app.ac()
        .master_session
        .chat
        .entries()
        .iter()
        .filter(|entry| {
            matches!(entry, ChatEntry::Status { text }
                if text.starts_with("Agent disconnected — recent agent stderr"))
        })
        .count()
}

/// #2044 R1-1: finishing the stream-closed disconnect CONSUMES the diagnosis
/// latch. The notice (toast + stderr-tail transcript entries) is emitted
/// once; a later completion for the same episode — a duplicate or a stale
/// one — takes the toast-only branch and never writes the stderr tail into
/// the transcript a second time (#1470 r3).
#[tokio::test]
async fn finishing_the_stream_closed_disconnect_consumes_the_diagnosis_latch() {
    use crate::shell::child_watch::{StderrTail, watch_child};

    let mut h = TuiHarness::new().await;
    let tail = StderrTail::default();
    tail.push("thread 'main' panicked at latch-pin".to_string());
    tail.mark_drained();
    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 7"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn exiting child");
    h.app_mut().set_child_exit_watch(watch_child(child, tail));

    h.app_mut().begin_agent_stream_closed();
    assert!(
        h.app_mut().ac().disconnect_diag_pending,
        "precondition: an owned child defers the diagnosis behind the latch"
    );
    h.pump_disconnect_diagnosis().await;

    assert!(
        !h.app_mut().ac().disconnect_diag_pending,
        "finishing the disconnect must clear the diagnosis latch"
    );
    assert_eq!(
        stderr_transcript_headers(h.app_mut()),
        1,
        "the finished disconnect writes the stderr tail into the transcript once"
    );

    h.app_mut()
        .finish_agent_stream_closed(Some("late completion".to_string()));

    assert_eq!(
        stderr_transcript_headers(h.app_mut()),
        1,
        "a completion after the latch was consumed must not write the transcript again"
    );
    let messages = h.notification_messages();
    assert!(
        messages
            .iter()
            .any(|message| message == "Agent disconnected — late completion"),
        "the late diagnosis still surfaces as a toast (#1470 r6), got {messages:?}"
    );
}

/// #2044 R1-5: the duplicate gate. A second `Closed` sentinel in the same
/// disconnect episode is a no-op on an ownerless connection — exactly one
/// "Agent disconnected" notice, not one per sentinel.
#[tokio::test]
async fn a_second_closed_sentinel_does_not_repeat_the_disconnect_notice() {
    use crate::shell::connection::SourcedEvent;

    let mut h = TuiHarness::new().await;
    let app = h.app_mut();

    app.route_sourced(SourcedEvent::Closed);
    app.route_sourced(SourcedEvent::Closed);

    let messages = app.notifications.messages();
    let notices = messages
        .iter()
        .filter(|message| message.as_str() == "Agent disconnected")
        .count();
    assert_eq!(
        notices, 1,
        "one disconnect episode raises one notice, got {messages:?}"
    );
}

/// #2044 R1-5: with an owned child the duplicate gate also keeps the
/// off-loop diagnosis single — a second sentinel must not spawn a second
/// diagnosis task, whose completion would repeat the notice.
#[tokio::test(start_paused = true)]
async fn a_second_closed_sentinel_does_not_start_a_second_diagnosis() {
    use crate::shell::connection::SourcedEvent;

    let mut h = TuiHarness::new().await;
    let app = h.app_mut();
    app.set_child_exit_watch(crate::shell::child_watch::ChildWatch::for_tests(Some(77)));

    app.route_sourced(SourcedEvent::Closed);
    app.route_sourced(SourcedEvent::Closed);
    h.pump_disconnect_diagnosis().await;

    let second = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        h.app_mut().disconnect_diag_rx.recv(),
    )
    .await;
    assert!(
        second.is_err(),
        "one disconnect episode reports one diagnosis, got a second: {second:?}"
    );
    let messages = h.notification_messages();
    let notices = messages
        .iter()
        .filter(|message| message.starts_with("Agent disconnected"))
        .count();
    assert_eq!(
        notices, 1,
        "one disconnect episode raises one notice, got {messages:?}"
    );
}
