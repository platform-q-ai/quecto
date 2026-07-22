//! Resume/prompt persistence dispatch regression tests split from
//! `uds_dispatch_cov_tests.rs` to keep coverage files below the line-count gate.

use super::cov_tests::Fixture;
use super::{dispatch_command, handle_resume_session};
use crate::domain::session::{Session, SessionStore};
use crate::interface::cli::protocol::AgentCommand;

#[tokio::test]
async fn prompt_persists_user_message_before_assistant_reply() {
    // Regression: if a TUI/session is closed or interrupted before the provider
    // returns an assistant message, the first user message must still be
    // durable and visible after /resume.
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        super::persist_user_prompt_before_run(&mut ctx, "first only")
            .await
            .unwrap();
    }

    let loaded = fx.store.load("cli:test").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].role, crate::domain::message::Role::User);
    assert_eq!(loaded.messages[0].content, "first only");

    fx.store
        .save(&Session {
            key: Session::build_key("cli", "saved-one"),
            messages: loaded.messages.clone(),
            workflow_run: None,
        })
        .await
        .unwrap();
    {
        let mut ctx = fx.ctx();
        assert!(
            !handle_resume_session(&mut ctx, Some("rs"), "resume_session", "saved-one".into())
                .await
        );
    }
    assert_eq!(fx.messages.len(), 1);
    assert_eq!(fx.messages[0].content, "first only");
}

#[tokio::test]
async fn dispatch_unknown_history_cursor_is_rejected() {
    let mut fx = Fixture::new();
    fx.messages = vec![crate::domain::message::Message::user("newest")];
    assert!(
        !dispatch_command(
            AgentCommand::GetMessages {
                id: Some("stale-page".into()),
                count: None,
                before: Some("unknown-message-id".into()),
                agent_id: None,
            },
            &mut fx.ctx(),
        )
        .await
    );
}

#[tokio::test]
async fn dispatch_agent_targeted_tail_without_registry_emits_error() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::GetMessagesTail {
        id: Some("inspector-tail:worker".into()),
        count: 5,
        agent_id: Some("worker".into()),
    };
    // subagent_registry is None: the early intercept still handles it.
    assert!(!dispatch_command(cmd, &mut fx.ctx()).await);
}

#[tokio::test]
async fn refresh_conversation_snapshot_clones_current_messages() {
    let mut fx = Fixture::new();
    fx.messages = vec![
        crate::domain::message::Message::user("hello"),
        crate::domain::message::Message::assistant("hi", vec![]),
    ];
    let ctx = fx.ctx();
    assert!(
        ctx.conversation_snapshot.read().await.messages.is_empty(),
        "starts empty"
    );
    crate::interface::cli::uds_snapshots::refresh_conversation_snapshot(&ctx).await;
    let snap = ctx.conversation_snapshot.read().await;
    assert_eq!(snap.messages.len(), 2, "snapshot mirrors current messages");
    crate::interface::cli::uds_snapshots::refresh_state_snapshot(&ctx).await;
    assert_eq!(ctx.state_snapshot.read().await.message_count, 2);
}
