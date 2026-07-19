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

pub async fn execute(
    gateway: &dyn AgentGateway,
    input: SetModelInput,
) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    if input.model.is_none() && input.provider.is_none() && input.model_id.is_none() {
        return Err(ApiError::InvalidRequest(
            "model or provider/modelId must be provided".into(),
        ));
    }
    gateway
        .send(AgentCommand::SetModel {
            model: input.model,
            provider: input.provider,
            model_id: input.model_id,
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
