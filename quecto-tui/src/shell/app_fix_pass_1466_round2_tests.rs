//! #1466 round-2 fix-pass RED tests (PR #1485 second field-testing round).
//!
//! Covers the three round-2 regressions:
//! 1. The `quecto-tui vX — Enter send…` header line is dropped, replaced by a
//!    BLANK spacer line so the tab bar / Master status keep breathing room
//!    and the frame geometry stays otherwise identical.
//! 2. New-tab chord: Ctrl+T is already the tool-policy selector, so the next
//!    best terminal-safe plain-control chord is Ctrl+N (0x0E in every
//!    terminal/tmux; only Ctrl+SHIFT+N is taken, a distinct key). /hotkeys
//!    documents it.
//! 3. User sends to live restored sub-agents must attach the feed on demand
//!    (like the master-driven path) instead of failing "unattached", while
//!    sends to genuinely dead sub-agents keep surfacing a visible error.

use super::app_fix_pass_1466_tests::{headless_app, subagent_info, two_tab_app};
use super::*;
use crate::shell::keys::Key;

// ── Item 1: version/help header line → blank spacer ──────────────────────

#[tokio::test]
async fn frame_contains_no_version_header_line() {
    let mut app = headless_app();
    let lines = app.compose_frame();
    assert!(
        !lines.iter().any(|l| l.contains("quecto-tui v")),
        "the version/help header line must no longer render anywhere in the \
         frame (round-2 item 1)"
    );
}

#[tokio::test]
async fn single_tab_frame_starts_with_blank_spacer_and_keeps_height() {
    let mut app = headless_app();
    let lines = app.compose_frame();
    let first = super::app_render_helpers::strip_ansi(&lines[0]);
    // The frame may carry a left panel cell; the header slot is the BODY
    // segment past the divider.
    let body = first.rsplit('│').next().unwrap_or(&first);
    assert_eq!(
        body.trim(),
        "",
        "with one tab the header slot must be a BLANK spacer line, so the \
         Master status line keeps its breathing room; first={first:?}"
    );
    assert_eq!(
        lines.len(),
        24,
        "frame height must stay identical to the terminal height after the \
         header swap (only content changed, not geometry)"
    );
}

#[tokio::test]
async fn multi_tab_frame_has_blank_spacer_after_tab_bar() {
    let mut app = two_tab_app();
    let lines = app.compose_frame();
    let first = super::app_render_helpers::strip_ansi(&lines[0]);
    assert!(
        first.contains(" 1 ") && first.contains(" 2 "),
        "with 2+ tabs the tab bar stays the first frame line; first={first:?}"
    );
    let second = super::app_render_helpers::strip_ansi(&lines[1]);
    let second_body = second.rsplit('│').next().unwrap_or(&second);
    assert_eq!(
        second_body.trim(),
        "",
        "the line after the tab bar must be the blank spacer that replaced \
         the version header; second={second:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("quecto-tui v")),
        "no multi-tab frame line may contain the version header"
    );
    assert_eq!(
        lines.len(),
        24,
        "the multi-tab frame (tab bar + spacer) must also keep the terminal \
         height — the geometry most likely to drift (round-2 review)"
    );
}

// ── Item 2: Ctrl+N opens a new tab (Ctrl+T is taken by tool policy) ──────

#[test]
fn ctrl_n_opens_a_new_pending_attach_tab() {
    let mut app = headless_app();
    app.handle_key(Key::Ctrl('n'));
    assert_eq!(
        app.tabs.len(),
        2,
        "Ctrl+N (0x0E, terminal-safe everywhere incl. tmux) must open a new \
         tab like /tab-new and the clickable ' + '"
    );
    let tab = app.active_tab;
    assert_ne!(
        tab,
        crate::shell::connection::TabId::MASTER,
        "the new tab must become the active tab"
    );
    assert!(
        app.tab_has_pending_attach(tab),
        "Ctrl+N must take the same live-attach path as /tab-new"
    );
}

/// Conflict guard (expected green): Ctrl+T stays the tool-policy selector —
/// the reason the new-tab chord is Ctrl+N — and must never open a tab.
#[tokio::test]
async fn ctrl_t_still_opens_tool_policy_selector_not_a_tab() {
    let mut app = headless_app();
    app.handle_key(Key::Ctrl('t'));
    assert!(
        app.tool_policy_modal_pending_catalogue_id.is_some(),
        "Ctrl+T must keep requesting the tool-policy catalogue"
    );
    assert_eq!(app.tabs.len(), 1, "Ctrl+T must not open a tab");
}

#[test]
fn hotkeys_text_documents_ctrl_n_as_new_tab_chord() {
    let mut app = headless_app();
    app.show_help();
    let text = app
        .ac_mut()
        .master_session
        .chat
        .last_status_text()
        .expect("help status entry")
        .to_string();
    assert!(
        text.contains("Ctrl+N "),
        "/hotkeys must list plain Ctrl+N (distinct from Ctrl+Shift+N) as a \
         chord; text={text}"
    );
    let ctrl_n_line = text
        .lines()
        .find(|l| l.contains("Ctrl+N ") && !l.contains("Ctrl+Shift"))
        .unwrap_or("")
        .to_lowercase();
    assert!(
        ctrl_n_line.contains("new tab") || ctrl_n_line.contains("open tab"),
        "the Ctrl+N line must describe opening a new tab; text={text}"
    );
}

// ── Item 3: user sends to restored sub-agents attach on demand ───────────

/// No delivery-failure Status line or toast for `app`'s active session.
fn assert_no_delivery_failure(app: &App) {
    let status = app
        .active_session()
        .chat
        .last_status_text()
        .unwrap_or("")
        .to_string();
    let last_note = app
        .notifications
        .messages()
        .last()
        .cloned()
        .unwrap_or_default();
    let failed = |s: &str| s.contains("not delivered");
    assert!(
        !failed(&status) && !failed(&last_note),
        "no delivery-failure may surface for a reachable sub-agent; \
         status={status:?}, notification={last_note:?}"
    );
}

/// The user entry landed in the active (sub-agent) transcript.
fn assert_user_entry(app: &App, text: &str) {
    let has_user_entry = app
        .active_session()
        .chat
        .entries()
        .iter()
        .any(|e| matches!(e, ChatEntry::User { text: t } if t == text));
    assert!(
        has_user_entry,
        "the user message {text:?} must appear in the sub-agent transcript"
    );
}

/// Await the wire-delivered command lines until the routed user message
/// arrives (a `prompt` for an idle child, a `follow_up` when mid-turn).
async fn expect_message_on_wire(cmd_rx: &mut tokio::sync::mpsc::Receiver<String>, message: &str) {
    let deadline = std::time::Duration::from_secs(5);
    tokio::time::timeout(deadline, async {
        while let Some(line) = cmd_rx.recv().await {
            let kind = super::tui_harness::child_command_type(&line);
            if matches!(kind.as_deref(), Some("prompt" | "follow_up")) && line.contains(message) {
                return;
            }
        }
        panic!("child socket closed before the user message arrived");
    })
    .await
    .expect("the routed user message must arrive on the child's live socket")
}

#[tokio::test]
async fn user_send_to_live_restored_subagent_is_delivered_not_unattached() {
    let mut app = headless_app();
    // Post-resume field state: the restored child is focused BEFORE its live
    // socket is known, so the focus-time feed is inspection-only …
    app.update_subagent_bar(vec![subagent_info("w1", "running")]);
    app.select_agent(Some("w1"));
    assert!(
        !app.subagent_feed_is_direct("w1"),
        "precondition: focusing before the socket is known attaches an \
         inspection-only feed"
    );
    // … then the child's live registry socket becomes known (master-driven
    // messaging already works at this point), but the stale feed lingers.
    let (socket, mut cmd_rx) = super::tui_harness::spawn_subagent_socket_with_commands("w1");
    app.ac_mut()
        .roster
        .tracked
        .get_mut("w1")
        .expect("tracked w1")
        .info
        .socket_path = Some(socket.to_string_lossy().into_owned());

    app.handle_submit("hello restored agent");

    // The observable routing side effects (round-2 falsifiability review):
    // the send attached a DIRECT feed and the prompt reached the child's
    // socket — an echo-only reorder or an attach-code revert fails here.
    assert!(
        app.subagent_feed_is_direct("w1"),
        "the user-send path must attach the direct feed on demand, like the \
         master-driven roster-refresh path"
    );
    expect_message_on_wire(&mut cmd_rx, "hello restored agent").await;
    assert_user_entry(&app, "hello restored agent");
    assert_no_delivery_failure(&app);
}

/// Coverage-review boundary: restored sub-agents present as "detached" before
/// any feed attach. Detached-but-REACHABLE (live registry socket) must attach
/// on demand and deliver; only detached-and-unreachable keeps erroring (the
/// round-1 test pins that side with no usable socket).
#[tokio::test]
async fn user_send_to_detached_but_reachable_subagent_is_delivered() {
    let mut app = headless_app();
    let (socket, mut cmd_rx) = super::tui_harness::spawn_subagent_socket_with_commands("w1");
    app.update_subagent_bar(vec![super::tui_harness::subagent_with_socket(
        "w1",
        "detached",
        None,
        Some(socket),
    )]);
    app.select_agent(Some("w1"));
    app.handle_submit("hello detached agent");

    assert!(
        app.subagent_feed_is_direct("w1"),
        "a detached roster row with a live socket must carry a direct feed"
    );
    expect_message_on_wire(&mut cmd_rx, "hello detached agent").await;
    assert_user_entry(&app, "hello detached agent");
    assert_no_delivery_failure(&app);
}

/// Round-1 guarantee guard (expected green): genuinely dead sub-agents still
/// surface a visible delivery failure — attach-on-demand must not resurrect
/// sends to the dead.
#[tokio::test]
async fn user_send_to_dead_subagent_still_surfaces_visible_error() {
    let mut app = headless_app();
    app.update_subagent_bar(vec![subagent_info("w2", "dead")]);
    app.select_agent(Some("w2"));
    app.handle_submit("hello dead agent");
    let status = app
        .active_session()
        .chat
        .last_status_text()
        .unwrap_or("")
        .to_string();
    let last_note = app
        .notifications
        .messages()
        .last()
        .cloned()
        .unwrap_or_default();
    let surfaced = |s: &str| s.contains("not delivered") && s.contains("w2");
    assert!(
        surfaced(&status) || surfaced(&last_note),
        "sends to a dead sub-agent must keep surfacing a visible error; \
         status={status:?}, notification={last_note:?}"
    );
}
