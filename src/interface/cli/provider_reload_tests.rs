use std::sync::Arc;

use tempfile::TempDir;

use crate::domain::provider::LlmProvider;
use crate::interface::cli::provider_reload::{
    ProviderReloadInputs, force_provider_reload, poll_provider_reload, seeded_provider_reload,
};

fn provider() -> Arc<dyn LlmProvider> {
    crate::interface::test_support::make_stub_provider()
}

fn write_config(dir: &TempDir, body: &str) -> std::path::PathBuf {
    let path = dir.path().join("config.json");
    std::fs::write(&path, body).unwrap();
    path
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
        .unwrap();

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
        .unwrap();

    assert!(matches!(
        result,
        crate::infrastructure::reload::ReloadResult::Unchanged
    ));
}
