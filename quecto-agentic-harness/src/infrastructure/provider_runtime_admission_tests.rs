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
    assert!(
        AdmissionRuntimeContext::new(
            unknown,
            gates(),
            crate::infrastructure::providers::SingleAttemptClient::build(
                reqwest::Client::builder().no_proxy()
            )
            .unwrap()
        )
        .is_err()
    );
    assert!(
        AdmissionRuntimeContext::new(
            proposal(),
            BTreeMap::new(),
            crate::infrastructure::providers::SingleAttemptClient::build(
                reqwest::Client::builder().no_proxy()
            )
            .unwrap()
        )
        .is_err()
    );
}

#[test]
fn configured_provider_without_binding_cannot_compose() {
    let mut proposal = proposal();
    proposal.bindings.clear();
    let context = AdmissionRuntimeContext::new(
        proposal.clone(),
        gates(),
        crate::infrastructure::providers::SingleAttemptClient::build(
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap(),
    )
    .unwrap();
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
    let result =
        compose_agent_provider_with_admission(&config, &inputs, &context, Some(&proposal), false);
    assert!(
        result
            .unwrap_err()
            .contains("requires an explicit alias binding")
    );
}

#[test]
fn a_child_inherits_the_authority_without_matching_its_own_config() {
    // #2024 S3 / #2023: a child composes against the published policy and
    // never validates its own config's admission section. A candidate that a
    // root would reject as "restart required" (a cleared binding set, or none
    // at all) composes cleanly when `inherit` is set.
    let mut initial = proposal();
    initial.bindings = BTreeMap::from([("openai-api".into(), "account".into())]);
    let context = AdmissionRuntimeContext::new(
        initial.clone(),
        gates(),
        SingleAttemptClient::build(reqwest::Client::builder().no_proxy()).unwrap(),
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let inputs = AgentRuntimeInputs {
        base_dir: tmp.path().into(),
        http_client: reqwest::Client::new(),
        refresh_fn: Arc::new(|_, _| Box::pin(async { panic!("no refresh") })),
        openai_oauth_factory: Arc::new(|_| panic!("no oauth")),
        model_registry: Ok(
            crate::infrastructure::model_registry::ModelRegistry::from_file_records(vec![]),
        ),
    };
    let mut config = Config::default();
    config.providers.openai.api_key = "child-key".into();
    // A root would reject `None` as an enabled-state change; a child does not.
    assert!(compose_agent_provider_with_admission(&config, &inputs, &context, None, true).is_ok());
    let mut diverged = initial.clone();
    diverged.bindings.clear();
    assert!(
        compose_agent_provider_with_admission(&config, &inputs, &context, Some(&diverged), true)
            .is_ok()
    );
    // The same divergence is still "restart required" for a root.
    assert!(
        compose_agent_provider_with_admission(&config, &inputs, &context, Some(&diverged), false)
            .unwrap_err()
            .contains("restart required")
    );
}

#[test]
fn invalid_policy_cannot_create_a_runtime_authority() {
    let mut invalid = proposal();
    invalid.policy.groups.values_mut().next().unwrap().capacity = 0;
    let result = AdmissionRuntimeContext::new(
        invalid,
        gates(),
        SingleAttemptClient::build(reqwest::Client::builder().no_proxy()).unwrap(),
    );
    assert!(matches!(result, Err(message) if message.starts_with("invalid admission policy:")));
}

#[test]
fn runtime_factory_preserves_authority_across_reload_and_rejects_policy_changes() {
    let mut initial = proposal();
    initial.bindings = BTreeMap::from([("openai-api".into(), "account".into())]);
    let authority = Arc::new(
        AdmissionRuntimeContext::new(
            initial.clone(),
            gates(),
            SingleAttemptClient::build(reqwest::Client::builder().no_proxy()).unwrap(),
        )
        .unwrap(),
    );
    let factory = AdmissionProviderRuntimeFactory::new(authority.clone());
    let tmp = tempfile::tempdir().unwrap();
    let inputs = AgentRuntimeInputs {
        base_dir: tmp.path().into(),
        http_client: reqwest::Client::new(),
        refresh_fn: Arc::new(|_, _| {
            Box::pin(async { panic!("API-key composition must not refresh OAuth") })
        }),
        openai_oauth_factory: Arc::new(|_| panic!("API-key composition must not create OAuth")),
        model_registry: Ok(
            crate::infrastructure::model_registry::ModelRegistry::from_file_records(vec![]),
        ),
    };
    let mut config = Config::default();
    config.providers.openai.api_key = "fixture-key".into();
    let original_gate = authority.binding("openai-api").unwrap().gate;
    assert!(
        factory
            .compose_runtime(
                &AdmissionRuntimeCandidate {
                    providers: &config,
                    admission: Some(&initial),
                    inherit: false,
                },
                &inputs
            )
            .is_ok()
    );

    let mut changed_binding = initial.clone();
    changed_binding.bindings.clear();
    let mut changed_policy = initial.clone();
    changed_policy
        .policy
        .groups
        .values_mut()
        .next()
        .unwrap()
        .capacity += 1;
    for candidate in [None, Some(&changed_binding), Some(&changed_policy)] {
        let rejected = factory.compose_runtime(
            &AdmissionRuntimeCandidate {
                providers: &config,
                admission: candidate,
                inherit: false,
            },
            &inputs,
        );
        assert!(matches!(rejected, Err(message) if message.contains("restart required")));
    }
    config.providers.openai.api_key = "refreshed-fixture-key".into();
    assert!(
        factory
            .compose_runtime(
                &AdmissionRuntimeCandidate {
                    providers: &config,
                    admission: Some(&initial),
                    inherit: false,
                },
                &inputs
            )
            .is_ok()
    );
    assert!(Arc::ptr_eq(
        &original_gate,
        &authority.binding("openai-api").unwrap().gate
    ));
}

#[test]
fn a_default_binding_catches_unlisted_slots() {
    // #2024 S3: a `*` (or `default`) binding resolves any slot that has no
    // explicit binding, so an added provider does not fail composition.
    let mut proposal = proposal();
    proposal.bindings = BTreeMap::from([("*".into(), "account".into())]);
    let context = AdmissionRuntimeContext::new(
        proposal,
        gates(),
        SingleAttemptClient::build(reqwest::Client::builder().no_proxy()).unwrap(),
    )
    .unwrap();
    // An unlisted slot binds to the default alias.
    assert!(context.binding("openai-api").is_ok());
    assert!(context.binding("anything-else").is_ok());
}

#[test]
fn an_explicit_binding_still_wins_over_the_default() {
    let mut proposal = proposal();
    // A second alias/group so the explicit and default bindings differ.
    let other = crate::domain::inference_admission::GroupId::new("other").unwrap();
    proposal.policy.groups.insert(
        other.clone(),
        proposal.policy.groups.values().next().unwrap().clone(),
    );
    proposal.policy.aliases.insert("second".into(), other);
    proposal.bindings = BTreeMap::from([
        ("openai-api".into(), "account".into()),
        ("*".into(), "second".into()),
    ]);
    let mut gates = gates();
    gates.insert(
        "second".into(),
        Arc::new(NeverSend) as Arc<dyn AttemptAdmission>,
    );
    let context = AdmissionRuntimeContext::new(
        proposal,
        gates,
        SingleAttemptClient::build(reqwest::Client::builder().no_proxy()).unwrap(),
    )
    .unwrap();
    assert!(context.binding("openai-api").is_ok());
    assert!(context.binding("unlisted").is_ok());
}
