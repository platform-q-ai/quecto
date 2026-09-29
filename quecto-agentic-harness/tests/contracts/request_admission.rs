use quecto::application::providers::ports::RequestAdmission;
use quecto::domain::provider::RequestAttempt;
#[tokio::test]
async fn real_admission_tracks_pause_resume_and_terminal_actor() {
    let (_directory, context) = super::swarm_control_fixture::context();
    context.check(RequestAttempt::First).await.unwrap();
    quecto::infrastructure::tools::call_work::off_the_runtime(|| context.pause("approval"))
        .unwrap();
    assert!(context.check(RequestAttempt::First).await.is_err());
    assert!(
        quecto::infrastructure::tools::call_work::off_the_runtime(|| context.resume()).is_err(),
        "members cannot resume (#1729)"
    );
    quecto::infrastructure::tools::call_work::off_the_runtime(|| context.resume_external())
        .unwrap();
    context.check(RequestAttempt::First).await.unwrap();
    quecto::infrastructure::tools::call_work::off_the_runtime(|| context.cancel_run()).unwrap();
    context.check(RequestAttempt::First).await.unwrap();
    let unknown = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        member: "unknown".into(),
        ..context
    };
    assert!(unknown.check(RequestAttempt::First).await.is_err());
}
