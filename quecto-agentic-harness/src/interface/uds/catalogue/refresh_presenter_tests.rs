use super::*;
use crate::application::catalogue::dto::SourceRefreshOutcome;

fn outcome(source: &str, status: SourceRefreshStatus) -> SourceRefreshOutcome {
    SourceRefreshOutcome {
        source: source.into(),
        status,
    }
}

#[test]
fn a_republish_renders_the_exact_published_generation() {
    let store = crate::application::catalogue::CatalogueSnapshotStore::empty();
    let resolved = crate::application::catalogue::ResolveCatalogueUseCase.resolve_and_publish(
        &[],
        &GrantNone,
        &store,
    );
    let json = render(&CatalogueRefreshReport {
        outcomes: vec![],
        resolved: Some(resolved),
    });
    assert_eq!(json["generation"], store.current().generation());
    assert_eq!(json["generation"], 1);
}

struct GrantNone;

impl crate::application::catalogue::CredentialStatusPort for GrantNone {
    fn credential_available(&self, _entry: &crate::domain::catalogue::CatalogueEntry) -> bool {
        false
    }
}

#[test]
fn every_status_renders_its_legacy_shape_and_no_republish_is_a_null_generation() {
    let json = render(&CatalogueRefreshReport {
        outcomes: vec![
            outcome("a", SourceRefreshStatus::Updated { models: 3 }),
            outcome("b", SourceRefreshStatus::Unchanged { models: 1 }),
            outcome(
                "c",
                SourceRefreshStatus::Unsupported {
                    reason: "listing".into(),
                },
            ),
            outcome(
                "d",
                SourceRefreshStatus::Failed {
                    reason: "boom".into(),
                },
            ),
            outcome("e", SourceRefreshStatus::Cancelled),
        ],
        resolved: None,
    });
    assert_eq!(
        json["outcomes"],
        serde_json::json!([
            {"source": "a", "status": "updated", "models": 3, "reason": null},
            {"source": "b", "status": "unchanged", "models": 1, "reason": null},
            {"source": "c", "status": "unsupported", "models": null, "reason": "listing"},
            {"source": "d", "status": "failed", "models": null, "reason": "boom"},
            {"source": "e", "status": "cancelled", "models": null, "reason": null},
        ])
    );
    assert!(json["generation"].is_null());
}
