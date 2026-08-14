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

#[tokio::test]
async fn user_send_to_live_restored_subagent_is_delivered_not_unattached() {
    let mut app = headless_app();
    // Post-resume state: the roster tracks a LIVE child (master-driven
    // messaging works) but there is no direct child socket, so the focused
    // feed is inspection-only. Round 1 made this send fail "unattached";
    // round 2 requires the user-send path to attach/route like the
    // master-driven path.
    app.update_subagent_bar(vec![subagent_info("w1", "running")]);
    app.select_agent(Some("w1"));
    app.handle_submit("hello restored agent");

    let has_user_entry = app
        .active_session()
        .chat
        .entries()
        .iter()
        .any(|e| matches!(e, ChatEntry::User { text } if text == "hello restored agent"));
    assert!(
        has_user_entry,
        "a user message to a LIVE restored sub-agent must be delivered and \
         appear in its transcript (attach-on-demand), not be dropped"
    );

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
        "no delivery-failure may surface for a live restored sub-agent; \
         status={status:?}, notification={last_note:?}"
    );
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
