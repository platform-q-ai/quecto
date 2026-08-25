use super::*;

struct FakeRefreshPort {
    single: CatalogueRefreshOutcome,
    all: Vec<CatalogueRefreshOutcome>,
}

impl CatalogueRefreshPort for FakeRefreshPort {
    fn refresh_source(&self, source: &str) -> CatalogueRefreshOutcome {
        assert_eq!(source, self.single.source);
        self.single.clone()
    }

    fn refresh_all_sources(&self) -> Vec<CatalogueRefreshOutcome> {
        self.all.clone()
    }
}

#[test]
fn application_refresh_delegates_to_the_port() {
    let application = CatalogueRefreshApplication::new(FakeRefreshPort {
        single: CatalogueRefreshOutcome {
            source: "open".to_string(),
            status: CatalogueRefreshStatus::Refreshed { models: 2 },
        },
        all: vec![],
    });

    let outcome = application.refresh("open");

    assert_eq!(outcome.source, "open");
    assert_eq!(
        outcome.status,
        CatalogueRefreshStatus::Refreshed { models: 2 }
    );
}

#[test]
fn application_refresh_all_preserves_per_source_outcomes() {
    let application = CatalogueRefreshApplication::new(FakeRefreshPort {
        single: CatalogueRefreshOutcome {
            source: "unused".to_string(),
            status: CatalogueRefreshStatus::Failed {
                error: "unused".to_string(),
            },
        },
        all: vec![
            CatalogueRefreshOutcome {
                source: "anthropic".to_string(),
                status: CatalogueRefreshStatus::Skipped {
                    reason: "unsupported".to_string(),
                },
            },
            CatalogueRefreshOutcome {
                source: "open".to_string(),
                status: CatalogueRefreshStatus::Failed {
                    error: "down".to_string(),
                },
            },
        ],
    });

    let outcomes = application.refresh_all();

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
        CatalogueRefreshStatus::Failed { .. }
    ));
}
