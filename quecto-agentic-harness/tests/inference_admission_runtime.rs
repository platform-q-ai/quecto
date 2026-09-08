//! Explicit runtime admission composition with real public chat/HTTP oracles.
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use quecto::application::catalogue::{
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, SourceEntries,
};
use quecto::application::inference_admission::AdmissionService;
use quecto::application::inference_attempt::{AttemptAdmission, AttemptPermit};
use quecto::application::ports::{AdmissionClient, AdmissionDispatcher, AdmissionRegistry};
use quecto::application::provider_runtime::{
    ComposeProviderRuntimeUseCase, CompositionPorts, ProviderRuntimeFactory, RuntimeSnapshotStore,
};
use quecto::domain::catalogue::{CatalogueEntry, SourceLayer};
use quecto::domain::error::DomainError;
use quecto::domain::inference_admission::*;
use quecto::domain::provider::{ChatRequest, LlmProvider};
use quecto::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
use quecto::infrastructure::model_registry::ModelRegistry;
use quecto::infrastructure::provider_runtime::{AgentProviderRuntimeFactory, AgentRuntimeInputs};
use quecto::infrastructure::provider_runtime_admission::{
    AdmissionProviderRuntimeFactory, AdmissionRuntimeCandidate as Candidate,
    AdmissionRuntimeContext, AdmissionRuntimeProposal as Proposal,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const LIMIT: Duration = Duration::from_secs(3);

fn proposal() -> Proposal {
    let shared = GroupId::new("shared").unwrap();
    let separate = GroupId::new("separate").unwrap();
    let policy = GroupPolicy {
        capacity: 2,
        reserve: 0,
        min_interval_ms: 1,
        queue_capacity: 16,
        queue_timeout_ms: 1_000,
        attempt_timeout_ms: 1_000,
        fallback_base_ms: 1000,
        max_cooldown_ms: 1_000,
    };
    Proposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(shared.clone(), policy.clone()), (separate.clone(), policy)]),
            aliases: BTreeMap::from([
                ("account-a".into(), shared.clone()),
                ("account-b".into(), shared),
                ("account-c".into(), separate),
            ]),
            max_scopes: 8,
            terminal_capacity: 16,
        },
        bindings: BTreeMap::from([
            ("alpha".into(), "account-a".into()),
            ("beta".into(), "account-b".into()),
            ("gamma".into(), "account-c".into()),
        ]),
    }
}

#[derive(Debug)]
struct Authority {
    service: AdmissionService,
    scope: ScopeId,
    sequence: u64,
    now: u64,
}

impl Authority {
    fn new(config: AdmissionConfig) -> Arc<Mutex<Self>> {
        let mut service = AdmissionService::new(1, config).unwrap();
        let scope = service.register_root(WorkloadClass::Interactive).unwrap();
        Arc::new(Mutex::new(Self {
            service,
            scope,
            sequence: 10,
            now: 0,
        }))
    }

    fn seed_active_and_cooldown(&mut self) {
        let group = GroupId::new("shared").unwrap();
        for (sequence, now) in [(1, 0), (2, 1)] {
            self.service
                .enqueue(self.scope, sequence, "account-a", now)
                .unwrap();
            assert_eq!(
                self.service.next(&group, now).unwrap(),
                Some(RequestId {
                    scope: self.scope,
                    sequence
                })
            );
        }
        self.service
            .complete(self.scope, 1, Feedback::Throttle { delay_ms: 90 }, 1)
            .unwrap();
        self.now = 1;
        assert_eq!(self.snapshot("shared").active, 1);
        assert_eq!(self.snapshot("shared").cooldown_until, 91);
    }

    fn snapshot(&mut self, group: &str) -> GroupSnapshot {
        self.service
            .snapshot(&GroupId::new(group).unwrap(), self.now)
            .unwrap()
    }
}

// A transport-neutral contract adapter over the REAL P1 authority. Not a fake
// scheduler or admission policy. The clock is explicitly driven by each test.
#[derive(Debug)]
struct BoundAdmission {
    authority: Arc<Mutex<Authority>>,
    alias: String,
    group: GroupId,
}

impl AttemptAdmission for BoundAdmission {
    fn acquire(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AttemptPermit>, DomainError>> + Send + '_>>
    {
        let mut sequence = None;
        Box::pin(std::future::poll_fn(move |_| {
            let mut authority = self.authority.lock().unwrap();
            let Authority {
                service,
                scope,
                sequence: next,
                now,
            } = &mut *authority;
            let id = *sequence.get_or_insert_with(|| {
                *next += 1;
                *next
            });
            let result = service
                .enqueue(*scope, id, &self.alias, *now)
                .and_then(|_| service.next(&self.group, *now))
                .and_then(|_| service.status(*scope, id, *now));
            match result {
                Ok(RequestState::Active { .. }) => Poll::Ready(Ok(Box::new(Permit {
                    authority: self.authority.clone(),
                    sequence: id,
                    report: 0,
                })
                    as Box<dyn AttemptPermit>)),
                Ok(RequestState::Queued { .. }) => Poll::Pending,
                other => Poll::Ready(Err(DomainError::Provider(format!("admission: {other:?}")))),
            }
        }))
    }
}

#[derive(Debug)]
struct Permit {
    authority: Arc<Mutex<Authority>>,
    sequence: u64,
    report: u64,
}
impl AttemptPermit for Permit {
    fn feedback(&mut self, feedback: ThrottleFeedback) {
        self.report += 1;
        let mut authority = self.authority.lock().unwrap();
        let Authority {
            service,
            scope,
            now,
            ..
        } = &mut *authority;
        service
            .report_feedback(*scope, self.sequence, self.report, feedback, *now)
            .unwrap();
    }
    fn finish(self: Box<Self>, feedback: Feedback) {
        let mut authority = self.authority.lock().unwrap();
        let Authority {
            service,
            scope,
            now,
            ..
        } = &mut *authority;
        service
            .complete(*scope, self.sequence, feedback, *now)
            .unwrap();
    }
}

// Fixture constructs bound public-port capabilities; production owns composition,
// binding validation, and restart-only proposal acceptance.
fn runtime_factory(
    effective: Proposal,
    authority: Arc<Mutex<Authority>>,
) -> AdmissionProviderRuntimeFactory {
    let gates = effective
        .policy
        .aliases
        .iter()
        .map(|(alias, group)| {
            let gate: Arc<dyn AttemptAdmission> = Arc::new(BoundAdmission {
                authority: authority.clone(),
                alias: alias.clone(),
                group: group.clone(),
            });
            (alias.clone(), gate)
        })
        .collect();
    AdmissionProviderRuntimeFactory::new(Arc::new(
        AdmissionRuntimeContext::new(
            effective,
            gates,
            quecto::infrastructure::providers::SingleAttemptClient::build(
                reqwest::Client::builder().no_proxy(),
            )
            .unwrap(),
        )
        .unwrap(),
    ))
}

struct EmptySource;
impl CatalogueSource for EmptySource {
    fn id(&self) -> &str {
        "runtime-admission-fixture"
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::BuiltIn
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::default())
    }
}
struct Credentials;
impl CredentialStatusPort for Credentials {
    fn credential_available(&self, _: &CatalogueEntry) -> bool {
        true
    }
}

fn inputs(dir: &std::path::Path) -> AgentRuntimeInputs {
    AgentRuntimeInputs {
        base_dir: dir.into(),
        http_client: reqwest::Client::builder().no_proxy().build().unwrap(),
        refresh_fn: Arc::new(|_, _| Box::pin(async { panic!("fixture has no OAuth") })),
        openai_oauth_factory: Arc::new(|_| panic!("fixture has no OAuth")),
        model_registry: Ok(ModelRegistry::from_file_records(vec![])),
    }
}
fn config(uri: &str, key: &str) -> Config {
    let mut config = Config::default();
    config.providers.openai_compatible.endpoints = ["alpha", "beta", "gamma"]
        .into_iter()
        .map(|slot| OpenAiCompatibleEndpoint {
            prefix: slot.into(),
            api_key: key.into(),
            api_base: uri.into(),
            allow_remote_http: false,
        })
        .collect();
    config
}
fn request(model: &str) -> ChatRequest<'_> {
    ChatRequest {
        trace: None,
        admission: None,
        messages: &[],
        tools: &[],
        model,
        max_tokens: 64,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}
async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":"fixture", "choices":[{"message":{"role":"assistant","content":"runtime-ok"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1}
        }))).mount(&server).await;
    server
}
async fn chat(provider: &Arc<dyn LlmProvider>, model: &str) {
    let response = tokio::time::timeout(LIMIT, provider.chat(request(model)))
        .await
        .unwrap()
        .unwrap();
    assert_content(response.content.as_deref());
}
fn poll_once<T>(future: Pin<&mut (impl Future<Output = T> + ?Sized)>) -> Poll<T> {
    future.poll(&mut Context::from_waker(futures::task::noop_waker_ref()))
}
// Pure oracles below consume observations only. Integration and counterexample
// tests use the same functions; none implement admission or reload behavior.
fn assert_wire_trace(actual: &[(String, String)], expected: &[(&str, &str)]) {
    assert_eq!(actual.len(), expected.len(), "exact raw HTTP starts");
    for ((model, credential), (expected_model, expected_credential)) in actual.iter().zip(expected)
    {
        assert_eq!(model, expected_model, "wire model");
        assert_eq!(credential, expected_credential, "wire credential");
    }
}
async fn assert_wire(server: &MockServer, expected: &[(&str, &str)]) {
    let actual = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            (
                body["model"].as_str().unwrap().to_owned(),
                request
                    .headers
                    .get("authorization")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_wire_trace(&actual, expected);
}
fn assert_budget(actual: GroupSnapshot, expected: (usize, usize, u64)) {
    assert_eq!(actual.active, expected.0, "retained occupancy");
    assert_eq!(actual.queued, expected.1, "alias queue placement");
    assert_eq!(
        actual.cooldown_until, expected.2,
        "retained cooldown deadline"
    );
}
fn assert_retained_publication(
    message: &str,
    retained: Option<u64>,
    runtime: u64,
    catalogue: u64,
    expected: u64,
) {
    assert!(
        message.contains("restart"),
        "visible restart-only rejection: {message}"
    );
    assert_eq!(retained, Some(expected), "error retained snapshot");
    assert_eq!(runtime, expected, "published runtime generation");
    assert_eq!(catalogue, expected, "published catalogue generation");
}
fn assert_blocked(pending: bool) {
    assert!(pending, "no dispatch before exact cooldown deadline");
}
fn assert_content(content: Option<&str>) {
    assert_eq!(content, Some("runtime-ok"));
}

#[tokio::test]
async fn default_runtime_stays_disabled_and_rebuilds_real_wire_credentials_and_model() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let inputs = inputs(dir.path());
    let old = AgentProviderRuntimeFactory
        .compose_runtime(&config(&server.uri(), "old-secret"), &inputs)
        .unwrap();
    let new = AgentProviderRuntimeFactory
        .compose_runtime(&config(&server.uri(), "rotated-secret"), &inputs)
        .unwrap();
    chat(&old, "alpha/model-v1").await;
    chat(&new, "alpha/model-v2").await;
    assert_wire(
        &server,
        &[
            ("model-v1", "Bearer old-secret"),
            ("model-v2", "Bearer rotated-secret"),
        ],
    )
    .await;
}

#[tokio::test]
async fn explicit_aliases_survive_credential_and_model_rebuild_with_exact_budget_and_wire_behavior()
{
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let inputs = inputs(dir.path());
    let proposal = proposal();
    let authority = Authority::new(proposal.policy.clone());
    let factory = runtime_factory(proposal.clone(), authority.clone());
    let old = factory
        .compose_runtime(
            &Candidate {
                providers: &config(&server.uri(), "old-secret"),
                admission: Some(&proposal),
            },
            &inputs,
        )
        .unwrap();
    authority.lock().unwrap().seed_active_and_cooldown();
    let rebuilt = factory
        .compose_runtime(
            &Candidate {
                providers: &config(&server.uri(), "rotated-secret"),
                admission: Some(&proposal.clone()),
            },
            &inputs,
        )
        .unwrap();
    let mut blocked = rebuilt.chat(request("beta/model-v2"));
    assert_blocked(poll_once(blocked.as_mut()).is_pending());
    assert_budget(authority.lock().unwrap().snapshot("shared"), (1, 1, 91));
    assert_wire(&server, &[]).await;
    chat(&rebuilt, "gamma/other-model").await;
    assert_budget(authority.lock().unwrap().snapshot("separate"), (0, 0, 0));
    assert_wire(&server, &[("other-model", "Bearer rotated-secret")]).await;
    authority.lock().unwrap().now = 90;
    assert_blocked(poll_once(blocked.as_mut()).is_pending());
    assert_eq!(authority.lock().unwrap().snapshot("shared").active, 1);
    authority.lock().unwrap().now = 91;
    let response = tokio::time::timeout(LIMIT, blocked).await.unwrap().unwrap();
    assert_content(response.content.as_deref());
    assert_budget(authority.lock().unwrap().snapshot("shared"), (1, 0, 91));
    authority.lock().unwrap().now = 92;
    chat(&old, "alpha/model-v1").await;
    assert_wire(
        &server,
        &[
            ("other-model", "Bearer rotated-secret"),
            ("model-v2", "Bearer rotated-secret"),
            ("model-v1", "Bearer old-secret"),
        ],
    )
    .await;
}

#[tokio::test]
async fn unknown_explicit_account_alias_rejects_composition_before_http() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let mut proposal = proposal();
    proposal
        .bindings
        .insert("alpha".into(), "unmapped-account".into());
    // Valid existing authority; candidate binding must be validated by runtime,
    // not by a test fixture's BTreeMap indexing.
    let valid = self::proposal();
    let authority = Authority::new(valid.policy.clone());
    let factory = runtime_factory(valid, authority);
    let result = factory.compose_runtime(
        &Candidate {
            providers: &config(&server.uri(), "secret"),
            admission: Some(&proposal),
        },
        &inputs(dir.path()),
    );
    assert!(
        result.is_err(),
        "unknown alias must fail before a runnable runtime is returned"
    );
    assert_wire(&server, &[]).await;
}

// Every mutation has its own executable test: one RED must not mask the rest.
async fn assert_restart_only_reload_retains_live_runtime(mutation: &str) {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let inputs = inputs(dir.path());
    let mut proposal = proposal();
    if mutation == "fallback-base" {
        // Exercise an explicit nondefault base through the accepted equal-value
        // rebuild and rejected change, rather than relying on default literals.
        proposal
            .policy
            .groups
            .get_mut(&GroupId::new("shared").unwrap())
            .unwrap()
            .fallback_base_ms = 37;
    }
    let authority = Authority::new(proposal.policy.clone());
    let factory = runtime_factory(proposal.clone(), authority.clone());
    let providers = config(&server.uri(), "live-secret");
    let candidate_providers = config(&server.uri(), "must-not-publish");
    let runtime_store = RuntimeSnapshotStore::new();
    let catalogue_store = CatalogueSnapshotStore::empty();
    let ports = CompositionPorts {
        sources: &[&EmptySource],
        credentials: &Credentials,
        catalogue_store: &catalogue_store,
        runtime_store: &runtime_store,
    };
    let compose = ComposeProviderRuntimeUseCase::new();
    let initial = compose
        .compose_and_publish(
            &factory,
            &Candidate {
                providers: &providers,
                admission: Some(&proposal),
            },
            &inputs,
            &ports,
        )
        .unwrap();
    authority.lock().unwrap().seed_active_and_cooldown();
    // No-op allocated separately is accepted while active + cooldown, not compared
    // by pointer identity. Acceptance alone does not prove retained enforcement.
    let noop = proposal.clone();
    let current = compose
        .compose_and_publish(
            &factory,
            &Candidate {
                providers: &providers,
                admission: Some(&noop),
            },
            &inputs,
            &ports,
        )
        .unwrap();
    assert!(current.snapshot.generation() > initial.snapshot.generation());
    {
        let mut changed = proposal.clone();
        let shared = GroupId::new("shared").unwrap();
        let group = changed.policy.groups.get_mut(&shared).unwrap();
        match mutation {
            "capacity" => group.capacity = 3,
            "reserve" => group.reserve = 1,
            "interval" => group.min_interval_ms = 2,
            "queue-capacity" => group.queue_capacity = 17,
            "queue-timeout" => group.queue_timeout_ms = 2_000,
            "attempt-timeout" => group.attempt_timeout_ms = 2_000,
            "max-cooldown" => group.max_cooldown_ms = 2_000,
            "fallback-base" => group.fallback_base_ms = 38,
            "max-scopes" => changed.policy.max_scopes = 9,
            "terminal-capacity" => changed.policy.terminal_capacity = 17,
            "alias-map" => {
                changed
                    .policy
                    .aliases
                    .insert("account-b".into(), GroupId::new("separate").unwrap());
            }
            "binding-map" => {
                changed.bindings.insert("beta".into(), "account-c".into());
            }
            "disable" => {}
            _ => unreachable!(),
        }
        let result = compose.compose_and_publish(
            &factory,
            &Candidate {
                providers: &candidate_providers,
                admission: (mutation != "disable").then_some(&changed),
            },
            &inputs,
            &ports,
        );
        let error = result.expect_err(mutation);
        assert_retained_publication(
            &error.error,
            error
                .retained
                .as_ref()
                .map(|snapshot| snapshot.generation()),
            runtime_store.current().unwrap().generation(),
            catalogue_store.current().generation(),
            current.snapshot.generation(),
        );
        assert_budget(authority.lock().unwrap().snapshot("shared"), (1, 0, 91));
    }
    let retained = runtime_store.current().unwrap().provider.clone();
    let mut blocked = retained.chat(request("beta/retained-model"));
    assert_blocked(poll_once(blocked.as_mut()).is_pending());
    assert_budget(authority.lock().unwrap().snapshot("shared"), (1, 1, 91));
    assert_budget(authority.lock().unwrap().snapshot("separate"), (0, 0, 0));
    assert_wire(&server, &[]).await;
    authority.lock().unwrap().now = 90;
    assert_blocked(poll_once(blocked.as_mut()).is_pending());
    authority.lock().unwrap().now = 91;
    assert_content(
        tokio::time::timeout(LIMIT, blocked)
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
    );
    assert_wire(&server, &[("retained-model", "Bearer live-secret")]).await;
    assert_budget(authority.lock().unwrap().snapshot("shared"), (1, 0, 91));
    if mutation == "fallback-base" {
        // The retained authority's first no-hint throttle uses the nondefault
        // base37, not a default1000 or rejected candidate38 (91 + 37 = 128).
        let mut authority = authority.lock().unwrap();
        let Authority {
            service,
            scope,
            now,
            ..
        } = &mut *authority;
        service
            .report_feedback(*scope, 2, 1, ThrottleFeedback::NoHint { jitter: 0 }, *now)
            .unwrap();
        assert_budget(authority.snapshot("shared"), (1, 0, 128));
    }
}

macro_rules! restart_only_case {
    ($test:ident, $mutation:literal) => {
        #[tokio::test]
        async fn $test() {
            assert_restart_only_reload_retains_live_runtime($mutation).await;
        }
    };
}

restart_only_case!(capacity_reload_rejected_retaining_live_runtime, "capacity");
restart_only_case!(reserve_reload_rejected_retaining_live_runtime, "reserve");
restart_only_case!(interval_reload_rejected_retaining_live_runtime, "interval");
restart_only_case!(
    queue_capacity_reload_rejected_retaining_live_runtime,
    "queue-capacity"
);
restart_only_case!(
    queue_timeout_reload_rejected_retaining_live_runtime,
    "queue-timeout"
);
restart_only_case!(
    attempt_timeout_reload_rejected_retaining_live_runtime,
    "attempt-timeout"
);
restart_only_case!(
    max_cooldown_reload_rejected_retaining_live_runtime,
    "max-cooldown"
);
restart_only_case!(
    max_scopes_reload_rejected_retaining_live_runtime,
    "max-scopes"
);
restart_only_case!(
    terminal_capacity_reload_rejected_retaining_live_runtime,
    "terminal-capacity"
);
restart_only_case!(
    alias_map_reload_rejected_retaining_live_runtime,
    "alias-map"
);
restart_only_case!(
    binding_map_reload_rejected_retaining_live_runtime,
    "binding-map"
);
restart_only_case!(disable_reload_rejected_retaining_live_runtime, "disable");
restart_only_case!(
    fallback_base_reload_rejected_retaining_nondefault_live_runtime,
    "fallback-base"
);

#[path = "common/admission_runtime_oracle_tests.rs"]
mod oracle_tests;
