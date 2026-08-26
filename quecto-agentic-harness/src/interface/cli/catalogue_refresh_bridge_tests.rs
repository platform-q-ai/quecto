//! Tests for the interface refresh composition (epic #1193, slice 4): real
//! discovery sources composed from `models.json`, refreshed over HTTP into
//! source caches, published through the shared snapshot store.

use super::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn write_registry(dir: &std::path::Path, providers: serde_json::Value) {
    std::fs::write(
        dir.join("models.json"),
        serde_json::json!({ "providers": providers }).to_string(),
    )
    .unwrap();
}

#[test]
fn refresh_all_reports_per_source_outcomes_and_publishes_discovered_models() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "alpha", "name": "Alpha"}]
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        write_registry(
            tmp.path(),
            serde_json::json!({
                "openrouter": {
                    "api": "openai-completions",
                    "baseUrl": format!("{}/v1", server.uri()),
                    "models": []
                },
                "anthropic-api": {"api": "anthropic-messages", "models": []}
            }),
        );

        let base_dir = tmp.path().to_path_buf();
        let report = tokio::task::spawn_blocking(move || {
            refresh_catalogue(&base_dir, &RefreshSelection::All, RefreshBounds::default())
        })
        .await
        .unwrap();

        let outcome = |source: &str| {
            report
                .outcomes
                .iter()
                .find(|o| o.source == source)
                .unwrap_or_else(|| panic!("no outcome for {source}"))
                .status
                .clone()
        };
        assert_eq!(
            outcome("openrouter"),
            SourceRefreshStatus::Updated { models: 1 }
        );
        match outcome("anthropic-api") {
            SourceRefreshStatus::Unsupported { reason } => {
                assert!(reason.contains("model listing"), "got: {reason}");
            }
            other => panic!("expected unsupported, got {other:?}"),
        }

        // The refreshed model is published and readable through the ordinary
        // network-free listing path (UDS list_models shape).
        let listing = super::super::uds_models::list_models_data(tmp.path());
        let listed = listing["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["model"] == "openrouter/alpha");
        assert!(
            listed,
            "refreshed model must appear in the published listing: {listing}"
        );
    });
}

#[test]
fn user_override_still_wins_over_refreshed_discovered_data_at_the_interface() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "alpha", "name": "Discovered Alpha"}]
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        write_registry(
            tmp.path(),
            serde_json::json!({
                "openrouter": {
                    "api": "openai-completions",
                    "baseUrl": format!("{}/v1", server.uri()),
                    "models": [{"id": "alpha", "name": "My Alpha"}]
                }
            }),
        );

        let base_dir = tmp.path().to_path_buf();
        let report = tokio::task::spawn_blocking(move || {
            refresh_catalogue(&base_dir, &RefreshSelection::All, RefreshBounds::default())
        })
        .await
        .unwrap();
        let resolved = report.resolved.expect("an updating refresh must republish");
        let entry = resolved
            .snapshot
            .find(&crate::domain::catalogue::ModelRef::parse_qualified("openrouter/alpha").unwrap())
            .expect("refreshed model must be published");
        assert_eq!(
            entry.model.display_name.as_deref(),
            Some("My Alpha"),
            "the user's models.json entry must win over discovered data"
        );
    });
}

/// Falsifiable redaction guard (slice-4 review): the provider's baseUrl path
/// embeds the configured apiKey value, and the invalid-baseUrl failure reason
/// echoes the (userinfo/query-stripped) URL — so without the `SecretsRedaction`
/// wiring the secret WOULD appear verbatim in the outcome text. The test
/// asserts the reason was actually transformed, so reverting the redaction
/// wiring (or making it a no-op) fails it.
#[test]
fn failure_reasons_never_contain_configured_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    write_registry(
        tmp.path(),
        serde_json::json!({
            "openrouter": {
                "api": "openai-completions",
                // Remote plain-http is rejected; the failure reason echoes the
                // URL, whose path deliberately contains the secret value.
                "baseUrl": "http://models.example.com/sk-secret-123/v1",
                "apiKey": "sk-secret-123",
                "models": []
            }
        }),
    );

    let report = refresh_catalogue(tmp.path(), &RefreshSelection::All, RefreshBounds::default());
    let status = &report
        .outcomes
        .iter()
        .find(|o| o.source == "openrouter")
        .expect("openrouter must report an outcome")
        .status;
    match status {
        SourceRefreshStatus::Failed { reason } => {
            assert!(
                !reason.contains("sk-secret-123"),
                "refresh outcome leaked a credential: {reason}"
            );
            assert!(
                reason.contains("[redacted]"),
                "the reason must show the secret was redacted (not merely absent): {reason}"
            );
        }
        other => panic!("expected a failed outcome, got {other:?}"),
    }
    let text = format!("{:?}", report.outcomes);
    assert!(
        !text.contains("sk-secret-123"),
        "refresh outcomes leaked a credential: {text}"
    );
}

#[test]
fn malformed_registry_is_one_failed_outcome() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), "{not json").unwrap();
    let report = refresh_catalogue(tmp.path(), &RefreshSelection::All, RefreshBounds::default());
    assert_eq!(report.outcomes.len(), 1);
    assert_eq!(report.outcomes[0].source, "models.json");
    assert!(matches!(
        report.outcomes[0].status,
        SourceRefreshStatus::Failed { .. }
    ));
    assert!(report.resolved.is_none());
}

#[test]
fn describe_outcome_covers_every_status() {
    let cases = [
        (SourceRefreshStatus::Updated { models: 3 }, "discovered 3"),
        (SourceRefreshStatus::Unchanged { models: 3 }, "unchanged"),
        (
            SourceRefreshStatus::Unsupported {
                reason: "no listing".to_string(),
            },
            "not refreshable",
        ),
        (
            SourceRefreshStatus::Failed {
                reason: "boom".to_string(),
            },
            "failed (boom)",
        ),
        (SourceRefreshStatus::Cancelled, "cancelled"),
    ];
    for (status, expected) in cases {
        let line = describe_outcome(&SourceRefreshOutcome {
            source: "s".to_string(),
            status,
        });
        assert!(line.contains(expected), "got: {line}");
    }
}
