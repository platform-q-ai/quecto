//! Contract coverage for the application-owned catalogue refresh port.

use quecto::catalogue_refresh_app::{
    CatalogueRefreshApplication, CatalogueRefreshOutcome, CatalogueRefreshPort,
    CatalogueRefreshStatus,
};

struct FakeRefreshPort;

impl CatalogueRefreshPort for FakeRefreshPort {
    fn refresh_source(&self, source: &str) -> CatalogueRefreshOutcome {
        CatalogueRefreshOutcome {
            source: source.to_string(),
            status: CatalogueRefreshStatus::Refreshed { models: 3 },
        }
    }

    fn refresh_all_sources(&self) -> Vec<CatalogueRefreshOutcome> {
        vec![
            CatalogueRefreshOutcome {
                source: "anthropic".to_string(),
                status: CatalogueRefreshStatus::Skipped {
                    reason: "unsupported".to_string(),
                },
            },
            self.refresh_source("open"),
        ]
    }
}

#[test]
fn catalogue_refresh_port_reports_one_source_through_the_application() {
    let outcome = CatalogueRefreshApplication::new(FakeRefreshPort).refresh("open");

    assert_eq!(outcome.source, "open");
    assert_eq!(
        outcome.status,
        CatalogueRefreshStatus::Refreshed { models: 3 }
    );
}

#[test]
fn catalogue_refresh_port_preserves_per_source_refresh_all_outcomes() {
    let outcomes = CatalogueRefreshApplication::new(FakeRefreshPort).refresh_all();

    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| outcome.source.as_str())
            .collect::<Vec<_>>(),
        ["anthropic", "open"]
    );
    assert!(matches!(
        outcomes[0].status,
        CatalogueRefreshStatus::Skipped { .. }
    ));
    assert!(matches!(
        outcomes[1].status,
        CatalogueRefreshStatus::Refreshed { models: 3 }
    ));
}
