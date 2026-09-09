use quecto::domain::request_observation::{RequestAccounting, RequestObservation};
#[tokio::test]
async fn real_accounting_redelivery_counts_once_and_rejects_changed_measurements() {
    let (_directory, context) = super::swarm_control_fixture::context();
    let mut observation = RequestObservation {
        started_unix_ms: Some(1000),
        finished_unix_ms: Some(1001),
        attempt_diagnostics: vec![quecto::domain::attempt_diagnostics::AttemptDiagnostics {
            attempt_number: 1,
            wire_status: Some(200),
            ..Default::default()
        }],
        request_id: "contract-request".into(),
        model: "test".into(),
        provider: "test".into(),
        outcome: "succeeded".into(),
        error_class: None,
        input_tokens: Some(10),
        context_input_tokens: Some(10),
        output_tokens: Some(2),
        cache_read_tokens: None,
        cache_write_tokens: None,
        estimated_cost_micro_usd: None,
        estimated_context_tokens: 8,
        instrumented_attempts: 1,
        oauth_retries: 0,
        duration_ms: 1,
        harness_prefix_sha256: "prefix".into(),
        harness_prefix_bytes: 4,
        harness_prefix_unchanged: None,
    };
    context.record(&observation).await.unwrap();
    context.record(&observation).await.unwrap();
    let usage = context.usage_report().unwrap();
    assert_eq!(usage["totals"]["requests"], 1);
    assert_eq!(
        usage["recent_requests"][0]["observation"]["started_unix_ms"],
        1000
    );
    assert_eq!(
        usage["recent_requests"][0]["observation"]["attempt_diagnostics"][0]["wire_status"],
        200
    );
    assert_eq!(usage["totals"]["observed_tokens"], 12);
    observation.output_tokens = Some(3);
    assert!(context.record(&observation).await.is_err());
    assert_eq!(
        context.usage_report().unwrap()["totals"]["observed_tokens"],
        12
    );
}
