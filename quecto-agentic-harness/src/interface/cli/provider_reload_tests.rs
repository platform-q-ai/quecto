use std::sync::Arc;

use tempfile::TempDir;

use crate::domain::provider::LlmProvider;
use crate::interface::cli::provider_reload::{
    ProviderReloadInputs, force_provider_reload, poll_provider_reload, seeded_provider_reload,
    seeded_provider_reload_with_base,
};

fn provider() -> Arc<dyn LlmProvider> {
    crate::interface::test_support::make_stub_provider()
}

fn write_config(dir: &TempDir, body: &str) -> std::path::PathBuf {
    let path = dir.path().join("config.json");
    std::fs::write(&path, body).unwrap();
    path
}

/// The runtime a seeded gate should retain: the published snapshot, including
/// the prefixes the provider can route.
fn seed(
    provider: Arc<dyn LlmProvider>,
) -> crate::application::provider_runtime::CatalogueRuntimeSnapshot {
    let catalogue = crate::domain::catalogue::CatalogueSnapshot::new(
        0,
        provider.model_descriptors().unwrap_or(&[]).to_vec(),
    )
    .with_open_providers(
        provider
            .open_provider_names()
            .into_iter()
            .filter_map(|name| crate::domain::catalogue::ProviderId::new(name).ok())
            .collect(),
    );
    crate::application::provider_runtime::CatalogueRuntimeSnapshot {
        provider,
        catalogue,
    }
}

fn inputs(path: std::path::PathBuf, dir: &TempDir) -> ProviderReloadInputs {
    ProviderReloadInputs::new(
        path,
        dir.path().to_path_buf(),
        std::collections::HashMap::new(),
        reqwest::Client::new(),
    )
}

fn config_with_fireworks(api_base: &str) -> String {
    format!(
        r#"{{
  "providers": {{
    "openai": {{ "api_key": "sk-test", "api_base": "http://127.0.0.1:9" }},
    "openai_compatible": {{
      "endpoints": [{{
        "prefix": "fireworks",
        "api_key": "sk-fireworks",
        "api_base": "{api_base}",
        "allow_remote_http": true
      }}]
    }}
  }}
}}"#
    )
}

#[tokio::test]
async fn poll_provider_reload_returns_none_when_not_configured() {
    assert!(poll_provider_reload(None, None).await.is_none());
}

#[tokio::test]
async fn force_provider_reload_returns_none_when_not_configured() {
    assert!(force_provider_reload(None, None).await.is_none());
}

#[tokio::test]
async fn changed_poll_reloads_new_provider() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        &dir,
        r#"{"providers":{"openai":{"api_key":"sk-test","api_base":"http://127.0.0.1:9"}}}"#,
    );
    let mut reload = seeded_provider_reload(&path, provider());

    // Move the file mtime forward deterministically so the reload sees a
    // changed file without depending on wall-clock timing.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(later)).unwrap();

    std::fs::write(&path, config_with_fireworks("http://127.0.0.1:9")).unwrap();
    let inputs = inputs(path, &dir);

    let result = poll_provider_reload(Some(&mut reload), Some(&inputs))
        .await
        .unwrap()
        .expect("a valid config must reload");

    assert!(matches!(
        result,
        crate::infrastructure::reload::ReloadResult::Reloaded(_)
    ));
}

#[tokio::test]
async fn forced_reload_reports_malformed_config_error() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, "{ invalid json");
    let mut reload = seeded_provider_reload(&path, provider());
    let inputs = inputs(path, &dir);

    let result = force_provider_reload(Some(&mut reload), Some(&inputs))
        .await
        .unwrap();

    assert!(result.is_err());
    assert!(!result.unwrap_err().is_empty());
}

#[tokio::test]
async fn unchanged_poll_does_not_rebuild() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, r#"{"agents":{"defaults":{"model":"openai/gpt-4o"}}}"#);
    let mut reload = seeded_provider_reload(&path, provider());
    let inputs = inputs(path, &dir);

    let result = poll_provider_reload(Some(&mut reload), Some(&inputs))
        .await
        .unwrap()
        .expect("an unchanged source must not fail");

    assert!(matches!(
        result,
        crate::infrastructure::reload::ReloadResult::Unchanged
    ));
}

#[tokio::test]
async fn forced_reload_publishes_owned_catalogue_snapshot_from_models_json() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    std::fs::write(
        dir.path().join("models.json"),
        serde_json::json!({"providers": {
            "custom": {
                "api": "openai-completions",
                "baseUrl": "https://example.test/v1",
                "apiKey": "sk-custom",
                "models": [{"id": "custom-model", "displayName": "Custom Model"}]
            }
        }})
        .to_string(),
    )
    .unwrap();
    let mut reload =
        seeded_provider_reload_with_base(&path, Some(dir.path().to_path_buf()), seed(provider()));
    let inputs = inputs(path, &dir);

    let result = force_provider_reload(Some(&mut reload), Some(&inputs))
        .await
        .unwrap()
        .unwrap();

    let crate::infrastructure::reload::ReloadResult::Reloaded(runtime) = result else {
        panic!("expected reloaded runtime");
    };
    assert!(
        runtime
            .catalogue
            .models()
            .iter()
            .any(|model| model.qualified_id() == "custom/custom-model")
    );
}

#[tokio::test]
async fn models_json_only_change_reloads_catalogue_and_runtime_together() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    let models_path = dir.path().join("models.json");
    std::fs::write(&models_path, r#"{"providers":{}}"#).unwrap();
    let mut reload =
        seeded_provider_reload_with_base(&path, Some(dir.path().to_path_buf()), seed(provider()));
    std::fs::write(
        &models_path,
        r#"{"providers":{"custom":{"api":"openai-completions","baseUrl":"https://example.test/v1","auth":{"mode":"apiKey","apiKey":"sk-custom"},"models":[{"id":"after-race"}]}}}"#,
    )
    .unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
    filetime::set_file_mtime(&models_path, filetime::FileTime::from_system_time(later)).unwrap();
    let inputs = inputs(path, &dir);

    let crate::infrastructure::reload::ReloadResult::Reloaded(runtime) =
        poll_provider_reload(Some(&mut reload), Some(&inputs))
            .await
            .unwrap()
            .expect("models.json edit must reload")
    else {
        panic!("models.json-only edit should reload");
    };

    assert!(runtime.catalogue.models().iter().any(|model| {
        model.qualified_id() == "custom/after-race"
            && runtime
                .catalogue
                .models()
                .iter()
                .any(|runtime_model| runtime_model == model)
    }));
}

#[tokio::test]
async fn forced_reload_catalogue_matches_runtime_descriptors_with_oauth_credentials() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    std::fs::write(
        dir.path().join("models.json"),
        r#"{"providers":{"custom-oauth":{"api":"openai-completions","baseUrl":"https://api.openai.com/v1","auth":{"mode":"oauth","oauthProvider":"openai"},"models":[{"id":"gpt-oauth"}]}}}"#,
    )
    .unwrap();
    CredentialStore::new(dir.path())
        .store(Credential {
            provider: "openai".to_string(),
            token: "oauth-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
    let mut reload =
        seeded_provider_reload_with_base(&path, Some(dir.path().to_path_buf()), seed(provider()));
    let inputs = inputs(path, &dir);

    let crate::infrastructure::reload::ReloadResult::Reloaded(runtime) =
        force_provider_reload(Some(&mut reload), Some(&inputs))
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("expected reloaded runtime");
    };
    assert!(
        runtime.catalogue.models().iter().any(|model| {
            model.qualified_id() == "custom-oauth/gpt-oauth"
                && model.configured
                && model.availability.runnable()
        }),
        "{:?}",
        runtime.catalogue.models()
    );
}

#[tokio::test]
async fn reload_generations_are_monotonic() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    let mut reload =
        seeded_provider_reload_with_base(&path, Some(dir.path().to_path_buf()), seed(provider()));
    let inputs = inputs(path.clone(), &dir);

    let first = force_provider_reload(Some(&mut reload), Some(&inputs))
        .await
        .unwrap()
        .unwrap();
    let crate::infrastructure::reload::ReloadResult::Reloaded(first) = first else {
        panic!("expected reload")
    };
    let second = force_provider_reload(Some(&mut reload), Some(&inputs))
        .await
        .unwrap()
        .unwrap();
    let crate::infrastructure::reload::ReloadResult::Reloaded(second) = second else {
        panic!("expected reload")
    };

    assert!(second.generation() > first.generation());
}
