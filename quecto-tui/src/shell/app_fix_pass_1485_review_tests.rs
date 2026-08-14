//! PR #1485 review-fix tests, split from `app_fix_pass_1466_tests.rs` to
//! keep that file inside the line-count baseline.

use super::app_fix_pass_1466_tests::{headless_app, subagent_info, two_tab_app};
use super::*;
use crate::protocol::client::Event;
use crate::shell::connection::{SourcedEvent, TabId};
use crate::shell::workspace_manifest::{WorkspaceManifestStore, WorkspaceTabEntry};

#[test]
fn rehydrated_roster_preserves_reported_liveness_states() {
    // #1461: the kernel probes cross-process liveness and reports
    // detached/dead in the roster snapshot; the TUI must PRESERVE those
    // states through apply, never coerce them toward running.
    let mut app = headless_app();
    app.update_subagent_bar(vec![
        subagent_info("w-detached", "detached"),
        subagent_info("w-dead", "dead"),
    ]);
    let status_of = |app: &App, id: &str| {
        app.ac()
            .roster
            .tracked
            .get(id)
            .map(|t| t.info.status.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        status_of(&app, "w-detached"),
        "detached",
        "a rehydrated detached entry must stay detached in the roster"
    );
    assert_eq!(
        status_of(&app, "w-dead"),
        "dead",
        "a rehydrated dead entry must stay dead in the roster"
    );
    // Falsifiability (PR #1485 review): the round-trip alone passes on
    // pre-PR code; the preserved states must also DRIVE the panel styling
    // this PR added, so reverting the status_colored_name arms fails here.
    use super::app_subagent_panel::controller_subagent_panel_helpers::status_colored_name;
    use crate::components::theme;
    assert_eq!(
        status_colored_name(&status_of(&app, "w-detached"), "w-detached"),
        theme::dim("w-detached"),
        "the roster-preserved detached state must render dimmed in the panel"
    );
    assert_eq!(
        status_colored_name(&status_of(&app, "w-dead"), "w-dead"),
        theme::red("w-dead"),
        "the roster-preserved dead state must render red in the panel"
    );
}

// ── PR #1485 review fixes ────────────────────────────────────────────────

#[tokio::test]
async fn background_turn_transitions_are_not_demoted_to_silent() {
    // Review finding (tab_activity.rs): the background render gate demoted
    // the turn-END event to Silent, and with the turn over
    // `needs_animation_tick` disarms — no repaint ever clears the bar
    // spinner, which stays frozen "running" forever.
    use super::app_event_loop::SourcedRender;
    let mut app = two_tab_app();

    let start = app.route_sourced(SourcedEvent::Tab(TabId(1), Event::AgentStart));
    assert_ne!(
        start,
        SourcedRender::Silent,
        "turn START on a background tab must schedule a paint (bar spinner lights)"
    );
    // Mid-turn events without a running-state transition stay silent — the
    // background token paint gate must not regress.
    let mid = app.route_sourced(SourcedEvent::Tab(
        TabId(1),
        Event::Token {
            token: "x".to_string(),
        },
    ));
    assert_eq!(
        mid,
        SourcedRender::Silent,
        "mid-turn background tokens must stay Silent (paint gate intact)"
    );
    let end = app.route_sourced(SourcedEvent::Tab(
        TabId(1),
        Event::AgentEnd {
            messages: Vec::new(),
            message_refs: Vec::new(),
        },
    ));
    assert_ne!(
        end,
        SourcedRender::Silent,
        "turn END on a background tab must schedule a paint so the frozen \
         spinner clears and the unread dot shows"
    );
    assert!(
        app.tab_unread(TabId(1)),
        "the transition paint must still mark the background tab unread"
    );
}

#[test]
fn resume_selector_sanitizes_persisted_snippet_escapes() {
    // Review finding (tab_lifecycle.rs snippet_of): manifests written before
    // the sanitize fix may carry raw escape bytes; the selector must never
    // replay them into the terminal.
    let mut app = headless_app();
    let dir = tempfile::tempdir().unwrap();
    let mpath = dir.path().join("m.json");
    let mut store = WorkspaceManifestStore::new();
    store.upsert(App::test_workspace_manifest(
        "ws-esc",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("s0".into()),
            name: None,
            summary: Some("evil\x1b]8;;file:///tmp/x\x07click me\x1b]8;;\x07".into()),
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
    let descriptions: Vec<_> = sel
        .items_for_tests()
        .iter()
        .map(|i| i.description.clone().unwrap_or_default())
        .collect();
    assert!(
        descriptions.iter().any(|d| d.contains("evil")),
        "the snippet text itself must survive; rows={descriptions:?}"
    );
    assert!(
        descriptions
            .iter()
            .all(|d| !d.contains('\x1b') && !d.contains('\x07')),
        "persisted snippet escape/control bytes must be stripped before the \
         selector renders them; rows={descriptions:?}"
    );
}
