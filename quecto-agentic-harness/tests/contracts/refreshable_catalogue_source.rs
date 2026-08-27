//! Contract for the application `RefreshableCatalogueSource` port (issue
//! #1574): a refreshable source is an ordinary catalogue source whose network
//! access happens only inside `refresh` — ordinary loads stay network-free —
//! and whose refresh honours the run's bounds and reports a typed outcome.

use quecto::application::ports::{
    CatalogueSource, RefreshBounds, RefreshChange, RefreshContext, RefreshError,
    RefreshableCatalogueSource,
};
use quecto::domain::catalogue::SourceLayer;
use quecto::infrastructure::catalogue_discovery::{
    DiscoveryEndpoint, DiscoverySourceCache, HttpDiscoverySource, UnsupportedRefreshSource,
};

fn assert_refreshable_contract(source: &dyn RefreshableCatalogueSource) {
    assert!(!source.id().is_empty(), "sources must be identifiable");
    assert_eq!(
        source.layer(),
        SourceLayer::Discovered,
        "refreshable discovery sources feed the discovered layer"
    );
    // Ordinary loads must be network-free: loading with no cache present
    // yields an empty layer (or a descriptive error), never a fetch.
    match source.load() {
        Ok(loaded) => {
            for entry in loaded.entries {
                assert_eq!(entry.reference().provider(), &entry.provider.id);
            }
        }
        Err(error) => assert!(!error.is_empty(), "load errors must be descriptive"),
    }
}

#[test]
fn unsupported_source_satisfies_the_contract_with_an_actionable_reason() {
    let source = UnsupportedRefreshSource::new("anthropic-api", "no model listing endpoint");
    assert_refreshable_contract(&source);
    match source.refresh(&RefreshContext::default()) {
        Err(RefreshError::Unsupported { reason }) => {
            assert!(reason.contains("model listing"), "got: {reason}");
        }
        other => panic!("expected unsupported outcome, got {other:?}"),
    }
    assert!(
        source.load().unwrap().entries.is_empty(),
        "an unsupported source contributes no entries"
    );
}

#[test]
fn http_discovery_source_refreshes_into_its_cache_and_loads_it_offline() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "alpha", "name": "Alpha"}]
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let endpoint = DiscoveryEndpoint::for_openai_compatible(
            "openrouter",
            &format!("{}/v1", server.uri()),
            true,
            None,
        );
        let source = HttpDiscoverySource::new(
            DiscoverySourceCache::new(tmp.path(), "openrouter"),
            endpoint,
        );
        assert_refreshable_contract(&source);
        assert!(
            source.load().unwrap().entries.is_empty(),
            "pre-refresh loads must read only the (absent) cache"
        );

        let change = tokio::task::spawn_blocking(move || {
            let change = source.refresh(&RefreshContext::default());
            (change, source)
        })
        .await
        .unwrap();
        let (change, source) = change;
        assert_eq!(change, Ok(RefreshChange::Updated { models: 1 }));
        server.reset().await; // any further load must not touch the network
        assert_eq!(
            source.load().unwrap().entries.len(),
            1,
            "post-refresh loads read the persisted cache"
        );
    });
}

#[test]
fn http_discovery_source_fails_bounded_when_the_response_exceeds_the_cap() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(2048)))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let endpoint = DiscoveryEndpoint::for_openai_compatible(
            "openrouter",
            &format!("{}/v1", server.uri()),
            true,
            None,
        );
        let source = HttpDiscoverySource::new(
            DiscoverySourceCache::new(tmp.path(), "openrouter"),
            endpoint,
        );
        let ctx = RefreshContext::new(RefreshBounds {
            max_response_bytes: 1024,
            ..RefreshBounds::default()
        });
        let result = tokio::task::spawn_blocking(move || source.refresh(&ctx))
            .await
            .unwrap();
        match result {
            Err(RefreshError::Failed { reason }) => {
                assert!(reason.contains("exceeds"), "got: {reason}");
            }
            other => panic!("an over-cap response must fail bounded, got {other:?}"),
        }
    });
}
