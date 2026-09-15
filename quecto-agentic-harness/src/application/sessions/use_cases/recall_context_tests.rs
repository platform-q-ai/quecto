use super::RecallContext;
use crate::application::sessions::dto::retained_context::{
    RecallError, RecallOutcome, RecallQuery,
};
use crate::application::sessions::use_cases::retention_rig::JournalingRetention;
use crate::domain::session_identity::{SessionIdentity, SpillId};

fn cli(name: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(format!("cli:{name}"))
}

#[tokio::test]
async fn index_lists_every_retained_entry_in_exact_append_order_with_one_store_call() {
    let store = JournalingRetention::seeded(&cli("s"), &["turn2:bash:0", "turn1:msg:user", "a"]);
    let recall = RecallContext::new(store.clone());

    let outcome = recall.recall(&cli("s"), &RecallQuery::Index).await.unwrap();

    let RecallOutcome::Index(entries) = outcome else {
        panic!("the index query answers the index");
    };
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["turn2:bash:0", "turn1:msg:user", "a"]);
    assert_eq!(
        store.journal(),
        vec!["list cli:s"],
        "one index read: no per-entry load, no recall"
    );
}

#[tokio::test]
async fn index_of_an_empty_namespace_is_empty() {
    let recall = RecallContext::new(JournalingRetention::new());
    let outcome = recall.recall(&cli("s"), &RecallQuery::Index).await.unwrap();
    assert!(matches!(outcome, RecallOutcome::Index(entries) if entries.is_empty()));
}

#[tokio::test]
async fn a_known_id_answers_the_entry_with_its_content_in_one_store_call() {
    let store = JournalingRetention::seeded(&cli("s"), &["turn1:bash:0", "turn1:bash:1"]);
    let recall = RecallContext::new(store.clone());

    let outcome = recall
        .recall(&cli("s"), &RecallQuery::Entry(SpillId::new("turn1:bash:1")))
        .await
        .unwrap();

    let RecallOutcome::Entry(entry) = outcome else {
        panic!("a known id answers its entry");
    };
    assert_eq!(entry.id, "turn1:bash:1");
    assert_eq!(entry.content, "content of turn1:bash:1");
    assert_eq!(store.journal(), vec!["recall turn1:bash:1 cli:s"]);
}

#[tokio::test]
async fn an_unknown_id_is_missing() {
    let store = JournalingRetention::seeded(&cli("s"), &["turn1:bash:0"]);
    let recall = RecallContext::new(store.clone());
    let outcome = recall
        .recall(&cli("s"), &RecallQuery::Entry(SpillId::new("turn9:bash:0")))
        .await
        .unwrap();
    assert!(matches!(outcome, RecallOutcome::Missing(id) if id.as_str() == "turn9:bash:0"));
}

#[tokio::test]
async fn another_sessions_entry_is_missing_under_this_identity() {
    let store = JournalingRetention::seeded(&cli("a"), &["turn1:bash:0"]);
    let recall = RecallContext::new(store.clone());
    let outcome = recall
        .recall(&cli("b"), &RecallQuery::Entry(SpillId::new("turn1:bash:0")))
        .await
        .unwrap();
    assert!(matches!(outcome, RecallOutcome::Missing(_)));
    let index = recall.recall(&cli("b"), &RecallQuery::Index).await.unwrap();
    assert!(matches!(index, RecallOutcome::Index(entries) if entries.is_empty()));
}

#[tokio::test]
async fn a_malformed_id_is_refused_before_any_lookup() {
    let store = JournalingRetention::seeded(&cli("s"), &["turn1:bash:0"]);
    let recall = RecallContext::new(store.clone());
    let err = RecallQuery::parse("").expect_err("the empty id is malformed");
    assert!(matches!(err, RecallError::MalformedId));
    assert!(store.journal().is_empty());
    // The tool maps the refusal to its not-found result; the use case itself
    // is never asked for the empty id.
    let _ = recall;
}

#[tokio::test]
async fn store_errors_surface_as_recall_errors() {
    let store = JournalingRetention::seeded(&cli("s"), &["turn1:bash:0"]);
    *store.fail_recall.lock().unwrap() = true;
    *store.fail_list.lock().unwrap() = true;
    let recall = RecallContext::new(store.clone());
    let entry = recall
        .recall(&cli("s"), &RecallQuery::Entry(SpillId::new("turn1:bash:0")))
        .await
        .expect_err("recall failure surfaces");
    assert!(matches!(entry, RecallError::Store(_)));
    assert!(entry.to_string().contains("recall failed"), "{entry}");
    let index = recall
        .recall(&cli("s"), &RecallQuery::Index)
        .await
        .expect_err("index failure surfaces");
    assert!(index.to_string().contains("list failed"), "{index}");
}

#[tokio::test]
async fn list_and_clear_reach_the_namespace_of_the_identity_only() {
    let store = JournalingRetention::seeded(&cli("a"), &["x"]);
    store.append_for_test(&cli("b"), "y").await;
    let recall = RecallContext::new(store.clone());
    store.clear_journal();

    assert_eq!(recall.list(&cli("a")).await.unwrap().len(), 1);
    recall.clear(&cli("a")).await.unwrap();
    assert!(recall.list(&cli("a")).await.unwrap().is_empty());
    assert_eq!(
        store.ids_of(&cli("b")),
        vec!["y"],
        "the other namespace stands"
    );
    assert_eq!(
        store.journal(),
        vec!["list cli:a", "clear cli:a", "list cli:a"]
    );
    *store.fail_clear.lock().unwrap() = true;
    assert!(recall.clear(&cli("b")).await.is_err());
}

#[tokio::test]
async fn scrub_ephemeral_removes_the_ephemeral_namespace_only_for_an_ephemeral_run() {
    let store = JournalingRetention::seeded(&SessionIdentity::ephemeral(), &["turn1:bash:0"]);
    store.append_for_test(&cli("named"), "kept").await;
    let recall = RecallContext::new(store.clone());
    store.clear_journal();

    recall.scrub_ephemeral(false);
    assert!(store.journal().is_empty(), "a named run scrubs nothing");
    assert_eq!(
        store.ids_of(&SessionIdentity::ephemeral()),
        vec!["turn1:bash:0"]
    );

    recall.scrub_ephemeral(true);
    assert_eq!(store.journal(), vec!["scrub "]);
    assert!(store.ids_of(&SessionIdentity::ephemeral()).is_empty());
    assert_eq!(store.ids_of(&cli("named")), vec!["kept"]);
}

#[test]
fn debug_names_the_use_case_only() {
    assert_eq!(
        format!("{:?}", RecallContext::new(JournalingRetention::new())),
        "RecallContext { .. }"
    );
}
