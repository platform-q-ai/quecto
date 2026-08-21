//! #1466 fix-pass RED tests (PR #1485 field regressions).
//!
//! Covers the six regressions found in local testing:
//! 1. Tab bar must render herdr-style reverse-video number blocks (spike
//!    design), never a default ":Master" suffix.
//! 2. Terminal-safe cycle chords: Ctrl+PgUp / Ctrl+PgDn (WM-proof), and the
//!    /hotkeys text presenting Ctrl+1-9 + Ctrl+PgUp/PgDn as primary.
//! 3. /resume: workspaces sorted by last-active desc with per-tab
//!    conversation snippets, not opaque labels.
//! 4. Workspace resurrection: the FIRST manifest entry (reused master slot)
//!    must resume like every other tab even when stored tab ids don't match.
//! 5. Dead sub-agents: distinct panel styling for detached/dead, and no
//!    silent swallow when sending to an unattached sub-agent.
//! 6. Background-tab spinner: the animation service must tick BACKGROUND tab
//!    spinners and request a repaint so the bar keeps animating.

use super::*;
use crate::protocol::client::Event;
use crate::shell::connection::{SourcedEvent, TabId};
use crate::shell::keys::{Key, parse_key};
use crate::shell::terminal::Terminal;
use crate::shell::workspace_manifest::{WorkspaceManifestStore, WorkspaceTabEntry};

pub(super) fn headless_app() -> App {
    let client = crate::protocol::client::Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    let mut app = App::new(term, client);
    app.suppress_paint = true;
    app
}

pub(super) fn two_tab_app() -> App {
    let mut app = headless_app();
    app.test_insert_disconnected_tab(1);
    app
}

fn route_and_render(
    app: &mut App,
    coalescer: &mut super::app_event_loop::StreamRenderCoalescer,
    tab: u32,
    ev: Event,
) {
    let render = app.route_sourced(SourcedEvent::Tab(TabId(tab), ev));
    app.apply_sourced_render(render, coalescer);
}

pub(super) fn subagent_info(id: &str, status: &str) -> crate::protocol::client::SubagentInfoEvent {
    crate::protocol::client::SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        compact: false,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: None,
        environment: None,
    }
}

// ── Item 1: tab bar herdr-style number blocks ────────────────────────────

/// The active tab's block, composed through the SAME theme helpers as the
/// implementation (PR #1485 review: no hand-rolled SGR bytes to cement).
fn active_block(text: &str) -> String {
    crate::components::theme::reverse(&crate::components::theme::cyan(text))
}

/// An inactive tab's block (reverse-video dim, via theme helpers).
fn inactive_block(text: &str) -> String {
    crate::components::theme::reverse(&crate::components::theme::dim(text))
}

#[test]
fn tab_bar_renders_number_blocks_without_default_master_suffix() {
    let mut app = two_tab_app();
    let bar = app.render_tab_bar(80).expect("bar with 2 tabs");
    assert!(
        !bar.contains(":Master"),
        "unnamed tabs must render the bare 1-based number block, never a \
         default ':Master' suffix; bar={bar:?}"
    );
    let stripped = super::app_render_helpers::strip_ansi(&bar);
    assert!(
        !stripped.contains(':'),
        "unnamed tabs must render no colon at all; bar={stripped:?}"
    );
    // Falsifiable active/inactive roles: the CYAN block is the ACTIVE tab.
    assert!(
        bar.contains(&active_block(" 1 ")),
        "the ACTIVE tab (1) must be the reverse-video cyan block; bar={bar:?}"
    );
    assert!(
        !bar.contains(&active_block(" 2 ")),
        "only the active tab gets the cyan block; bar={bar:?}"
    );
    assert!(
        bar.contains(&inactive_block(" 2 ")),
        "the inactive tab (2) must be a reverse-video dim block; bar={bar:?}"
    );
    // Switch focus: the cyan role must follow the active tab.
    assert!(app.switch_tab(TabId(1)));
    let bar = app.render_tab_bar(80).expect("bar with 2 tabs");
    assert!(
        bar.contains(&active_block(" 2 ")),
        "after switching, tab 2 must be the cyan block; bar={bar:?}"
    );
    assert!(
        bar.contains(&inactive_block(" 1 ")),
        "after switching, tab 1 must be a dim block; bar={bar:?}"
    );
}

#[test]
fn tab_bar_truncates_custom_names_to_sixteen_with_ellipsis() {
    let mut app = two_tab_app();
    if let Some(c) = app.conn_mut(TabId(1)) {
        c.name = Some("averyverylongtabname".to_string()); // 20 chars
    }
    let bar = app.render_tab_bar(120).expect("bar with 2 tabs");
    assert!(
        !bar.contains("averyverylongtabname"),
        "custom names longer than 16 chars must be truncated; bar={bar:?}"
    );
    // Exact 16-column rendering: 15-char prefix + ellipsis.
    assert!(
        bar.contains(" 2:averyverylongta… "),
        "a 20-char name must render as its 15-char prefix + '…' inside the \
         ' N:name ' block; bar={bar:?}"
    );
}

#[test]
fn tab_bar_renders_sixteen_char_names_in_full() {
    let mut app = two_tab_app();
    if let Some(c) = app.conn_mut(TabId(1)) {
        c.name = Some("sixteencharsname".to_string()); // exactly 16 chars
    }
    let bar = app.render_tab_bar(120).expect("bar with 2 tabs");
    assert!(
        bar.contains(" 2:sixteencharsname "),
        "an exactly-16-char custom name must render in full as ' N:name '; bar={bar:?}"
    );
    assert!(
        !bar.contains('…'),
        "a name at the 16-char cap must not be ellipsized; bar={bar:?}"
    );
}

#[test]
fn tab_bar_renders_short_custom_names_as_number_colon_name() {
    let mut app = two_tab_app();
    if let Some(c) = app.conn_mut(TabId(1)) {
        c.name = Some("spike".to_string());
    }
    let bar = app.render_tab_bar(80).expect("bar with 2 tabs");
    assert!(
        bar.contains(" 2:spike "),
        "a custom-named tab must render ' N:name '; bar={bar:?}"
    );
}

#[test]
fn tab_bar_ends_with_dim_new_tab_button() {
    let app = two_tab_app();
    let bar = app.render_tab_bar(80).expect("bar with 2 tabs");
    let stripped = super::app_render_helpers::strip_ansi(&bar);
    assert!(
        stripped.trim_end().ends_with('+'),
        "the bar must END with the ' + ' new-tab button; bar={stripped:?}"
    );
    // The `+` segment must be introduced by the dim SGR, not a block style.
    assert!(
        bar.ends_with(&crate::components::theme::dim(" + ")),
        "the ' + ' button must render dim; bar={bar:?}"
    );
}

/// Midpoint of the recorded hit range for the given predicate.
fn hit_midpoint(app: &App, pred: impl Fn(&super::tab_activity::TabBarHit) -> bool) -> u16 {
    let (_, _, width) = app.frame_split();
    let (range, _) = app
        .tab_bar_hit_ranges(width)
        .into_iter()
        .find(|(_, hit)| pred(hit))
        .expect("hit range recorded");
    ((range.start + range.end) / 2) as u16
}

#[test]
fn clicking_a_tab_block_switches_tabs() {
    use super::tab_activity::TabBarHit;
    let mut app = two_tab_app();
    assert_eq!(app.active_tab, TabId(0));
    // Click the midpoint of tab 2's RECORDED hit range — no guessed columns,
    // so indicator glyphs inside blocks cannot silently shift the target.
    let col = hit_midpoint(&app, |h| *h == TabBarHit::Select(TabId(1)));
    app.handle_key(Key::MousePress(col, 0));
    app.handle_key(Key::MouseRelease(col, 0));
    assert_eq!(
        app.active_tab,
        TabId(1),
        "clicking a tab's number block must focus that tab"
    );
}

#[test]
fn clicking_the_plus_button_opens_a_tab() {
    use super::tab_activity::TabBarHit;
    let mut app = two_tab_app();
    let col = hit_midpoint(&app, |h| *h == TabBarHit::New);
    app.handle_key(Key::MousePress(col, 0));
    app.handle_key(Key::MouseRelease(col, 0));
    assert_eq!(
        app.tabs.len(),
        3,
        "clicking the trailing ' + ' button must open a new tab"
    );
}

#[test]
fn clicking_outside_any_hit_range_changes_nothing() {
    let mut app = two_tab_app();
    // Past every recorded range on the bar row: neither a switch nor a new tab.
    app.handle_key(Key::MousePress(70, 0));
    app.handle_key(Key::MouseRelease(70, 0));
    assert_eq!(
        app.active_tab,
        TabId(0),
        "a click in the bar's dead space must not switch tabs"
    );
    assert_eq!(
        app.tabs.len(),
        2,
        "a click in the bar's dead space must not open a tab"
    );
}

// ── Item 2: terminal-safe cycle chords + /hotkeys text ───────────────────

#[test]
fn ctrl_page_keys_parse_to_tab_switch_chords_legacy_csi() {
    // xterm/alacritty/tmux legacy encoding: CSI 5;5~ / CSI 6;5~ (modifier
    // 5 = Ctrl). These pass through Hyprland untouched, unlike Alt/Ctrl+Tab.
    let (prev, _) = parse_key(b"\x1b[5;5~").expect("Ctrl+PgUp parses");
    assert_eq!(
        prev,
        Key::TabSwitchPrev,
        "Ctrl+PageUp must cycle to the previous tab (tmux/browser convention)"
    );
    let (next, _) = parse_key(b"\x1b[6;5~").expect("Ctrl+PgDn parses");
    assert_eq!(
        next,
        Key::TabSwitchNext,
        "Ctrl+PageDown must cycle to the next tab (tmux/browser convention)"
    );
}

#[test]
fn ctrl_page_keys_parse_to_tab_switch_chords_kitty() {
    let (prev, _) = parse_key(b"\x1b[7;5u").expect("kitty Ctrl+PgUp parses");
    assert_eq!(
        prev,
        Key::TabSwitchPrev,
        "kitty-encoded Ctrl+PageUp must alias the legacy CSI chord"
    );
    let (next, _) = parse_key(b"\x1b[8;5u").expect("kitty Ctrl+PgDn parses");
    assert_eq!(
        next,
        Key::TabSwitchNext,
        "kitty-encoded Ctrl+PageDown must alias the legacy CSI chord"
    );
}

#[test]
fn ctrl_page_key_dispatch_cycles_with_direction() {
    // Three tabs so direction is falsifiable: from MASTER, prev wraps to
    // TabId(2) while next lands on TabId(1).
    let mut app = two_tab_app();
    app.test_insert_disconnected_tab(2);
    let (prev, _) = parse_key(b"\x1b[5;5~").expect("Ctrl+PgUp parses");
    app.handle_key(prev);
    assert_eq!(
        app.active_tab,
        TabId(2),
        "Ctrl+PageUp from the first tab must wrap to the LAST tab (prev)"
    );
    let (next, _) = parse_key(b"\x1b[6;5~").expect("Ctrl+PgDn parses");
    app.handle_key(next);
    assert_eq!(
        app.active_tab,
        TabId(0),
        "Ctrl+PageDown must cycle forward (wraps from last to first)"
    );
}

#[test]
fn hotkeys_text_presents_ctrl_digit_and_ctrl_page_chords_as_primary() {
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
        text.contains("Ctrl+1"),
        "/hotkeys must present Ctrl+1-9 as the primary tab-focus chord; text={text}"
    );
    assert!(
        text.contains("Ctrl+PgUp") || text.contains("Ctrl+PageUp"),
        "/hotkeys must present Ctrl+PgUp/PgDn as the primary tab-cycle chords; text={text}"
    );
    // Primary means FIRST: the Ctrl chords must appear before any Alt
    // fallback mention for the same actions.
    let ctrl_pos = text.find("Ctrl+1").expect("Ctrl+1 present");
    if let Some(alt_pos) = text.find("Alt+1") {
        assert!(
            ctrl_pos < alt_pos,
            "Ctrl+1-9 must be presented BEFORE the Alt+1-9 fallback; text={text}"
        );
    }
}

// ── Item 3: /resume recency sort + conversation snippets ─────────────────

#[test]
fn resume_selector_sorts_workspaces_by_last_active_descending() {
    let mut app = headless_app();
    let dir = tempfile::tempdir().unwrap();
    let mpath = dir.path().join("m.json");
    let mut store = WorkspaceManifestStore::new();
    let mut older = App::test_workspace_manifest(
        "ws-older",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("s-old".into()),
            name: None,
            summary: None,
        }],
        0,
    );
    older.last_active_unix_s = 1_000;
    let mut newer = App::test_workspace_manifest(
        "ws-newer",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("s-new".into()),
            name: None,
            summary: None,
        }],
        0,
    );
    newer.last_active_unix_s = 2_000;
    // Stored older-first: the selector must still list most-recent first.
    store.upsert(older);
    store.upsert(newer);
    store.store(&mpath).unwrap();

    app.open_resume_selector_with_workspaces(Vec::new(), &mpath, None);
    let sel = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .expect("selector");
    let values: Vec<_> = sel
        .items_for_tests()
        .iter()
        .map(|i| i.value.clone())
        .collect();
    assert_eq!(
        values[0], "workspace:ws-newer",
        "workspaces must be sorted by last-active descending; got {values:?}"
    );
}

#[test]
fn resume_selector_shows_per_tab_conversation_snippets() {
    let mut app = headless_app();
    let dir = tempfile::tempdir().unwrap();
    let mpath = dir.path().join("m.json");
    let mut store = WorkspaceManifestStore::new();
    store.upsert(App::test_workspace_manifest(
        "ws-snip",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("s0".into()),
            name: None,
            summary: Some("fix the auth bug".into()),
        }],
        0,
    ));
    store.store(&mpath).unwrap();

    app.open_resume_selector_with_workspaces(Vec::new(), &mpath, None);
    let sel = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .expect("selector");
    let rows: Vec<_> = sel
        .items_for_tests()
        .iter()
        .map(|i| {
            format!(
                "{} | {}",
                i.label,
                i.description.clone().unwrap_or_default()
            )
        })
        .collect();
    assert!(
        rows.iter().any(|r| r.contains("fix the auth bug")),
        "workspace rows must show a chat-context snippet per tab, not just \
         label + tab count; rows={rows:?}"
    );
}

#[test]
fn resume_selector_shows_relative_last_active_time_per_workspace_row() {
    let mut app = headless_app();
    let dir = tempfile::tempdir().unwrap();
    let mpath = dir.path().join("m.json");
    let mut store = WorkspaceManifestStore::new();
    let mut ws = App::test_workspace_manifest(
        "ws-aged",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("s0".into()),
            name: None,
            summary: None,
        }],
        0,
    );
    // Two hours ago (with slack inside the hour bucket).
    ws.last_active_unix_s = crate::shell::tab_registry::unix_now_s() - 2 * 3_600 - 30;
    store.upsert(ws);
    store.store(&mpath).unwrap();

    app.open_resume_selector_with_workspaces(Vec::new(), &mpath, None);
    let sel = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .expect("selector");
    let rows: Vec<_> = sel
        .items_for_tests()
        .iter()
        .map(|i| i.description.clone().unwrap_or_default())
        .collect();
    assert!(
        rows.iter().any(|r| r.contains("2h ago")),
        "workspace rows must show a human-relative last-active time; rows={rows:?}"
    );
}

#[test]
fn resume_selector_sorts_sessions_by_last_active_descending() {
    let mut app = headless_app();
    let dir = tempfile::tempdir().unwrap();
    let mpath = dir.path().join("m.json"); // empty manifest: sessions only
    let data = serde_json::json!({
        "sessions": [
            { "key": "s-old", "title": "older session", "messageCount": 3,
              "updatedUnixSecs": 1_000 },
            { "key": "s-new", "title": "newer session", "messageCount": 5,
              "updatedUnixSecs": 2_000 },
        ]
    });
    app.open_resume_selector_at(&data, &mpath);
    let sel = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .expect("selector");
    let values: Vec<_> = sel
        .items_for_tests()
        .iter()
        .map(|i| i.value.clone())
        .collect();
    assert_eq!(
        values[0], "session:s-new",
        "bare sessions must also list most-recently-active first; got {values:?}"
    );
}

#[test]
fn workspace_snapshot_captures_last_user_message_as_tab_summary() {
    let mut app = headless_app();
    app.ac_mut()
        .master_session
        .chat
        .add_entry(crate::components::chat::ChatEntry::User {
            text: "refactor the tokenizer".to_string(),
        });
    let manifest = app.workspace_manifest_snapshot("ws-x");
    assert_eq!(
        manifest.tabs[0].summary.as_deref(),
        Some("refactor the tokenizer"),
        "the manifest snapshot must persist each tab's last user message as \
         its /resume summary"
    );
}

// ── Item 4: first-tab workspace resurrection ─────────────────────────────

#[test]
fn first_manifest_entry_resumes_even_when_stored_tab_ids_do_not_match() {
    let mut app = headless_app();
    app.ac_mut().agent_connected = true;
    // Stored ids from a previous run (7/8) do not exist in the fresh TUI,
    // whose only tab is MASTER(0). Entry 0 reuses the master slot; entry 1
    // opens a new tab. BOTH must carry their session resume.
    let manifest = App::test_workspace_manifest(
        "ws-mismatch",
        vec![
            WorkspaceTabEntry {
                tab_id: 7,
                session_key: Some("sess-a".into()),
                name: Some("one".into()),
                summary: None,
            },
            WorkspaceTabEntry {
                tab_id: 8,
                session_key: Some("sess-b".into()),
                name: Some("two".into()),
                summary: None,
            },
        ],
        0,
    );
    app.apply_workspace_manifest(&manifest);

    let carries = |key: &str| {
        app.tabs.values().any(|c| {
            c.session_key.as_deref() == Some(key)
                || c.pending_session_resume.as_deref() == Some(key)
        })
    };
    assert!(
        carries("sess-a"),
        "the FIRST manifest entry (reused master slot) must resume its \
         session like every other tab"
    );
    assert!(
        carries("sess-b"),
        "later manifest entries must resume their sessions"
    );
    let msgs = app.notifications.messages().join("\n");
    assert!(
        !msgs.contains("missing tab"),
        "reusing the master slot for entry 0 must not report it missing; \
         notifications={msgs:?}"
    );
}

#[test]
fn first_manifest_entry_with_stale_id_still_reattaches_a_live_registry_socket() {
    use crate::shell::tab_registry::{TabAgentRecord, TabAgentRegistry, TabAgentStatus};
    use std::os::unix::net::UnixListener;

    let mut app = headless_app();
    app.ac_mut().agent_connected = true;
    // Registry row persisted by the PREVIOUS run under ITS numbering (tab 7)
    // with a still-live socket: the reused master slot must reattach to it,
    // not fall into the resume-respawn path.
    let dir = tempfile::tempdir().unwrap();
    let live_sock = dir.path().join("old-tab7.sock");
    let _listener = UnixListener::bind(&live_sock).unwrap();
    let rpath = dir.path().join("registry.json");
    let mut reg = TabAgentRegistry::new();
    reg.upsert(TabAgentRecord {
        tab_id: 7,
        pid: Some(std::process::id()),
        socket_path: live_sock.clone(),
        session_key: Some("sess-a".into()),
        tab_name: Some("one".into()),
        workspace_id: Some("ws-mismatch".into()),
        updated_unix_s: 1,
        status: TabAgentStatus::Live,
    });
    reg.store(&rpath).unwrap();

    let manifest = App::test_workspace_manifest(
        "ws-mismatch",
        vec![WorkspaceTabEntry {
            tab_id: 7,
            session_key: Some("sess-a".into()),
            name: Some("one".into()),
            summary: None,
        }],
        0,
    );
    app.apply_workspace_manifest_with_registry(&manifest, &rpath);

    let master = app.conn_for(TabId::MASTER).expect("master slot");
    assert_eq!(
        master.socket_path.as_deref(),
        Some(live_sock.as_path()),
        "the reused master slot must reattach the live detached socket even \
         when the stored tab id (7) does not match the local slot (0)"
    );
    assert!(
        master.pending_attach,
        "live reattach must be scheduled for the reused master slot"
    );
    assert!(
        master.pending_session_resume.is_none(),
        "live reattach must not latch resume_session for the running owner"
    );
}

// ── Item 5: dead sub-agents are visible, sends are never swallowed ───────

#[test]
fn detached_and_dead_subagent_names_are_visually_distinct() {
    use super::app_subagent_panel::controller_subagent_panel_helpers::status_colored_name;
    use crate::components::theme;
    assert_eq!(
        status_colored_name("detached", "w1"),
        theme::dim("w1"),
        "a detached roster entry must render dimmed, not like a live agent \
         (#1461 liveness states)"
    );
    assert_eq!(
        status_colored_name("dead", "w1"),
        theme::red("w1"),
        "a dead roster entry must render red, not like a live agent \
         (#1461 liveness states)"
    );
}

#[tokio::test]
async fn sending_to_an_unattached_subagent_surfaces_a_visible_error() {
    let mut app = headless_app();
    // Rehydrated roster entry that is detached AND unreachable: no usable
    // child socket exists, so attach-on-demand (#1466 round 2) has no route
    // and the send must keep erroring. The detached-but-REACHABLE side is
    // pinned in `app_fix_pass_1466_round2_tests` (live socket → delivered).
    app.update_subagent_bar(vec![subagent_info("w1", "detached")]);
    app.select_agent(Some("w1"));
    // Note: a feed channel may exist and even accept the enqueue — with a
    // dead/detached child nothing consumes it, which is exactly the silent
    // swallow being fixed. Liveness must be judged from the roster state.
    app.handle_submit("hello there");

    // The outcome must SPECIFICALLY reference the failed delivery and the
    // agent — an incidental unrelated notification must not pass.
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
    let surfaced = |s: &str| s.contains("not delivered") && s.contains("w1");
    assert!(
        surfaced(&status) || surfaced(&last_note),
        "a message to a dead/unattached sub-agent must surface a delivery \
         failure naming the agent; last status={status:?}, last \
         notification={last_note:?}"
    );
}

// ── Item 6: background-tab spinners keep animating ───────────────────────

#[tokio::test]
async fn animation_service_ticks_background_tab_spinners() {
    let mut app = two_tab_app();
    let mut coalescer = super::app_event_loop::StreamRenderCoalescer::default();
    route_and_render(&mut app, &mut coalescer, 1, Event::AgentStart);
    assert!(
        app.conn_for(TabId(1))
            .and_then(|c| c.spinner.as_ref())
            .is_some(),
        "precondition: a routed background AgentStart creates that tab's spinner"
    );
    let frame_before = app
        .conn_for(TabId(1))
        .and_then(|c| c.spinner.as_ref())
        .map(|s| s.frame_index())
        .unwrap();

    let mut kitty_done = true;
    let repaint = app.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());

    let frame_after = app
        .conn_for(TabId(1))
        .and_then(|c| c.spinner.as_ref())
        .map(|s| s.frame_index())
        .unwrap();
    assert_ne!(
        frame_after, frame_before,
        "the animation service must advance BACKGROUND tab spinners so the \
         tab bar keeps animating"
    );
    assert!(
        repaint,
        "a busy background tab must schedule the bar-cadence repaint from the \
         animation tick (background tokens still schedule none)"
    );
}
