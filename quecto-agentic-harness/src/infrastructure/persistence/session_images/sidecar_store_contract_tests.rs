//! Contract tests for the `ImageSidecarStore` seam (#2424).
//!
//! Drives `FileImageSidecarStore` (the adapter composition wires into the
//! session store) through the trait object. The contract: a stored image
//! reads back as its exact text under its digest; storing it again keeps it;
//! a name with no sidecar is missing, and a name that is no digest is
//! corrupt; collecting keeps exactly the live digests; removing all leaves
//! the session nothing.
//! Every operation is keyed by the typed `SessionIdentity`: one session's
//! sidecars are never another's.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{ImageSidecarStore, SidecarRead};
use crate::domain::conversation::stored_images::{ImageRef, VerifiedText, sha256_hex};
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::infrastructure::persistence::session_images::FileImageSidecarStore;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;

fn under_test(base_dir: &std::path::Path) -> Arc<dyn ImageSidecarStore> {
    Arc::new(FileImageSidecarStore::new(FlatSessionLayout::new(base_dir)))
}

fn id(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

/// An image: its base64 text and the reference naming it.
struct Image {
    reference: ImageRef,
    text: String,
}

fn image(text: &str) -> Image {
    Image {
        reference: ImageRef {
            sha256: sha256_hex(text.as_bytes()),
            mime_type: "image/png".into(),
        },
        text: text.to_string(),
    }
}

async fn put(store: &dyn ImageSidecarStore, key: &str, image: &Image) {
    store
        .put(&id(key), &image.reference, &image.text)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_stored_image_reads_back_under_its_digest_and_storing_it_again_keeps_it() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    let png = image("iVBORw0KGgo=one");
    put(store.as_ref(), "cli:a", &png).await;
    put(store.as_ref(), "cli:a", &png).await;
    assert_eq!(
        store
            .get(&id("cli:a"), &png.reference.sha256)
            .await
            .unwrap(),
        SidecarRead::Found(VerifiedText::of(png.text.clone()))
    );
}

#[tokio::test]
async fn a_name_with_no_sidecar_is_missing_and_a_name_that_is_no_digest_is_corrupt() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    let png = image("iVBORw0KGgo=two");
    assert_eq!(
        store
            .get(&id("cli:a"), &png.reference.sha256)
            .await
            .unwrap(),
        SidecarRead::Missing
    );
    for name in [
        "../../etc/passwd",
        "",
        "ABC",
        &png.reference.sha256.to_uppercase(),
    ] {
        assert_eq!(
            store.get(&id("cli:a"), name).await.unwrap(),
            SidecarRead::Corrupt,
            "{name:?}"
        );
    }
}

#[tokio::test]
async fn one_sessions_sidecars_are_not_anothers() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    let png = image("iVBORw0KGgo=three");
    put(store.as_ref(), "cli:a", &png).await;
    assert_eq!(
        store
            .get(&id("cli:b"), &png.reference.sha256)
            .await
            .unwrap(),
        SidecarRead::Missing
    );
    store.remove_all(&id("cli:b")).await.unwrap();
    assert!(matches!(
        store
            .get(&id("cli:a"), &png.reference.sha256)
            .await
            .unwrap(),
        SidecarRead::Found(_)
    ));
}

#[tokio::test]
async fn collecting_keeps_exactly_the_live_digests() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    let (kept, dropped) = (image("iVBORw0KGgo=kept"), image("iVBORw0KGgo=dropped"));
    put(store.as_ref(), "cli:a", &kept).await;
    put(store.as_ref(), "cli:a", &dropped).await;
    let live = BTreeSet::from([kept.reference.sha256.clone()]);
    store.retain_only(&id("cli:a"), &live).await.unwrap();
    assert!(matches!(
        store
            .get(&id("cli:a"), &kept.reference.sha256)
            .await
            .unwrap(),
        SidecarRead::Found(_)
    ));
    assert_eq!(
        store
            .get(&id("cli:a"), &dropped.reference.sha256)
            .await
            .unwrap(),
        SidecarRead::Missing
    );
}

#[tokio::test]
async fn removing_all_leaves_the_session_no_sidecar_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path());
    let png = image("iVBORw0KGgo=four");
    put(store.as_ref(), "cli:a", &png).await;
    store.remove_all(&id("cli:a")).await.unwrap();
    store.remove_all(&id("cli:a")).await.unwrap();
    assert_eq!(
        store
            .get(&id("cli:a"), &png.reference.sha256)
            .await
            .unwrap(),
        SidecarRead::Missing
    );
    store
        .retain_only(&id("cli:a"), &BTreeSet::new())
        .await
        .unwrap();
}
