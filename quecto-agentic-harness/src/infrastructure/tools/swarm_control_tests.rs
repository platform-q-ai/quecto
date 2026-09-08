use super::*;
use serde_json::json;
#[tokio::test]
async fn native_supervisor_validates_budget_and_event_requests_without_mutation() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    for (op, input) in [
        ("usage_budget", json!({})),
        (
            "usage_budget",
            json!({"token_limit":4,"strict_unknown":"yes"}),
        ),
        ("usage_budget", json!({"token_limit":-1})),
        ("events", json!({"limit":0})),
        ("events", json!({"after":"bad"})),
        ("unknown", json!({})),
    ] {
        assert!(control(context.clone(), op, input).await.is_err());
    }
    let configured = control(context.clone(), "usage_budget", json!({"token_limit":100}))
        .await
        .unwrap();
    assert_eq!(configured["budget"]["token_limit"], 100);
    assert_eq!(
        control(context.clone(), "usage", json!({})).await.unwrap()["budget"]["strict_unknown"],
        true
    );
    assert!(
        control(context.clone(), "usage_budget", json!({"token_limit":null}))
            .await
            .unwrap()["budget"]["token_limit"]
            .is_null()
    );
    let paused = control(context.clone(), "pause", json!({"reason":"approval"}))
        .await
        .unwrap();
    assert_eq!(paused["status"], "paused");
    let resumed = control(context.clone(), "resume", json!({})).await.unwrap();
    assert_eq!(resumed["status"], "running");
    let cancelled = control(context, "cancel_run", json!({})).await.unwrap();
    assert_eq!(cancelled["status"], "cancelled");
}
