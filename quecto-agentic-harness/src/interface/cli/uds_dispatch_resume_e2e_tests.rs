use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};
use crate::interface::cli::protocol::AgentCommand;

use super::cov_tests::Fixture;

#[tokio::test]
async fn e2e_resume_picker_lists_persisted_default_tui_chat_session() {
    let mut fx = Fixture::new();
    let persisted_key = crate::domain::session::Session::build_key("cli", "default");
    fx.store
        .save(&Session {
            key: persisted_key.clone(),
            messages: vec![Message::user("persisted message that /resume must offer")],
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();

    let listed = fx.store.list(None).await.unwrap();
    assert!(
        listed.iter().any(|summary| summary.key == persisted_key),
        "a TUI-owned persisted default session must be offered by bare /resume; listed={listed:?}"
    );

    let mut ctx = fx.ctx();
    assert!(
        !super::dispatch_command(
            AgentCommand::ListSessions {
                id: Some("resume-list".into()),
            },
            &mut ctx,
        )
        .await
    );
}
