use super::*;
use crate::application::ports::AttemptPermit;
use crate::domain::error::DomainError;
use crate::domain::inference_admission::GroupId;
use std::future::Future;
use std::pin::Pin;

#[derive(Debug)]
struct NeverSend;
impl AttemptAdmission for NeverSend {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        Box::pin(async { Err(DomainError::Provider("fixture refuses dispatch".into())) })
    }
}

fn proposal() -> AdmissionRuntimeProposal {
    let group = GroupId::new("group").unwrap();
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 2,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 2,
                    queue_timeout_ms: 100,
                    attempt_timeout_ms: 100,
                    fallback_base_ms: 100,
                    max_cooldown_ms: 100,
                },
            )]),
            aliases: BTreeMap::from([("account".into(), group)]),
            max_scopes: 2,
            terminal_capacity: 2,
        },
        bindings: BTreeMap::from([("endpoint".into(), "account".into())]),
    }
}
fn gates() -> BTreeMap<String, Arc<dyn AttemptAdmission>> {
    BTreeMap::from([(
        "account".into(),
        Arc::new(NeverSend) as Arc<dyn AttemptAdmission>,
    )])
}

#[test]
fn context_rejects_unknown_alias_and_missing_capability() {
    let mut unknown = proposal();
    unknown.bindings.insert("endpoint".into(), "unknown".into());
    assert!(AdmissionRuntimeContext::new(unknown, gates()).is_err());
    assert!(AdmissionRuntimeContext::new(proposal(), BTreeMap::new()).is_err());
}

#[test]
fn configured_provider_without_binding_cannot_compose() {
    let mut proposal = proposal();
    proposal.bindings.clear();
    let context = AdmissionRuntimeContext::new(proposal.clone(), gates()).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let inputs = AgentRuntimeInputs {
        base_dir: tmp.path().into(),
        http_client: reqwest::Client::new(),
        refresh_fn: Arc::new(|_, _| Box::pin(async { panic!("no OAuth") })),
        openai_oauth_factory: Arc::new(|_| panic!("no OAuth")),
        model_registry: Ok(
            crate::infrastructure::model_registry::ModelRegistry::from_file_records(vec![]),
        ),
    };
    let mut config = Config::default();
    config.providers.openai.api_key = "configured".into();
    let result = compose_agent_provider_with_admission(&config, &inputs, &context, Some(&proposal));
    assert!(
        result
            .unwrap_err()
            .contains("requires an explicit alias binding")
    );
}
