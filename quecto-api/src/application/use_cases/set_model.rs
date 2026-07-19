use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Input for switching the active model at runtime.
///
/// Accepts either the legacy combined `model` ("provider/modelId") form or the
/// split `provider` + `model_id` form. At least one must be present.
pub struct SetModelInput {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub model_id: Option<String>,
}

/// Collapse a missing, empty, or whitespace-only field to `None`, otherwise
/// return the trimmed value so downstream never sees blank targets.
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub async fn execute(
    gateway: &dyn AgentGateway,
    input: SetModelInput,
) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    // Normalize: treat blank/whitespace-only fields as absent so partial or
    // empty targets are rejected here (deterministic 400) rather than failing
    // upstream in the harness resolver.
    let model = non_blank(input.model);
    let provider = non_blank(input.provider);
    let model_id = non_blank(input.model_id);

    // A valid target is either a combined `model` ("provider/modelId") or BOTH
    // split `provider` and `model_id`. Anything else (e.g. `provider` alone) is
    // rejected.
    let has_combined = model.is_some();
    let has_split = provider.is_some() && model_id.is_some();
    if !has_combined && !has_split {
        return Err(ApiError::InvalidRequest(
            "model (\"provider/modelId\") or both provider and modelId must be provided and non-empty"
                .into(),
        ));
    }
    gateway
        .send(AgentCommand::SetModel {
            model,
            provider,
            model_id,
        })
        .await
}

#[cfg(test)]
mod tests {
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
}
