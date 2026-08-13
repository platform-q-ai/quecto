use crate::protocol::client::Client;
use crate::shell::app::App;
use crate::shell::connection::TabId;
use crate::shell::tab_registry::TabAgentRegistry;
use crate::shell::terminal::Terminal;
use crate::shell::workspace_manifest::WorkspaceManifestStore;

fn app() -> App {
    let client = Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    App::new(term, client)
}

#[test]
fn open_placeholder_allocates_and_focuses() {
    let mut a = app();
    assert_eq!(a.tabs.len(), 1);
    let t1 = a.open_placeholder_tab(Some("two".into()));
    assert_eq!(t1, TabId(1));
    assert_eq!(a.tabs.len(), 2);
    assert_eq!(a.active_tab, TabId(1));
    assert_eq!(a.ac().name.as_deref(), Some("two"));
    assert!(!a.ac().agent_connected);
}

#[test]
fn switch_tab_next_prev_wraps() {
    let mut a = app();
    let _ = a.open_placeholder_tab(None);
    let _ = a.open_placeholder_tab(None);
    assert_eq!(a.active_tab, TabId(2));
    assert_eq!(a.switch_tab_prev(), TabId(1));
    assert_eq!(a.switch_tab_prev(), TabId(0));
    assert_eq!(a.switch_tab_prev(), TabId(2));
    assert_eq!(a.switch_tab_next(), TabId(0));
}

#[test]
fn close_tab_detaches_and_refocuses() {
    let mut a = app();
    let t1 = a.open_placeholder_tab(None);
    let watch = a.close_tab(t1, false).unwrap();
    assert!(watch.is_none());
    assert_eq!(a.tabs.len(), 1);
    assert_eq!(a.active_tab, TabId::MASTER);
}

#[test]
fn close_last_tab_refused() {
    let mut a = app();
    assert!(a.close_tab(TabId::MASTER, false).is_err());
}

#[test]
fn close_active_prefers_previous_id() {
    let mut a = app();
    let _t1 = a.open_placeholder_tab(None);
    let t2 = a.open_placeholder_tab(None);
    assert_eq!(a.active_tab, t2);
    a.close_tab(t2, false).unwrap();
    assert_eq!(a.active_tab, TabId(1));
}

#[test]
fn registry_and_manifest_snapshots_track_tabs() {
    let mut a = app();
    a.open_placeholder_tab(Some("b".into()));
    let reg = a.registry_snapshot(Some("ws"));
    assert_eq!(reg.agents.len(), 2);
    assert_eq!(reg.agents[1].tab_name.as_deref(), Some("b"));
    let man = a.workspace_manifest_snapshot("ws");
    assert_eq!(man.tabs.len(), 2);
    assert_eq!(man.active_index, 1);

    let dir = tempfile::tempdir().unwrap();
    let rpath = dir.path().join("r.json");
    let mpath = dir.path().join("m.json");
    a.persist_durability_snapshot("ws", &rpath, &mpath);
    let loaded_r = TabAgentRegistry::load(&rpath);
    assert_eq!(loaded_r.agents.len(), 2);
    let loaded_m = WorkspaceManifestStore::load(&mpath);
    assert_eq!(loaded_m.get("ws").unwrap().tabs.len(), 2);
}

#[test]
fn switch_unknown_tab_is_false() {
    let mut a = app();
    assert!(!a.switch_tab(TabId(9)));
    assert_eq!(a.active_tab, TabId::MASTER);
}

#[test]
fn attach_connection_marks_tab_connected() {
    let mut a = app();
    let tab = a.open_placeholder_tab(None);
    assert!(!a.conn_for(tab).unwrap().agent_connected);
    let conn = crate::shell::connection::Connection::placeholder(tab);
    a.attach_connection_to_tab(tab, conn, None);
    assert!(a.conn_for(tab).unwrap().agent_connected);
    assert_eq!(a.active_tab, tab);
}

#[test]
fn registry_snapshot_includes_live_socket_pid_and_session_key() {
    let mut a = app();
    let tab = a.open_placeholder_tab(Some("worker".into()));
    let state = a.conn_mut(tab).unwrap();
    state.agent_connected = true;
    state.socket_path = Some(std::path::PathBuf::from("/tmp/quecto-tab-1.sock"));
    state.session_key = Some("cli:worker-1".into());
    state.child_pid = Some(4242);
    let reg = a.registry_snapshot(Some("ws"));
    let record = reg
        .agents
        .iter()
        .find(|r| r.tab_id == tab.0)
        .expect("tab record");
    assert_eq!(record.pid, Some(4242));
    assert_eq!(
        record.socket_path,
        std::path::PathBuf::from("/tmp/quecto-tab-1.sock")
    );
    assert_eq!(record.session_key.as_deref(), Some("cli:worker-1"));
    let man = a.workspace_manifest_snapshot("ws");
    let entry = man.tabs.iter().find(|t| t.tab_id == tab.0).unwrap();
    assert_eq!(entry.session_key.as_deref(), Some("cli:worker-1"));
}

#[test]
fn close_tab_with_kill_returns_child_watch_for_terminate() {
    let mut a = app();
    let t1 = a.open_placeholder_tab(None);
    a.conn_mut(t1).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(99)));
    let returned = a.close_tab(t1, true).expect("close ok");
    assert!(
        returned.is_some(),
        "AC3a: close must hand back the tab ChildWatch so the agent can be terminated"
    );
    assert_eq!(returned.unwrap().pid(), Some(99));
}

#[test]
fn tab_close_slash_command_requests_kill_not_detach() {
    let mut a = app();
    let t1 = a.open_placeholder_tab(None);
    a.active_tab = t1;
    a.handle_submit("/tab-close");
    assert_eq!(a.tabs.len(), 1, "tab closed");
    let msgs = a.notifications.messages().join("\n");
    assert!(
        !msgs.to_lowercase().contains("detach"),
        "AC3a: /tab-close must terminate, not detach; got {msgs:?}"
    );
}

#[test]
fn switch_tab_resyncs_panel_nav_to_active_roster() {
    let mut a = app();
    let t1 = a.open_placeholder_tab(None);
    a.switch_tab(TabId::MASTER);
    a.ac_mut().roster.tracked.insert(
        "child-a".into(),
        crate::agents::roster::TrackedSubagent::new(crate::protocol::client::SubagentInfoEvent {
            agent_uuid: None,
            display_name: None,
            agent_id: "child-a".into(),
            status: "running".into(),
            last_tool: None,
            last_error: None,
            pid: 0,
            socket_path: None,
            parent_id: None,
            workflow: None,
            read_only: false,
            execution_backend: None,
            environment: None,
        }),
    );
    a.subagents.panel_nav.set_selected(1);
    a.subagents.panel_nav_key = Some("agent:child-a".into());
    assert!(a.switch_tab(t1));
    assert_eq!(
        a.subagents.panel_nav.selected(),
        0,
        "panel cursor must resync to the newly focused tab roster"
    );
}

#[test]
fn collect_owned_child_watches_includes_every_tab() {
    let mut a = app();
    let t1 = a.open_placeholder_tab(None);
    a.conn_mut(TabId::MASTER).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(1)));
    a.conn_mut(t1).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(2)));
    let watches = a.take_all_child_exit_watches();
    assert_eq!(watches.len(), 2, "kill-on-exit must see every tab watch");
}

#[test]
fn open_tab_records_spawn_intent_not_dead_placeholder_only() {
    let mut a = app();
    a.handle_submit("/tab-new");
    assert_eq!(a.tabs.len(), 2);
    let tab = a.active_tab;
    assert_ne!(tab, TabId::MASTER);
    assert!(
        a.tab_has_pending_attach(tab),
        "AC1/AC2: /tab-new must start a non-blocking live agent attach path, not only a dead placeholder"
    );
}

#[test]
fn stale_attach_outcome_is_rejected_after_close() {
    let mut a = app();
    let tab = a.open_placeholder_tab(None);
    a.mark_tab_pending_attach(tab);
    let generation = a.bump_attach_generation(tab);
    assert_ne!(generation, 0);
    a.close_tab(tab, false).unwrap();
    // Recycle the same numeric id with a fresh generation.
    let tab2 = a.open_placeholder_tab(None);
    assert_eq!(tab2, tab, "allocator reuses lowest free id");
    a.mark_tab_pending_attach(tab2);
    let generation2 = a.bump_attach_generation(tab2);
    assert_ne!(generation2, generation);
    // Stale outcome from the closed tab must not attach into the new occupant.
    a.apply_tab_attach_outcome(super::TabAttachOutcome {
        tab: tab2,
        generation,
        result: Err("stale".into()),
        child_watch: None,
    });
    assert_eq!(a.conn_for(tab2).unwrap().attach_generation, generation2);
    assert!(
        a.conn_for(tab2).unwrap().pending_attach,
        "F2: stale outcome must not clear the recycled tab's pending_attach"
    );
}

#[test]
fn attach_connection_does_not_steal_focus() {
    let mut a = app();
    let t1 = a.open_placeholder_tab(None);
    a.switch_tab(TabId::MASTER);
    assert_eq!(a.active_tab, TabId::MASTER);
    let conn = crate::shell::connection::Connection::placeholder(t1);
    a.attach_connection_to_tab(t1, conn, None);
    assert_eq!(
        a.active_tab,
        TabId::MASTER,
        "F9: background attach must not steal focus after user navigated away"
    );
    assert!(a.conn_for(t1).unwrap().agent_connected);
}

#[test]
fn switch_tab_clears_open_pending_and_swaps_editor_draft() {
    let mut a = app();
    let t1 = a.open_placeholder_tab(None);
    a.editor.set_text("draft-t1");
    a.inference.model_registry.open_pending = true;
    a.switch_tab(TabId::MASTER);
    assert!(
        !a.inference.model_registry.open_pending,
        "F10: switch must clear global open_pending"
    );
    assert_eq!(a.editor.text(), "", "master starts empty");
    a.editor.set_text("draft-master");
    a.switch_tab(t1);
    assert_eq!(
        a.editor.text(),
        "draft-t1",
        "F11: per-tab editor draft restored"
    );
}

#[test]
fn pending_attach_queues_prompt_not_disconnect_refusal() {
    let mut a = app();
    let tab = a.open_placeholder_tab(None);
    a.mark_tab_pending_attach(tab);
    a.handle_submit("hello while connecting");
    let status = a
        .ac()
        .master_session
        .chat
        .last_status_text()
        .unwrap_or("")
        .to_lowercase();
    assert!(
        !status.contains("restart"),
        "F7: must not show disconnected/restart UX while connecting: {status}"
    );
    assert!(
        status.contains("connecting"),
        "F7: connecting UX expected: {status}"
    );
    assert_eq!(
        a.ac().queued_prompts,
        vec!["hello while connecting".to_string()]
    );
}

#[test]
fn tab_close_help_says_terminate_not_detach() {
    let cmds = super::super::builtin_commands();
    let close = cmds
        .iter()
        .find(|c| c.name == "tab-close")
        .expect("tab-close");
    assert!(
        close.description.to_lowercase().contains("terminate"),
        "F12: help must say terminate: {}",
        close.description
    );
    assert!(
        !close.description.to_lowercase().contains("detach"),
        "F12: help must not say detach: {}",
        close.description
    );
}

#[test]
fn single_resume_path_after_attach() {
    let mut a = app();
    let tab = a.open_placeholder_tab(None);
    a.conn_mut(tab).unwrap().pending_session_resume = Some("sess-x".into());
    let (live, mut rx) = crate::shell::connection::Connection::live_for_tests();
    a.attach_connection_to_tab(tab, live, None);
    // Drain any commands; resume must appear exactly once.
    let mut resumes = 0;
    while let Ok(line) = rx.try_recv() {
        if line.contains("resume_session") {
            resumes += 1;
        }
    }
    assert_eq!(resumes, 1, "F6: exactly one resume_session after attach");
    assert!(
        a.conn_for(tab).unwrap().pending_session_resume.is_none(),
        "pending latch cleared"
    );
}

#[test]
fn persist_merges_open_tabs_and_keeps_detached_live_registry_rows() {
    use crate::shell::tab_registry::{TabAgentRecord, TabAgentStatus};
    use std::os::unix::net::UnixListener;

    let mut a = app();
    // Fresh TUI: only MASTER is open, with a new socket/session.
    let dir = tempfile::tempdir().unwrap();
    let master_sock = dir.path().join("master-new.sock");
    let detached_sock = dir.path().join("tab1-live.sock");
    let _master_listener = UnixListener::bind(&master_sock).unwrap();
    let _detached_listener = UnixListener::bind(&detached_sock).unwrap();

    a.ac_mut().agent_connected = true;
    a.ac_mut().socket_path = Some(master_sock.clone());
    a.ac_mut().session_key = Some("cli:new-master".into());
    a.ac_mut().child_pid = Some(std::process::id());

    let rpath = dir.path().join("registry.json");
    let mpath = dir.path().join("manifest.json");
    let mut preexisting = TabAgentRegistry::new();
    preexisting.upsert(TabAgentRecord {
        tab_id: 0,
        pid: Some(std::process::id()),
        socket_path: dir.path().join("old-master.sock"),
        session_key: Some("cli:old-master".into()),
        tab_name: Some("old".into()),
        workspace_id: Some("ws".into()),
        updated_unix_s: 1,
        status: TabAgentStatus::Live,
    });
    preexisting.upsert(TabAgentRecord {
        tab_id: 1,
        pid: Some(std::process::id()),
        socket_path: detached_sock.clone(),
        session_key: Some("cli:tab1".into()),
        tab_name: Some("two".into()),
        workspace_id: Some("ws".into()),
        updated_unix_s: 1,
        status: TabAgentStatus::Live,
    });
    preexisting.store(&rpath).unwrap();

    a.persist_durability_snapshot("ws", &rpath, &mpath);

    let loaded = TabAgentRegistry::load(&rpath);
    let tab1 = loaded
        .agents
        .iter()
        .find(|r| r.tab_id == 1)
        .expect("AC3b/AC6: detached-but-live tab 1 must survive persist of a one-tab restart");
    assert_eq!(tab1.session_key.as_deref(), Some("cli:tab1"));
    assert_eq!(tab1.socket_path, detached_sock);
    let tab0_new = loaded
        .agents
        .iter()
        .find(|r| r.tab_id == 0 && r.session_key.as_deref() == Some("cli:new-master"))
        .expect("open master must remain in registry");
    assert_eq!(tab0_new.socket_path, master_sock);
    let tab0_old = loaded
        .agents
        .iter()
        .find(|r| r.tab_id == 0 && r.session_key.as_deref() == Some("cli:old-master"))
        .expect("AC3b/AC6: detached live master must survive fresh master persist");
    assert_eq!(tab0_old.socket_path, dir.path().join("old-master.sock"));
}

#[test]
fn failed_attach_clears_deferred_resume_so_prompts_are_not_queued_forever() {
    let mut a = app();
    let tab = a.open_placeholder_tab(None);
    a.mark_tab_pending_attach(tab);
    a.conn_mut(tab).unwrap().pending_session_resume = Some("cli:work".into());
    let generation = a.bump_attach_generation(tab);
    a.apply_tab_attach_outcome(super::TabAttachOutcome {
        tab,
        generation,
        result: Err("spawn failed".into()),
        child_watch: None,
    });
    assert!(
        !a.conn_for(tab).unwrap().pending_attach,
        "attach flag must clear on final failure"
    );
    assert!(
        a.conn_for(tab).unwrap().pending_session_resume.is_none(),
        "final attach failure must drop the resume latch so later prompts are not treated as still-connecting"
    );
    assert!(
        !a.tab_has_pending_attach(tab),
        "failed attach must not keep tab_has_pending_attach true"
    );
    a.handle_submit("hello after failed attach");
    let status = a
        .ac()
        .master_session
        .chat
        .last_status_text()
        .unwrap_or("")
        .to_lowercase();
    assert!(
        !status.contains("connecting"),
        "must not queue forever as connecting after final attach failure: {status}"
    );
    assert!(
        a.ac().queued_prompts.is_empty(),
        "prompt must not be latched as a connecting queue after attach failure"
    );
}
