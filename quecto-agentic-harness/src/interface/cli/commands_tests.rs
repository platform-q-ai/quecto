use crate::interface::cli::{CliContext, run_with_output};

fn args(s: &str) -> Vec<String> {
    let mut v = vec!["quecto".to_string()];
    if !s.is_empty() {
        v.extend(s.split_whitespace().map(String::from));
    }
    v
}

fn assert_contains_all(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "expected output to contain '{needle}', got:\n{haystack}"
        );
    }
}

#[test]
fn test_status_shows_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "agents": { "defaults": { "model": "gpt-5.4" } },
        "providers": {
            "openai": { "api_key": "sk-test" },
            "anthropic": { "api_key": "" }
        }
    }"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert_contains_all(
        &out.stdout,
        &[
            "quecto Status",
            "Config:",
            "Workspace:",
            "Model:",
            "gpt-5.4",
            "OpenAI API:",
            "configured",
            "Anthropic API:",
            "not set",
        ],
    );
}

#[test]
fn test_status_respects_global_config_flag() {
    let base = tempfile::TempDir::new().unwrap();
    let custom_dir = tempfile::TempDir::new().unwrap();
    let custom_config = custom_dir.path().join("custom.json");
    std::fs::write(
        &custom_config,
        r#"{"agents":{"defaults":{"model":"custom/status-model"}}}"#,
    )
    .unwrap();
    std::fs::write(
        base.path().join("config.json"),
        r#"{"agents":{"defaults":{"model":"base/status-model"}}}"#,
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(base.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        ..Default::default()
    };

    let out = run_with_output(
        vec![
            "quecto".into(),
            "--config".into(),
            custom_config.display().to_string(),
            "status".into(),
        ],
        &ctx,
    );

    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains(&format!("Config:    {}", custom_config.display())),
        "stdout: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("custom/status-model"),
        "stdout: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("base/status-model"),
        "stdout: {}",
        out.stdout
    );
}

#[test]
fn test_status_no_config_uses_defaults() {
    // Zero-config: status with no config file succeeds on defaults.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("quecto Status"));
    assert!(!out.stderr.contains("config not found"));
}

#[test]
fn test_status_redacts_api_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-super-secret-12345" }
        }
    }"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(!out.stdout.contains("sk-super-secret-12345"));
    assert!(out.stdout.contains("configured"));
}

#[test]
fn test_status_both_providers_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test-openai" },
            "anthropic": { "api_key": "sk-ant-test" }
        }
    }"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    let configured_count = out.stdout.matches("configured").count();
    assert_eq!(configured_count, 2, "stdout: {}", out.stdout);
}

#[test]
fn test_status_explicit_missing_config_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let missing_config = tmp.path().join("missing.json");
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        ..Default::default()
    };

    let out = run_with_output(
        vec![
            "quecto".into(),
            "--config".into(),
            missing_config.display().to_string(),
            "status".into(),
        ],
        &ctx,
    );

    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr
            .contains(&format!("config not found: {}", missing_config.display())),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_status_invalid_config_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "{ not valid json ").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to load config"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn status_reports_only_published_runtime_admission_warnings() {
    use crate::application::catalogue::{
        CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, SourceEntries,
    };
    use crate::application::provider_runtime::{
        AdmissionBindingDiagnostic, ComposeProviderRuntimeUseCase, CompositionPorts,
        ProviderRuntimeFactory, ProviderRuntimeOutcome,
    };
    use crate::application::providers::ports::{ChatRequest, LlmProvider};
    use crate::domain::catalogue::{CatalogueEntry, SourceLayer};
    use crate::domain::error::DomainError;
    use crate::domain::message::LlmResponse;
    use std::sync::Arc;

    #[derive(Debug)]
    struct StubProvider;
    impl LlmProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }
        fn chat<'a>(
            &'a self,
            _request: ChatRequest<'a>,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>>
        {
            Box::pin(async { Err(DomainError::Provider("unused".into())) })
        }
    }
    struct StubFactory;
    impl ProviderRuntimeFactory<(), ()> for StubFactory {
        fn compose_runtime(&self, _: &(), _: &()) -> Result<Arc<dyn LlmProvider>, String> {
            Ok(Arc::new(StubProvider))
        }
        fn compose_runtime_outcome(
            &self,
            _: &(),
            _: &(),
        ) -> Result<ProviderRuntimeOutcome, String> {
            Ok(ProviderRuntimeOutcome {
                provider: Arc::new(StubProvider),
                admission_binding_diagnostic: AdmissionBindingDiagnostic {
                    unbound_slots: vec!["openai-api".into()],
                },
            })
        }
    }
    struct EmptySource;
    impl CatalogueSource for EmptySource {
        fn id(&self) -> &str {
            "empty"
        }
        fn layer(&self) -> SourceLayer {
            SourceLayer::BuiltIn
        }
        fn load(&self) -> Result<SourceEntries, String> {
            Ok(SourceEntries::from(Vec::<CatalogueEntry>::new()))
        }
    }
    struct Credentials;
    impl CredentialStatusPort for Credentials {
        fn credential_available(&self, _: &CatalogueEntry) -> bool {
            true
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let ctx = CliContext {
        base_dir: Some(dir.path().to_path_buf()),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        ..Default::default()
    };
    let before = run_with_output(args("status"), &ctx);
    assert!(!before.stderr.contains("admission-broker gating"));
    let runtime_store = crate::infrastructure::catalogue_registry::runtime_store_for(dir.path());
    let catalogue_store = CatalogueSnapshotStore::empty();
    ComposeProviderRuntimeUseCase::new()
        .compose_and_publish(
            &StubFactory,
            &(),
            &(),
            &CompositionPorts {
                sources: &[&EmptySource],
                credentials: &Credentials,
                catalogue_store: &catalogue_store,
                runtime_store: &runtime_store,
            },
        )
        .unwrap();
    let after = run_with_output(args("status"), &ctx);
    assert!(!after.stderr.contains("admission-broker gating"));
}
