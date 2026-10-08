//! #2192 regression pin: a turn drained from the pending queue — how a
//! parent's fast-acked `agent_cmd prompt` reaches a launched child — is
//! persisted when it ends (by the per-turn save, #2218), so a child that
//! later crashes leaves that turn to its parent.
use super::super::dispatch_test_env::DispatchTestEnv as Env;
use super::*;
use crate::application::sessions::ports::SessionStore;
use crate::domain::conversation::value_objects::message::Role;
use crate::domain::sessions::entities::session_identity::SessionIdentity;

async fn persisted_user_prompts(env: &Env) -> Vec<String> {
    let session = env
        .store
        .load(&SessionIdentity::from_persisted_key(
            env.session_key.clone(),
        ))
        .await
        .unwrap();
    session
        .map(|session| session.messages)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.content)
        .collect()
}

#[tokio::test]
async fn a_drained_turn_is_persisted_when_it_ends() {
    let mut env = Env::with_unselected_workflow();
    {
        let mut ctx = env.ctx();
        ctx.session
            .enqueue_control(Some("p2"), "follow_up", "queued prompt".into(), false);
        drain_and_run_pending(&mut ctx).await;
        assert!(ctx.session.drain_pending().is_empty(), "the prompt ran");
    }
    assert_eq!(persisted_user_prompts(&env).await, ["queued prompt"]);
}

#[tokio::test]
async fn nothing_is_persisted_when_nothing_was_drained() {
    let mut env = Env::with_unselected_workflow();
    {
        let mut ctx = env.ctx();
        drain_and_run_pending(&mut ctx).await;
    }
    assert!(persisted_user_prompts(&env).await.is_empty());
}
