use quecto::domain::provider::RequestAdmission;
#[tokio::test]
async fn real_admission_tracks_pause_resume_and_terminal_actor() {
    let (_directory, context) = super::swarm_control_fixture::context();
    context.check().await.unwrap();
    context.pause("approval").unwrap();
    assert!(context.check().await.is_err());
    context.resume().unwrap();
    context.check().await.unwrap();
    context.cancel_run().unwrap();
    context.check().await.unwrap();
    let unknown = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        member: "unknown".into(),
        ..context
    };
    assert!(unknown.check().await.is_err());
}
