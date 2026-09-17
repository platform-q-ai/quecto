use super::*;
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;

#[tokio::test]
async fn handles_over_the_file_store_list_what_the_store_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let handles = build_session_handles(SessionLoopInputs {
        base_dir: tmp.path().to_path_buf(),
        identity: SessionIdentity::from_persisted_key("cli:composed"),
        ephemeral: false,
        system_prompt: String::new(),
        spill_store: None,
        durable_prefix: crate::application::durable_prefix::DurablePrefixLatch::shared(),
        workflow_state: None,
        subagent_registry: None,
    });
    let mut session = Session::new(SessionIdentity::named_cli("composed").unwrap());
    session.messages.push(Message::user("hello"));
    handles.store.save(&session).await.unwrap();

    let listed = handles
        .list_sessions
        .list(crate::application::sessions::dto::SessionListScope::Global)
        .await
        .unwrap();
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].summary.key, "cli:composed");
    assert!(
        tmp.path().join("sessions/cli_composed.json").exists(),
        "the composed store writes the flat layout"
    );
}

fn production_inputs(base: &std::path::Path, identity: SessionIdentity) -> SessionLoopInputs {
    SessionLoopInputs {
        base_dir: base.to_path_buf(),
        identity,
        ephemeral: false,
        system_prompt: String::new(),
        spill_store: None,
        durable_prefix: crate::application::durable_prefix::DurablePrefixLatch::shared(),
        workflow_state: None,
        subagent_registry: None,
    }
}

#[tokio::test]
async fn production_graph_save_restart_discovery_and_startup_share_home_authority() {
    use crate::application::sessions::dto::SaveTrigger;
    use crate::application::sessions::dto::SessionListScope;
    use crate::domain::session_home::SessionHomeScope;
    let base = tempfile::tempdir().unwrap();
    let identity = SessionIdentity::named_cli("composition-home").unwrap();
    let handles = build_session_handles(production_inputs(base.path(), identity.clone()));
    let opened = handles.switch.resume.open_at_startup().await.unwrap();
    assert!(opened.messages.is_empty());
    let mut messages = vec![Message::user("COMPOSED-HOME")];
    handles
        .save_session
        .save(&mut messages, SaveTrigger::OrdinaryExit)
        .await
        .unwrap();
    handles.store.release(&identity);
    drop(handles);

    let restarted = build_session_handles(production_inputs(base.path(), identity.clone()));
    let listed = restarted
        .list_sessions
        .list(SessionListScope::Local)
        .await
        .unwrap();
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].summary.key, identity.runtime_key());
    assert!(listed.sessions[0].resume_eligible);
    let SessionHomeScope::Scoped(home) = &listed.sessions[0].home else {
        panic!("new save has authoritative home")
    };
    assert_eq!(
        home.execution_dir,
        std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap()
    );
    let restored = restarted.switch.resume.open_at_startup().await.unwrap();
    assert_eq!(restored.messages.len(), 1);
    assert_eq!(restored.messages[0].content, "COMPOSED-HOME");
    assert_eq!(
        restored.messages[0].role,
        crate::domain::message::Role::User
    );
    assert_eq!(restored.messages[0].ordinal, messages[0].ordinal);
    restarted.store.release(&identity);
}
