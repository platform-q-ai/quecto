//! Unit coverage of the in-memory rejection cache and the walk's seeding
//! (#2010 R1-H1, R2-H1, R2-H2) at the adapter's own level — observable without the
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

fn named(snapshot: &SessionMetadataSnapshot, file: &str) -> usize {
    let lines = snapshot.diagnostics.iter();
    lines.filter(|d| d.starts_with(file)).count()
}

#[tokio::test]
async fn rejected_versions_are_named_on_every_answer_and_never_indexed() {
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
            assert_eq!(
                named(&snapshot, file),
                1,
                "{pass}: {:?}",
                snapshot.diagnostics
            );
        }
    }
    let index = index(&layout);
    assert!(index.get("rejected").is_none(), "{index}");
    assert_eq!(index["records"].as_object().unwrap().len(), 1, "{index}");
    assert!(index["records"]["chat-good"]["summary"].is_object());
}

#[tokio::test]
async fn a_new_process_answers_the_same_and_a_repair_clears_the_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let (store, catalogue) = process(&layout);
    mixed(&layout, &store).await;
    let first = catalogue.metadata().await.unwrap();
    // The listing alone (no summary walk) names the same rejections.
    let (_, listing_only) = process(&layout);
    let listing = listing_only.list().unwrap();
    assert_eq!(listing.entries.len(), 1, "{listing:?}");
    assert_eq!(listing.diagnostics.len(), 2, "{listing:?}");
    let (_, cold) = process(&layout);
    let seeded = cold.metadata().await.unwrap();
    assert_eq!(titles(&seeded), titles(&first));
    assert_eq!(seeded.diagnostics, first.diagnostics);
    assert!(!seeded.rebuilt);
    // Repaired in place: read again and listed.
    let mut repaired = ROT.to_vec();
    repaired.extend_from_slice(b"\"}]}");
    std::fs::write(layout.session_file(&identity("chat-rot")), repaired).unwrap();
    let healed = cold.metadata().await.unwrap();
    assert_eq!(titles(&healed), ["AAAA", "a good title", "cut short title"]);
    assert_eq!(named(&healed, "chat-rot.json: "), 0);
    // Deleted: forgotten.
    std::fs::remove_file(layout.session_file(&identity("chat-cut"))).unwrap();
    let after = cold.metadata().await.unwrap();
    assert!(after.diagnostics.is_empty(), "{:?}", after.diagnostics);
}

/// R2-H1: a failed READ is no verdict. Reported for that answer, by file
/// name — the one read serves both halves (#2042), so the strict catalogue's
/// own line names it; read again on the next query.
#[tokio::test]
async fn a_transient_read_failure_is_named_for_that_answer_and_retried() {
    use super::session_record_read::fail_next_reads;
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let (store, catalogue) = process(&layout);
    save(&store, "chat-good", "a good title").await;
    let good = layout.session_file(&identity("chat-good"));
    for (failures, why) in [(1, "session record unavailable")] {
        let (_, cold) = process(&layout);
        let _ = std::fs::remove_file(layout.home_catalogue_file());
        fail_next_reads(&good, failures);
        let starved = cold.metadata().await.unwrap();
        assert!(titles(&starved).is_empty(), "{failures}");
        assert_eq!(
            named(&starved, "chat-good.json: "),
            1,
            "{:?}",
            starved.diagnostics
        );
        assert!(
            starved.diagnostics[0].contains(why),
            "{:?}",
            starved.diagnostics
        );
        assert!(starved.diagnostics[0].contains("injected read failure"));
        let healed = cold.metadata().await.unwrap();
        assert_eq!(titles(&healed), ["a good title"], "{failures}");
        assert!(healed.diagnostics.is_empty(), "{:?}", healed.diagnostics);
    }
    drop(catalogue);
}

/// R2-H2: a `rejected` map (one pre-release head wrote it) is ignored — even
/// at the correct stamp of a valid record — and republished away, silently.
#[tokio::test]
async fn a_persisted_rejection_hides_nothing_and_an_unusable_index_is_rebuilt() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let (store, catalogue) = process(&layout);
    mixed(&layout, &store).await;
    let first = catalogue.metadata().await.unwrap();
    let mut doctored = index(&layout);
    let good = doctored["records"]
        .as_object_mut()
        .unwrap()
        .remove("chat-good");
    let stamp = good.unwrap()["stamp"].clone();
    doctored["rejected"] = serde_json::json!({
        "chat-good.json": {"stamp": stamp, "reason": "doctored", "unlisted": true},
        "chat-cut.json": {"stamp": stamp, "reason": "doctored",
            "listed": ["chat-good", "forged title", 9]},
    });
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
    assert_eq!(seen.diagnostics, first.diagnostics);
    assert!(!seen.rebuilt, "a legacy field is not corruption");
    assert!(index(&layout).get("rejected").is_none(), "not written back");
    // Garbage and an older version: nothing is seeded, everything is rebuilt.
    for bytes in [&b"\x00garbage"[..], br#"{"version":1,"records":{}}"#] {
        std::fs::write(layout.home_catalogue_file(), bytes).unwrap();
        let (_, fresh) = process(&layout);
        let rebuilt = fresh.metadata().await.unwrap();
        assert!(rebuilt.rebuilt);
        assert_eq!(titles(&rebuilt), titles(&first));
        assert_eq!(named(&rebuilt, "chat-rot.json: "), 1);
    }
}

/// R3-H6: a legacy `rejected` key is republished away whatever it holds —
/// `null` is present too.
#[tokio::test]
async fn a_null_legacy_rejected_key_is_republished_away_like_any_other() {
    let dir = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(dir.path().join("base"));
    let (store, catalogue) = process(&layout);
    save(&store, "chat-good", "a good title").await;
    let first = catalogue.metadata().await.unwrap();
    for legacy in [serde_json::Value::Null, serde_json::json!({})] {
        let mut doctored = index(&layout);
        doctored["rejected"] = legacy.clone();
        let bytes = serde_json::to_vec(&doctored).unwrap();
        std::fs::write(layout.home_catalogue_file(), bytes).unwrap();
        let (_, cold) = process(&layout);
        let seen = cold.metadata().await.unwrap();
        assert_eq!(titles(&seen), titles(&first));
        assert!(!seen.rebuilt, "{legacy}: a legacy field is not corruption");
        assert!(index(&layout).get("rejected").is_none(), "{legacy}");
    }
}
