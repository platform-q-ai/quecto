use super::cov_tests::Fixture;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};
use crate::interface::cli::protocol::AgentCommand;

#[tokio::test]
async fn set_effort_accepts_provider_vocabulary_and_rejects_invalid() {
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        assert!(ctx.agent.effort().is_none());
        assert!(
            !super::dispatch_command(
                AgentCommand::SetEffort {
                    id: Some("e1".into()),
                    effort: "xhigh".into(),
                },
                &mut ctx,
            )
            .await
        );
        assert_eq!(ctx.agent.effort().unwrap().as_str(), "xhigh");
    }
    fx.session
        .set_model("anthropic/claude-opus-4-6".to_string());
    {
        let mut ctx = fx.ctx();
        assert!(
            !super::dispatch_command(
                AgentCommand::SetEffort {
                    id: Some("e2".into()),
                    effort: "none".into(),
                },
                &mut ctx,
            )
            .await
        );
        assert_eq!(ctx.agent.effort().unwrap().as_str(), "xhigh");
    }
}

#[tokio::test]
async fn dispatch_fieldless_list_sessions_get_messages_and_() {
    let mut fx = Fixture::new();
    fx.store
        .save(&Session {
            key: "chat:one".into(),
            messages: vec![Message::user("hello")],
            workflow_run: None,
        })
        .await
        .unwrap();
    fx.messages.push(Message::user("live"));
    {
        let mut ctx = fx.ctx();
        assert!(
            !super::dispatch_command(
                AgentCommand::ListSessions {
                    id: Some("ls".into()),
                },
                &mut ctx,
            )
            .await
        );
        assert!(
            !super::dispatch_command(
                AgentCommand::GetMessages {
                    id: Some("gm".into()),
                    count: Some(1),
                    before: None,
                    agent_id: None,
                },
                &mut ctx,
            )
            .await
        );
    }
}

#[tokio::test]
async fn reload_without_provider_reload_configuration_errors() {
    let mut fx = Fixture::new();
    let mut ctx = fx.ctx();
    assert!(
        !super::dispatch_command(
            AgentCommand::Reload {
                id: Some("r".into()),
            },
            &mut ctx,
        )
        .await
    );
}
