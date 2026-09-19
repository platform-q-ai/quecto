use crate::protocol::client::Client;
use crate::shell::app::App;
use crate::shell::terminal::Terminal;

fn app() -> App {
    let client = Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    App::new(term, client)
}

#[test]
fn tab_commands_are_absent_from_help_registry() {
    let cmds = super::builtin_commands();
    for removed in ["tab-new", "tab-close", "tab-next", "tab-prev"] {
        assert!(
            cmds.iter().all(|c| c.name != removed),
            "removed tab command {removed} must not appear in help/autocomplete registry"
        );
    }
}

#[test]
fn new_command_resets_the_workspace_not_just_the_transcript() {
    let mut a = app();
    a.active_chat_mut()
        .add_entry(crate::components::chat::ChatEntry::User {
            text: "master old".into(),
        });
    a.subagents.panel_nav_key = Some("stale-agent".into());
    a.subagents.panel_nav.set_selected(3);
    // Seed what `/new` must clear, so the assertions below are not vacuous.
    a.ac_mut().session_key = Some("cli:old".into());

    a.handle_submit("/new");

    assert_eq!(a.ac().master_session.chat.entry_count(), 0);
    assert_eq!(a.ac().session_key, None);
    assert_eq!(a.subagents.panel_nav_key, None);
    assert_eq!(a.subagents.panel_nav.selected(), 0);
}

/// #1956 review: `/new` never hands the owned agent's watch out for
/// termination — the agent keeps serving the fresh session.
#[test]
fn new_command_leaves_the_owned_agent_watch_in_place() {
    let mut a = app();
    a.ac_mut().child_exit_watch = Some(crate::shell::child_watch::ChildWatch::for_tests(Some(1)));

    a.handle_submit("/new");

    assert_eq!(
        a.ac().child_exit_watch.as_ref().and_then(|w| w.pid()),
        Some(1),
        "the owned agent's watch survives /new"
    );
}

#[test]
fn removed_tab_slash_commands_are_unknown() {
    for command in ["/tab-new", "/tab-close", "/tab-next", "/tab-prev"] {
        let mut a = app();
        a.handle_submit(command);
        let has_unknown_status = a.ac().master_session.chat.entries().iter().any(|entry| {
            matches!(entry, crate::components::chat::ChatEntry::Status { text } if text.contains("Unknown slash command"))
        });
        assert!(
            has_unknown_status,
            "{command} should use ordinary unknown-command UX"
        );
    }
}
