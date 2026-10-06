//! #2192: a panic contained to a tool call while it held the workflow
//! engine's lock poisons it. The loop reads the engine as it was left, the
//! same way everywhere: its nudges, its idle reason, its snapshot and the
//! save still work, as the `bash` guard does (see the workflow tool tests).
use super::super::dispatch_test_env::DispatchTestEnv as Env;
use crate::application::sessions::ports::SessionStore;
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::interface::cli::protocol::WorkflowIdleReason;
use crate::interface::cli::uds_workflow_nudge::{
    workflow_idle_reason, workflow_nudge_message, workflow_progress_fingerprint,
};

fn poison(env: &Env) {
    let workflow = env.workflow.clone();
    let _ = std::thread::spawn(move || {
        let _held = workflow.lock().unwrap();
        panic!("a tool call panicked under the workflow engine lock");
    })
    .join();
    assert!(env.workflow.is_poisoned());
}

#[tokio::test]
async fn a_poisoned_engine_still_nudges_reports_and_is_saved() {
    let mut env = Env::with_selected_feature();
    env.workflow.lock().unwrap().check(1).unwrap();
    poison(&env);
    {
        let ctx = env.ctx();
        let nudge = workflow_nudge_message(&ctx).await.expect("still nudged");
        assert!(
            nudge
                .into_message(false)
                .content
                .contains("Workflow incomplete")
        );
        assert!(workflow_progress_fingerprint(&ctx).is_some());
        assert_eq!(workflow_idle_reason(&ctx), WorkflowIdleReason::Exhausted);
        let saved = ctx.save_session.save(
            ctx.messages,
            crate::application::sessions::dto::SaveTrigger::Routine,
        );
        saved.await.expect("the save goes on");
    }
    let saved = env
        .store
        .load(&SessionIdentity::from_persisted_key(
            env.session_key.clone(),
        ))
        .await
        .unwrap()
        .expect("the session was saved");
    let run = saved.workflow_run.expect("with its workflow run");
    assert_eq!(run.template_id.as_deref(), Some("feature"));
    assert_eq!(run.done.first(), Some(&true), "the checked step survived");
}
