//! #1466 RED-phase unit tests — multi-session TUI phase 5.
//!
//! Covers: background-tab paint gating (decision 3), spinner/unread-dot
//! semantics (decision 4), per-tab retained-session cap of 30 (decision 2),
//! UUID workspace identity + label/last-active resume + orphan GC
//! (decision 1), and kitty alias ↔ Alt primary parity (decision 5).

use super::*;
use crate::protocol::client::Event;
use crate::shell::connection::{SourcedEvent, TabId};
use crate::shell::keys::{Key, parse_key};
use crate::shell::tab_registry::TabAgentRegistry;
use crate::shell::terminal::Terminal;
use crate::shell::workspace_manifest::{
    WorkspaceManifest, WorkspaceManifestStore, WorkspaceTabEntry, generate_workspace_id,
};

fn headless_app() -> App {
    let client = crate::protocol::client::Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    let mut app = App::new(term, client);
    app.suppress_paint = true;
    app
}

fn two_tab_app() -> App {
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

fn token(text: &str) -> Event {
    Event::Token {
        token: text.to_string(),
    }
}

fn agent_end() -> Event {
    Event::AgentEnd {
        messages: Vec::new(),
        message_refs: Vec::new(),
    }
}

// ── Decision 3: background-tab stream events must not schedule paints ──

#[tokio::test]
async fn background_tab_tokens_paint_nothing_and_defer_nothing() {
    let mut app = two_tab_app();
    let mut coalescer = super::app_event_loop::StreamRenderCoalescer::default();
    assert_eq!(app.active_tab, TabId::MASTER);

    for i in 0..20 {
        route_and_render(&mut app, &mut coalescer, 1, token(&format!("bg-{i} ")));
    }

    assert_eq!(
        app.rendered_frames, 0,
        "background-tab tokens must not paint any frame (#1466 decision 3)"
    );
    assert!(
        coalescer.pending_deadline().is_none(),
        "background-tab tokens must not schedule a deferred paint / loop wakeup"
    );
}

#[tokio::test]
async fn background_tab_token_still_sets_unread_dot() {
    let mut app = two_tab_app();
    let mut coalescer = super::app_event_loop::StreamRenderCoalescer::default();

    route_and_render(&mut app, &mut coalescer, 1, token("hello"));

    assert!(
        app.tab_unread(TabId(1)),
        "background output must set the tab's unread dot even though it does not paint"
    );
    assert!(
        !app.tab_unread(TabId::MASTER),
        "the active tab must not be marked unread by another tab's output"
    );
}

#[tokio::test]
async fn active_tab_tokens_still_paint_through_the_coalescer() {
    let mut app = two_tab_app();
    let mut coalescer = super::app_event_loop::StreamRenderCoalescer::default();

    route_and_render(&mut app, &mut coalescer, 0, token("fg"));

    assert_eq!(
        app.rendered_frames, 1,
        "active-tab tokens must keep painting exactly as before"
    );
}

// ── Scope 2: needs_animation_tick considers any-tab-busy ──

#[tokio::test]
async fn needs_animation_tick_false_when_all_tabs_idle() {
    // The negative side of "any tab busy": merely having a second tab open
    // must not keep the loop awake (idle-efficiency acceptance, #1466).
    let app = two_tab_app();
    assert!(
        !app.needs_animation_tick(false),
        "N idle background tabs must add no animation wakeups vs single-tab idle"
    );
}

#[tokio::test]
async fn needs_animation_tick_true_while_only_background_tab_busy() {
    let mut app = two_tab_app();
    let mut coalescer = super::app_event_loop::StreamRenderCoalescer::default();
    route_and_render(&mut app, &mut coalescer, 1, Event::AgentStart);

    assert!(
        !app.ac().agent_state.is_running(),
        "precondition: the ACTIVE tab is idle"
    );
    assert!(
        app.needs_animation_tick(false),
        "a running background turn must keep the tab-bar spinner animating (#1466 scope 2)"
    );
}

// ── Decision 4: spinner / unread-dot semantics ──

#[tokio::test]
async fn background_turn_end_sets_unread_and_clears_spinner() {
    let mut app = two_tab_app();
    let mut coalescer = super::app_event_loop::StreamRenderCoalescer::default();

    route_and_render(&mut app, &mut coalescer, 1, Event::AgentStart);
    assert!(
        app.tab_spinner_active(TabId(1)),
        "a running background turn shows the tab spinner"
    );

    route_and_render(&mut app, &mut coalescer, 1, agent_end());
    assert!(
        !app.tab_spinner_active(TabId(1)),
        "an ended/aborted background turn clears the tab spinner"
    );
    assert!(
        app.tab_unread(TabId(1)),
        "an ended/aborted background turn sets the unread dot (no separate aborted state)"
    );
}

#[tokio::test]
async fn client_abort_clears_spinner_and_backgrounded_agent_end_sets_unread() {
    // The real abort path (#1466 decision 4): abort is client-driven on the
    // FOCUSED tab (`handle_abort` → `agent_state.abort()`); the aborted
    // turn's `AgentEnd` then lands after the user has switched away.
    let mut app = two_tab_app();
    let mut coalescer = super::app_event_loop::StreamRenderCoalescer::default();

    assert!(app.switch_tab(TabId(1)));
    route_and_render(&mut app, &mut coalescer, 1, Event::AgentStart);
    assert!(
        app.tab_spinner_active(TabId(1)),
        "precondition: spinner lit"
    );

    app.handle_abort();
    assert!(
        !app.tab_spinner_active(TabId(1)),
        "abort must clear the tab spinner immediately (Aborting shows no spinner)"
    );

    assert!(app.switch_tab(TabId::MASTER));
    route_and_render(&mut app, &mut coalescer, 1, agent_end());
    assert!(
        !app.tab_spinner_active(TabId(1)),
        "the aborted turn's trailing AgentEnd must not relight the spinner"
    );
    assert!(
        app.tab_unread(TabId(1)),
        "an aborted background turn sets the unread dot (#1466 decision 4)"
    );
}

#[tokio::test]
async fn switching_to_a_tab_clears_its_unread_dot() {
    let mut app = two_tab_app();
    // Pre-seed directly so this clear assertion cannot pass vacuously.
    app.conn_mut(TabId(1)).unwrap().unread_output = true;
    assert!(app.tab_unread(TabId(1)), "precondition: dot set");

    assert!(app.switch_tab(TabId(1)));

    assert!(
        !app.tab_unread(TabId(1)),
        "switching to a tab must clear its unread dot (#1466 decision 4)"
    );
}

// ── Decision 2: retained-session cap becomes 30 per tab ──

#[test]
fn retained_session_cap_is_30() {
    assert_eq!(
        crate::agents::focus::MAX_RETAINED_SESSIONS,
        30,
        "MAX_RETAINED_SESSIONS must be 30 per tab (#1466 decision 2)"
    );
}

#[tokio::test]
async fn exactly_30_sessions_survive_with_no_eviction() {
    // At-limit boundary: an off-by-one that evicts AT 30 must fail here.
    let mut app = two_tab_app();
    for i in 0..30 {
        app.ensure_session(&format!("agent-{i:03}"));
    }
    assert_eq!(
        app.ac().roster.sessions.len(),
        30,
        "exactly 30 retained sessions must survive without eviction"
    );
    for i in 0..30 {
        assert!(
            app.ac()
                .roster
                .sessions
                .contains_key(&format!("agent-{i:03}")),
            "no session may be evicted while at (not past) the cap: agent-{i:03}"
        );
    }
}

#[tokio::test]
async fn a_tab_retains_up_to_30_sessions_and_eviction_stays_per_tab() {
    let mut app = two_tab_app();
    // Seed retained sessions on the OTHER tab so cross-tab isolation is
    // actually exercised (a global cap or cross-tab eviction must fail this).
    assert!(app.switch_tab(TabId(1)));
    for i in 0..3 {
        app.ensure_session(&format!("bg-agent-{i:03}"));
    }
    assert!(app.switch_tab(TabId::MASTER));

    app.ac_mut().roster.active_agent_id = Some("agent-000".into());
    for i in 0..31 {
        app.ensure_session(&format!("agent-{i:03}"));
    }
    assert_eq!(
        app.ac().roster.sessions.len(),
        30,
        "one tab must retain up to 30 sessions before evicting (#1466 decision 2)"
    );
    assert_eq!(
        app.conn_for(TabId(1)).unwrap().roster.sessions.len(),
        3,
        "retention is per tab: eviction on tab 0 must not touch tab 1's sessions"
    );
}

// ── Decision 1: UUID workspace identity + label/last-active + orphan GC ──

#[test]
fn generated_workspace_id_is_a_hyphenated_uuid_v4() {
    let id = generate_workspace_id();
    assert_eq!(
        id.len(),
        36,
        "workspace id must be a hyphenated UUID: {id:?}"
    );
    for idx in [8, 13, 18, 23] {
        assert_eq!(
            id.as_bytes().get(idx),
            Some(&b'-'),
            "hyphen expected at {idx} in {id:?}"
        );
    }
    assert_eq!(
        id.as_bytes().get(14),
        Some(&b'4'),
        "workspace id must be UUID version 4: {id:?}"
    );
    assert_ne!(
        generate_workspace_id(),
        generate_workspace_id(),
        "workspace ids must be unique per mint"
    );
}

fn labelled_ws(id: &str, label: &str, session_key: Option<&str>) -> WorkspaceManifest {
    WorkspaceManifest {
        workspace_id: id.into(),
        label: label.into(),
        last_active_unix_s: 1_755_000_000,
        active_index: 0,
        tabs: vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: session_key.map(str::to_string),
            name: None,
        }],
        updated_unix_s: 1_755_000_000,
    }
}

#[tokio::test]
async fn resume_selector_lists_workspaces_by_label_and_last_active() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifests.json");
    let uuid = "3f2b6c1e-9d4a-4b6f-8c2d-5e7a1b9c0d42";
    let mut store = WorkspaceManifestStore::new();
    store.upsert(labelled_ws(uuid, "Auth spike", Some("sess-1")));
    store.store(&path).unwrap();

    let mut app = headless_app();
    app.open_resume_selector_with_workspaces(Vec::new(), &path, None);
    let sel = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .expect("selector");
    let item = &sel.items_for_tests()[0];

    assert!(
        item.label.contains("Auth spike"),
        "workspace rows list by human label (#1466 decision 1): {:?}",
        item.label
    );
    assert!(
        !item.label.contains(uuid) && !item.description.as_deref().unwrap_or("").contains(uuid),
        "workspace rows must not surface the raw UUID: {item:?}"
    );
    assert!(
        item.description.as_deref().unwrap_or("").contains("ago"),
        "workspace rows show relative last-active time: {:?}",
        item.description
    );
}

#[test]
fn gc_orphaned_removes_workspaces_with_no_sessions_and_no_registry_rows() {
    let mut store = WorkspaceManifestStore::new();
    store.upsert(labelled_ws("kept-ws", "Kept", Some("sess-1")));
    store.upsert(labelled_ws("orphan-ws", "Orphan", None));

    let removed = store.gc_orphaned(&TabAgentRegistry::new());

    assert_eq!(
        removed,
        vec!["orphan-ws".to_string()],
        "a workspace with no session keys and no registry rows is orphaned"
    );
    assert!(
        store.get("orphan-ws").is_none(),
        "orphaned workspaces are pruned from the store"
    );
    assert!(
        store.get("kept-ws").is_some(),
        "workspaces with resumable sessions survive GC"
    );
}

#[test]
fn gc_orphaned_keeps_session_less_workspace_with_live_registry_row() {
    // The other branch of the orphan AND-condition: no session key, but a
    // registry record still references the workspace — it must survive.
    let mut store = WorkspaceManifestStore::new();
    store.upsert(labelled_ws("registry-ws", "Registry-kept", None));

    let mut registry = TabAgentRegistry::new();
    registry.upsert(crate::shell::tab_registry::TabAgentRecord {
        tab_id: 0,
        pid: None,
        socket_path: std::path::PathBuf::new(),
        session_key: None,
        tab_name: None,
        workspace_id: Some("registry-ws".to_string()),
        updated_unix_s: 1_755_000_000,
        status: crate::shell::tab_registry::TabAgentStatus::Live,
    });

    let removed = store.gc_orphaned(&registry);

    assert!(
        removed.is_empty(),
        "a workspace referenced by a registry record is not orphaned: {removed:?}"
    );
    assert!(
        store.get("registry-ws").is_some(),
        "registry-referenced workspaces must survive GC even with no session keys"
    );
}

#[tokio::test]
async fn legacy_label_less_manifest_gets_a_non_uuid_fallback_row() {
    // Pre-#1466 manifests deserialize with an empty label; `/resume` must
    // still show something human, never the raw UUID.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifests.json");
    let uuid = "3f2b6c1e-9d4a-4b6f-8c2d-5e7a1b9c0d42";
    let mut store = WorkspaceManifestStore::new();
    store.upsert(labelled_ws(uuid, "", Some("sess-1")));
    store.store(&path).unwrap();

    let mut app = headless_app();
    app.open_resume_selector_with_workspaces(Vec::new(), &path, None);
    let sel = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .expect("selector");
    let item = &sel.items_for_tests()[0];

    assert!(
        item.label.contains("unnamed workspace"),
        "a legacy label-less workspace must render a human fallback: {:?}",
        item.label
    );
    assert!(
        !item.label.contains(uuid),
        "the fallback must not be the raw UUID: {:?}",
        item.label
    );
}

#[tokio::test]
async fn workspace_creation_mints_uuid_identity_and_non_empty_label() {
    let app = headless_app();
    assert_eq!(
        app.workspace_id.len(),
        36,
        "App startup must mint a UUID workspace identity: {:?}",
        app.workspace_id
    );
    assert!(
        !app.workspace_label.trim().is_empty(),
        "App startup must auto-generate a non-empty workspace label"
    );
    let manifest = app.workspace_manifest_snapshot(&app.workspace_id);
    assert_eq!(
        manifest.label, app.workspace_label,
        "the snapshot for our own workspace carries the auto-generated label"
    );
    assert!(
        manifest.last_active_unix_s > 0,
        "the snapshot stamps a last-active time"
    );
}

// ── Decision 5: kitty aliases map to the same actions as Alt primaries ──

#[tokio::test]
async fn alt_digit_primary_focuses_that_tab() {
    let mut app = two_tab_app();
    app.handle_key(Key::Alt('2'));
    assert_eq!(
        app.active_tab,
        TabId(1),
        "Alt+2 (primary) must focus the second tab"
    );
    app.handle_key(Key::Alt('1'));
    assert_eq!(
        app.active_tab,
        TabId::MASTER,
        "Alt+1 (primary) must focus the first tab"
    );
}

#[tokio::test]
async fn kitty_ctrl_digit_alias_matches_alt_digit_primary() {
    // Alias == primary is enforced at parse: the kitty Ctrl+digit sequences
    // yield the very same key as the Alt+digit primary, so the two can never
    // dispatch to different actions (#1466 decision 5).
    // CSI 49/50/57 ;5u = Ctrl + '1'/'2'/'9'.
    assert_eq!(parse_key(b"\x1b[50;5u").expect("Ctrl+2").0, Key::Alt('2'));
    assert_eq!(parse_key(b"\x1b[49;5u").expect("Ctrl+1").0, Key::Alt('1'));
    assert_eq!(parse_key(b"\x1b[57;5u").expect("Ctrl+9").0, Key::Alt('9'));

    let mut app = two_tab_app();
    let (key, _) = parse_key(b"\x1b[50;5u").expect("kitty Ctrl+2 parses");
    app.handle_key(key);
    assert_eq!(
        app.active_tab,
        TabId(1),
        "kitty Ctrl+2 must perform the same action as Alt+2 (#1466 decision 5)"
    );
    let (key, _) = parse_key(b"\x1b[49;5u").expect("kitty Ctrl+1 parses");
    app.handle_key(key);
    assert_eq!(
        app.active_tab,
        TabId::MASTER,
        "Ctrl+1 focuses the first tab"
    );
}

#[tokio::test]
async fn ctrl_digit_past_open_tab_count_is_a_no_op() {
    let mut app = two_tab_app();
    let (key, _) = parse_key(b"\x1b[57;5u").expect("kitty Ctrl+9 parses");
    // Assert parse identity so this test fails if the Ctrl+digit alias arm is
    // removed (Key::Unknown would also be a no-op, making the test vacuous).
    assert_eq!(key, Key::Alt('9'), "kitty Ctrl+9 must alias Alt+9");
    app.handle_key(key);
    assert_eq!(
        app.active_tab,
        TabId::MASTER,
        "a tab ordinal past the open tab count must not move focus"
    );
}

#[tokio::test]
async fn kitty_ctrl_tab_alias_cycles_to_next_tab() {
    let mut app = two_tab_app();
    // CSI 9;5u = Ctrl + Tab under the kitty keyboard protocol.
    let (key, _) = parse_key(b"\x1b[9;5u").expect("kitty Ctrl+Tab parses");
    assert_ne!(
        key,
        Key::Tab,
        "Ctrl+Tab must be distinguishable from plain Tab (panel focus toggle)"
    );
    // Alias == primary at parse: kitty Alt+Tab (CSI 9;3u) yields the same key.
    assert_eq!(
        key,
        parse_key(b"\x1b[9;3u").expect("kitty Alt+Tab parses").0,
        "the Ctrl+Tab alias must be the same key as the Alt+Tab primary"
    );
    app.handle_key(key);
    assert_eq!(
        app.active_tab,
        TabId(1),
        "kitty Ctrl+Tab must cycle to the next tab"
    );
}

#[tokio::test]
async fn kitty_ctrl_shift_tab_alias_cycles_to_previous_tab() {
    // Three tabs so direction is falsifiable: from MASTER, prev wraps to
    // TabId(2) while next would land on TabId(1) — two tabs cannot tell
    // switch_tab_prev apart from switch_tab_next.
    let mut app = two_tab_app();
    app.test_insert_disconnected_tab(2);
    // CSI 9;6u = Ctrl + Shift + Tab under the kitty keyboard protocol.
    let (key, _) = parse_key(b"\x1b[9;6u").expect("kitty Ctrl+Shift+Tab parses");
    assert_ne!(
        key,
        Key::BackTab,
        "Ctrl+Shift+Tab must be distinguishable from plain Shift+Tab"
    );
    // Alias == primary at parse: kitty Alt+Shift+Tab (CSI 9;4u) is the same key.
    assert_eq!(
        key,
        parse_key(b"\x1b[9;4u")
            .expect("kitty Alt+Shift+Tab parses")
            .0,
        "the Ctrl+Shift+Tab alias must be the same key as the Alt+Shift+Tab primary"
    );
    app.handle_key(key);
    assert_eq!(
        app.active_tab,
        TabId(2),
        "kitty Ctrl+Shift+Tab must cycle to the previous tab (wraps)"
    );
}
