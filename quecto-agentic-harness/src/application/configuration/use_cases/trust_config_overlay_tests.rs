use super::*;
use crate::application::configuration::use_cases::fakes::{FakeTrust, FakeValidator, MemoryStore};
use std::path::PathBuf;

const OVERLAY: &str = "/work/.quecto/config.json";

fn use_case(store: Arc<MemoryStore>, trust: Arc<FakeTrust>) -> TrustConfigOverlay {
    TrustConfigOverlay::new(store, Arc::new(FakeValidator::default()), trust)
}

fn request() -> OverlayTrustRequest {
    OverlayTrustRequest {
        path: PathBuf::from(OVERLAY),
    }
}

#[test]
fn a_valid_overlay_is_approved_by_content() {
    let content = r#"{"agents":{"defaults":{"model":"local"}}}"#;
    let store = MemoryStore::with(&[(OVERLAY, content)]);
    let trust = Arc::new(FakeTrust::default());
    let approval = use_case(store, trust.clone()).execute(request()).unwrap();
    assert_eq!(approval.path, PathBuf::from(OVERLAY));
    assert!(trust.is_approved(OVERLAY, content.as_bytes()));
}

#[test]
fn nothing_that_would_not_apply_is_approved() {
    type Expectation = fn(&OverlayTrustError) -> bool;
    let cases: [(&str, Expectation); 5] = [
        ("{ nope", |e| matches!(e, OverlayTrustError::Parse { .. })),
        ("[1]", |e| matches!(e, OverlayTrustError::NotAnObject(_))),
        (
            r#"{"providers":{}}"#,
            |e| matches!(e, OverlayTrustError::GlobalOnlyKey { key, .. } if key == "providers"),
        ),
        (
            r#"{"admission":null}"#,
            |e| matches!(e, OverlayTrustError::GlobalOnlyKey { key, .. } if key == "admission"),
        ),
        (r#"{"agents":{"defaults":{"effort":"bogus"}}}"#, |e| {
            matches!(e, OverlayTrustError::Invalid { .. })
        }),
    ];
    for (content, expected) in cases {
        let store = MemoryStore::with(&[(OVERLAY, content)]);
        let trust = Arc::new(FakeTrust::default());
        let error = use_case(store, trust.clone())
            .execute(request())
            .unwrap_err();
        assert!(expected(&error), "{content}: {error:?}");
        assert!(error.to_string().contains(OVERLAY), "{error}");
        assert!(!trust.is_approved(OVERLAY, content.as_bytes()));
    }
    let content = r#"{"container_scripts":{}}"#;
    let store = MemoryStore::with(&[(OVERLAY, content)]);
    assert!(matches!(
        use_case(store, Arc::new(FakeTrust::default()))
            .execute(request())
            .unwrap_err(),
        OverlayTrustError::Invalid { .. }
    ));
}

#[test]
fn a_missing_or_unreadable_overlay_and_a_failed_store_are_reported() {
    let error = use_case(MemoryStore::with(&[]), Arc::new(FakeTrust::default()))
        .execute(request())
        .unwrap_err();
    assert_eq!(error, OverlayTrustError::Missing(PathBuf::from(OVERLAY)));
    assert!(error.to_string().contains("no overlay to trust"));

    let mut store = MemoryStore::default();
    store.broken.insert(PathBuf::from(OVERLAY));
    assert!(matches!(
        use_case(Arc::new(store), Arc::new(FakeTrust::default()))
            .execute(request())
            .unwrap_err(),
        OverlayTrustError::Read { .. }
    ));

    let store = MemoryStore::with(&[(OVERLAY, "{}")]);
    let trust = Arc::new(FakeTrust {
        fail_approve: true,
        ..Default::default()
    });
    let error = use_case(store, trust).execute(request()).unwrap_err();
    assert!(matches!(error, OverlayTrustError::Store { .. }));
    assert!(error.to_string().contains("could not record trust"));
    assert!(
        format!(
            "{:?}",
            use_case(MemoryStore::with(&[]), Arc::new(FakeTrust::default()))
        )
        .contains("TrustConfigOverlay")
    );
}
