//! Entry-point contract for provider runtime composition (issue #1573, epic
//! #1193 slice 3; #1849 PR 1): no provider construction, credential
//! resolution, or router orchestration remains anywhere under `interface/`.
//! That orchestration lives behind the application's `ProviderRuntimeFactory`
//! port (implemented in `infrastructure::provider_runtime`); the composition
//! layer (`composition::runtime`) wires dependencies and invokes the shared
//! compose use case, and entry points receive its builder by injection.
//!
//! Source-level ratchet: reintroducing a construction call into any
//! `interface` module fails this test before review ever sees it.

use std::path::Path;

/// Symbols that only infrastructure and composition may touch. Any of these
/// appearing in a non-test `interface` source means provider construction,
/// refresh wiring or router orchestration leaked back into the interface.
const FORBIDDEN: &[&str] = &[
    "ProviderRouter",
    "RetryingProvider",
    "RefreshableProvider",
    "create_openai_compatible_provider",
    "create_anthropic_compatible_provider",
    "create_provider_with_client",
    "create_named_openai_provider_with_client",
    "create_openai_provider_with_client",
    "create_codex_provider_with_client",
    "make_provider_factory",
    "make_oauth_refresh_fn",
    "compose_and_publish_runtime",
    "ComposeProviderRuntimeUseCase",
];

fn is_test_source(name: &str) -> bool {
    name.ends_with("_tests.rs") || name.contains("_tests_")
}

#[test]
fn interface_contains_no_provider_construction_or_router_orchestration() {
    let interface_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/interface");
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    for entry in walk(&interface_dir) {
        let name = entry.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || is_test_source(&name) {
            continue;
        }
        scanned += 1;
        let source = std::fs::read_to_string(&entry).unwrap();
        for symbol in FORBIDDEN {
            if source.contains(symbol) {
                offenders.push(format!("{name}: {symbol}"));
            }
        }
        // Credential resolution is confined to the composition layer; the
        // sole exception is the `auth` command surface, which manages the
        // credential store itself (login/logout) rather than resolving
        // credentials to build providers.
        if name != "auth.rs" && source.contains("CredentialStore::new") {
            offenders.push(format!("{name}: CredentialStore::new"));
        }
    }
    assert!(
        scanned > 50,
        "expected to scan the interface modules, saw {scanned}"
    );
    assert!(
        offenders.is_empty(),
        "provider construction leaked back into interface:\n{}",
        offenders.join("\n")
    );
}

/// Composition's entry point routes through the shared composition use case:
/// the provider it returns IS the published runtime generation's provider,
/// and the runtime + catalogue stores publish one coherent generation.
#[test]
fn build_agent_provider_publishes_through_the_composition_use_case() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"entryish":{"api":"openai-completions","apiKey":"sk-entry",
            "baseUrl":"https://api.example.test/v1",
            "models":[{"id":"entry-model","name":"Entry Model"}]}}}"#,
    )
    .unwrap();
    let config = quecto::infrastructure::config::Config::default();
    let provider = quecto::composition::runtime::build_agent_provider(
        &config,
        tmp.path(),
        &reqwest::Client::new(),
    )
    .expect("provider builds");
    let published = quecto::infrastructure::catalogue_registry::runtime_store_for(tmp.path())
        .current()
        .expect("the entry point published a runtime generation");
    assert!(
        std::sync::Arc::ptr_eq(&provider, &published.provider),
        "the returned provider must be the published runtime generation's provider"
    );
    assert_eq!(
        published.generation(),
        quecto::infrastructure::catalogue_registry::snapshot_store_for(tmp.path())
            .current()
            .generation(),
        "runtime and catalogue stores share one generation"
    );
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}
