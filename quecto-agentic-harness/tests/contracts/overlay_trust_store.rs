//! Contract for the `OverlayTrustStore` port (#2024): trust is by canonical
//! path and exact content; an approval survives a fresh adapter; a changed
//! byte or another path is untrusted; a non-interactive adapter never
//! approves on its own.
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::configuration::ports::{OverlayTrust, OverlayTrustStore};
use quecto::infrastructure::config::persistence::PersistentOverlayTrustStore;

fn under_test(base_dir: &std::path::Path) -> Arc<dyn OverlayTrustStore> {
    Arc::new(PersistentOverlayTrustStore::for_base_dir(base_dir, false))
}

#[test]
fn an_approval_is_by_path_and_content_and_persists() {
    let base = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let overlay = repo.path().join("config.json");
    std::fs::write(&overlay, "{}").unwrap();
    let store = under_test(base.path());
    assert!(matches!(
        store.decide(&overlay, b"{}"),
        OverlayTrust::Untrusted { .. }
    ));
    let approval = store.approve(&overlay, b"{}").unwrap();
    assert!(!approval.fingerprint.is_empty());
    assert_eq!(approval.path, overlay.canonicalize().unwrap());

    let fresh = under_test(base.path());
    assert_eq!(fresh.decide(&overlay, b"{}"), OverlayTrust::Trusted);
    assert!(matches!(
        fresh.decide(&overlay, b"{ }"),
        OverlayTrust::Untrusted { .. }
    ));
    let other = repo.path().join("other.json");
    std::fs::write(&other, "{}").unwrap();
    assert!(matches!(
        fresh.decide(&other, b"{}"),
        OverlayTrust::Untrusted { .. }
    ));
}

#[test]
fn a_non_interactive_adapter_never_consents_and_a_new_approval_supersedes_the_old() {
    let base = TempDir::new().unwrap();
    let store = under_test(base.path());
    let path = base.path().join("x.json");
    assert!(!store.offer(&path, "abc", b"{}"));
    store.approve(&path, b"v1").unwrap();
    store.approve(&path, b"v2").unwrap();
    assert_eq!(store.decide(&path, b"v2"), OverlayTrust::Trusted);
    assert!(matches!(
        store.decide(&path, b"v1"),
        OverlayTrust::Untrusted { .. }
    ));
}

#[test]
fn the_untrusted_fingerprint_is_the_one_an_approval_records() {
    let base = TempDir::new().unwrap();
    let store = under_test(base.path());
    let path = base.path().join("x.json");
    let OverlayTrust::Untrusted { fingerprint } = store.decide(&path, b"abc") else {
        panic!("unrecorded content is untrusted");
    };
    assert_eq!(
        store.approve(&path, b"abc").unwrap().fingerprint,
        fingerprint
    );
}
