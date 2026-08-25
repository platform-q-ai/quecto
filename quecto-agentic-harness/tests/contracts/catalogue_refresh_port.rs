//! Contract coverage for catalogue refresh ports.

use quecto::catalogue_refresh_app::{
    CatalogueRefreshAllPort, CatalogueRefreshOutcome, CatalogueRefreshPort, CatalogueRefreshStatus,
    RefreshCatalogueSourceUseCase,
};

struct RefreshPort;

impl CatalogueRefreshPort for RefreshPort {
    fn refresh_source(&self, source: &str) -> CatalogueRefreshOutcome {
        CatalogueRefreshOutcome {
            source: source.to_string(),
            status: if source == "ok" {
                CatalogueRefreshStatus::Refreshed { models: 2 }
            } else {
                CatalogueRefreshStatus::Failed {
                    error: "failed".to_string(),
                }
            },
        }
    }
}

impl CatalogueRefreshAllPort for RefreshPort {
    fn refresh_all_sources(&self) -> Vec<CatalogueRefreshOutcome> {
        vec![self.refresh_source("ok"), self.refresh_source("bad")]
    }
}

#[test]
fn refresh_source_returns_structured_per_source_status() {
    let outcome = RefreshCatalogueSourceUseCase::new().refresh(&RefreshPort, "ok");

    assert_eq!(outcome.source, "ok");
    assert_eq!(
        outcome.status,
        CatalogueRefreshStatus::Refreshed { models: 2 }
    );
}

#[test]
fn refresh_all_does_not_short_circuit_failed_sources() {
    let outcomes = RefreshCatalogueSourceUseCase::new().refresh_all(&RefreshPort);

    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        outcomes[0].status,
        CatalogueRefreshStatus::Refreshed { models: 2 }
    );
    assert_eq!(
        outcomes[1].status,
        CatalogueRefreshStatus::Failed {
            error: "failed".to_string()
        }
    );
}
