use super::*;
use crate::application::ports::agent_gateway::AgentCommand;
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn rejects_when_all_fields_absent() {
    let gw = MockGateway::connected();
    let err = execute(
        &gw,
        SetModelInput {
            model: None,
            provider: None,
            model_id: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::InvalidRequest(_)));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn rejects_blank_and_whitespace_model() {
    for m in ["", "   ", "\t"] {
        let gw = MockGateway::connected();
        let err = execute(
            &gw,
            SetModelInput {
                model: Some(m.into()),
                provider: None,
                model_id: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::InvalidRequest(_)), "model={m:?}");
        assert!(gw.commands().is_empty(), "model={m:?}");
    }
}

#[tokio::test]
async fn rejects_provider_without_model_id() {
    let gw = MockGateway::connected();
    let err = execute(
        &gw,
        SetModelInput {
            model: None,
            provider: Some("openai".into()),
            model_id: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::InvalidRequest(_)));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn rejects_split_with_blank_model_id() {
    let gw = MockGateway::connected();
    let err = execute(
        &gw,
        SetModelInput {
            model: None,
            provider: Some("openai".into()),
            model_id: Some("   ".into()),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::InvalidRequest(_)));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn trims_combined_model_before_forwarding() {
    let gw = MockGateway::connected();
    execute(
        &gw,
        SetModelInput {
            model: Some("  anthropic/claude  ".into()),
            provider: None,
            model_id: None,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::SetModel { model: Some(m), .. }] if m == "anthropic/claude"
    ));
}

#[tokio::test]
async fn forwards_split_provider_model() {
    let gw = MockGateway::connected();
    execute(
        &gw,
        SetModelInput {
            model: None,
            provider: Some("openai".into()),
            model_id: Some("gpt".into()),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::SetModel { provider: Some(p), model_id: Some(m), .. }]
            if p == "openai" && m == "gpt"
    ));
}

#[tokio::test]
async fn rejects_when_disconnected() {
    let gw = MockGateway::disconnected();
    let err = execute(
        &gw,
        SetModelInput {
            model: Some("a/b".into()),
            provider: None,
            model_id: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
}
