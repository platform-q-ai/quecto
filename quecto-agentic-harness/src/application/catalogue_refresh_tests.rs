use super::*;

struct FakeRefreshPort;

impl CatalogueRefreshAllPort for FakeRefreshPort {
    fn refresh_all_sources(&self) -> Vec<CatalogueRefreshOutcome> {
        vec![
            self.refresh_source("supported"),
            CatalogueRefreshOutcome {
                source: "broken".to_string(),
                status: CatalogueRefreshStatus::Failed {
                    error: "network".to_string(),
                },
            },
        ]
    }
}

impl CatalogueRefreshPort for FakeRefreshPort {
    fn refresh_source(&self, source: &str) -> CatalogueRefreshOutcome {
        CatalogueRefreshOutcome {
            source: source.to_string(),
            status: if source == "supported" {
                CatalogueRefreshStatus::Refreshed { models: 3 }
            } else {
                CatalogueRefreshStatus::Skipped {
                    reason: "unsupported".to_string(),
                }
            },
        }
    }
}

#[test]
fn refresh_use_case_delegates_to_refresh_port_and_returns_structured_outcome() {
    let use_case = RefreshCatalogueSourceUseCase::new();

    assert_eq!(
        use_case.refresh(&FakeRefreshPort, "supported"),
        CatalogueRefreshOutcome {
            source: "supported".to_string(),
            status: CatalogueRefreshStatus::Refreshed { models: 3 }
        }
    );
    assert_eq!(
        use_case.refresh(&FakeRefreshPort, "google"),
        CatalogueRefreshOutcome {
            source: "google".to_string(),
            status: CatalogueRefreshStatus::Skipped {
                reason: "unsupported".to_string()
            }
        }
    );
    assert_eq!(
        use_case.refresh_all(&FakeRefreshPort),
        vec![
            CatalogueRefreshOutcome {
                source: "supported".to_string(),
                status: CatalogueRefreshStatus::Refreshed { models: 3 }
            },
            CatalogueRefreshOutcome {
                source: "broken".to_string(),
                status: CatalogueRefreshStatus::Failed {
                    error: "network".to_string()
                }
            }
        ]
    );
}
