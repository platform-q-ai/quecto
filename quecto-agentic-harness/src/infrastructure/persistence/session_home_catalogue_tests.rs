//! Unit coverage of the derived index's rejection cache and walk seeding
//! (#2010 R1-H1) at the adapter's own level — observable without the
//! `test-support` counters: through the diagnostics, the listed rows and the
//! bytes of `home.catalogue`. The counting proof is the contract suite's.
use super::session_home_catalogue::FileSessionHomeCatalogue;
use super::session_layout::FlatSessionLayout;
use super::session_store::FileSessionStore;
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::ports::session_home::{
    SessionHomeCatalogue, SessionMetadataSnapshot,
};
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;
use std::sync::Arc;

const ROT: &[u8] = br#"{"key":"chat-rot","messages":[{"role":"user","content":"AAAA"#;

fn identity(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

fn process(layout: &FlatSessionLayout) -> (Arc<FileSessionStore>, FileSessionHomeCatalogue) {
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    (store.clone(), FileSessionHomeCatalogue::with_store(store))
}

async fn save(store: &FileSessionStore, key: &str, title: &str) {
    let mut session = Session::new(identity(key));
    session.messages.push(Message::user(title));
    store.save(&session).await.unwrap();
    store.release(&identity(key));
}

fn index(layout: &FlatSessionLayout) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(layout.home_catalogue_file()).unwrap()).unwrap()
}

fn titles(snapshot: &SessionMetadataSnapshot) -> Vec<&str> {
    let mut titles: Vec<_> = (snapshot.records.iter())
        .map(|record| record.summary.title.as_str())
        .collect();
    titles.sort_unstable();
    titles
}

/// A good record, one neither half can parse, and an append cut short (the
/// walk lists it, the strict catalogue rejects it).
async fn mixed(layout: &FlatSessionLayout, store: &FileSessionStore) {
    save(store, "chat-good", "a good title").await;
    save(store, "chat-cut", "cut short title").await;
    let cut = layout.session_file(&identity("chat-cut"));
    let mut bytes = std::fs::read(&cut).unwrap();
    bytes.extend_from_slice(b"\n{\"type\":\"append\",\"messages\":[");
    std::fs::write(&cut, bytes).unwrap();
    std::fs::write(layout.session_file(&identity("chat-rot")), ROT).unwrap();
}

#[tokio::test]
async fn rejected_versions_are_indexed_with_what_the_walk_made_of_them() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let (store, catalogue) = process(&layout);
    mixed(&layout, &store).await;
    for pass in ["first", "warm"] {
        let snapshot = catalogue.metadata().await.unwrap();
        assert_eq!(
            titles(&snapshot),
            ["a good title", "cut short title"],
            "{pass}"
        );
        for file in ["chat-rot.json: ", "chat-cut.json: "] {
            let named = snapshot.diagnostics.iter().filter(|d| d.starts_with(file));
            assert_eq!(named.count(), 1, "{pass}: {:?}", snapshot.diagnostics);
        }
    }
    let rejected = index(&layout)["rejected"].clone();
    assert_eq!(rejected["chat-rot.json"]["unlisted"], true, "{rejected}");
    assert!(rejected["chat-rot.json"]["listed"].is_null(), "{rejected}");
    assert_eq!(
        rejected["chat-cut.json"]["listed"],
        serde_json::json!(["chat-cut", "cut short title", 1]),
        "{rejected}"
    );
    assert_eq!(rejected["chat-cut.json"]["unlisted"], false);
    assert_eq!(
        rejected["chat-rot.json"]["stamp"].as_array().unwrap().len(),
        8
    );
    assert!(index(&layout)["records"]["chat-good"]["summary"].is_object());
}

#[tokio::test]
async fn a_new_process_is_seeded_from_the_index_and_a_repair_clears_the_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let (store, catalogue) = process(&layout);
    mixed(&layout, &store).await;
    let first = catalogue.metadata().await.unwrap();
    // The listing alone (no summary walk) carries the same rejections.
    let (_, listing_only) = process(&layout);
    let listing = listing_only.list().unwrap();
    assert_eq!(listing.entries.len(), 1, "{listing:?}");
    assert_eq!(listing.diagnostics.len(), 2, "{listing:?}");
    let (_, cold) = process(&layout);
    let seeded = cold.metadata().await.unwrap();
    assert_eq!(titles(&seeded), titles(&first));
    assert_eq!(seeded.diagnostics, first.diagnostics);
    assert!(!seeded.rebuilt);
    // Repaired in place: read again, listed, and no longer indexed as rejected.
    let mut repaired = ROT.to_vec();
    repaired.extend_from_slice(b"\"}]}");
    std::fs::write(layout.session_file(&identity("chat-rot")), repaired).unwrap();
    let healed = cold.metadata().await.unwrap();
    assert_eq!(titles(&healed), ["AAAA", "a good title", "cut short title"]);
    assert!(index(&layout)["rejected"].get("chat-rot.json").is_none());
    // Deleted: forgotten, in memory and in the index.
    std::fs::remove_file(layout.session_file(&identity("chat-cut"))).unwrap();
    let after = cold.metadata().await.unwrap();
    assert!(after.diagnostics.is_empty(), "{:?}", after.diagnostics);
    assert!(index(&layout).get("rejected").is_none());
}

#[tokio::test]
async fn an_unusable_index_seeds_nothing_and_a_doctored_rejection_seeds_only_its_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let (store, catalogue) = process(&layout);
    mixed(&layout, &store).await;
    let first = catalogue.metadata().await.unwrap();
    let mut doctored = index(&layout);
    let stamp = doctored["records"]["chat-good"]["stamp"].clone();
    doctored["rejected"]["../chat-good.json"] =
        serde_json::json!({"stamp": stamp, "reason": "doctored", "unlisted": true});
    doctored["rejected"]["notes.txt"] =
        serde_json::json!({"stamp": stamp, "reason": "doctored", "unlisted": true});
    doctored["rejected"]["chat-rot.json"]["listed"] =
        serde_json::json!(["chat-good", "forged title", 9]);
    doctored["rejected"]["chat-cut.json"]["listed"] =
        serde_json::json!(["../evil", "forged title", 9]);
    std::fs::write(
        layout.home_catalogue_file(),
        serde_json::to_vec(&doctored).unwrap(),
    )
    .unwrap();
    let (_, cold) = process(&layout);
    let seen = cold.metadata().await.unwrap();
    assert_eq!(
        titles(&seen),
        titles(&first),
        "nothing forged, nothing hidden"
    );
    assert!(!seen.diagnostics.iter().any(|d| d.contains("doctored")));
    // Garbage and an older version: nothing is seeded, everything is rebuilt.
    for bytes in [&b"\x00garbage"[..], br#"{"version":1,"records":{}}"#] {
        std::fs::write(layout.home_catalogue_file(), bytes).unwrap();
        let (_, fresh) = process(&layout);
        let rebuilt = fresh.metadata().await.unwrap();
        assert!(rebuilt.rebuilt);
        assert_eq!(titles(&rebuilt), titles(&first));
        assert!(index(&layout)["rejected"]["chat-rot.json"].is_object());
    }
}
