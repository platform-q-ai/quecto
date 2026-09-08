use quecto::domain::swarm::{RunControlAction, RunStatus, SwarmRunControl};
#[tokio::test]
async fn real_control_is_idempotent_and_orders_durable_generations() {
    let (_directory, context) = super::swarm_control_fixture::context();
    let pause = || RunControlAction::Pause {
        reason: "approval".into(),
    };
    let first = context.apply(pause()).await.unwrap();
    assert_eq!(first.status, RunStatus::Paused);
    assert_eq!(
        context.apply(pause()).await.unwrap().generation,
        first.generation
    );
    let resumed = context.apply(RunControlAction::Resume).await.unwrap();
    assert_eq!(resumed.status, RunStatus::Running);
    assert!(resumed.generation > first.generation);
    assert_eq!(
        context
            .apply(RunControlAction::Status)
            .await
            .unwrap()
            .generation,
        resumed.generation
    );
    context.cancel_run().unwrap();
    assert!(context.apply(RunControlAction::Resume).await.is_err());
}
