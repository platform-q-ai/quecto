use super::{ListRetainedContext, RetainContext};
use crate::application::sessions::use_cases::retention_rig::{JournalingRetention, entry};
use crate::domain::session_identity::SessionIdentity;

fn cli(name: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(format!("cli:{name}"))
}

#[tokio::test]
async fn retain_appends_the_caller_supplied_id_verbatim_and_receipts_it() {
    let store = JournalingRetention::new();
    let retain = RetainContext::new(store.clone());
    let e = entry("turn3:bash:1", "ls output");

    let receipt = retain.retain(&cli("s"), &e).await.unwrap();

    assert_eq!(receipt.id, "turn3:bash:1");
    assert_eq!(store.ids_of(&cli("s")), vec!["turn3:bash:1"]);
    assert_eq!(store.journal(), vec!["append turn3:bash:1 cli:s"]);
    assert_eq!(
        e.content, "ls output",
        "the entry is borrowed, not consumed"
    );
}

#[tokio::test]
async fn retain_never_deduplicates_a_tool_id_the_grammar_is_verbatim() {
    // `turn{n}:{tool}:{idx}` restarts its turn numbering every prompt: the
    // same id can be retained twice across prompts, exactly as before D9
    // (recall answers the first), and the stub the caller stamps stays the
    // id it allocated.
    let store = JournalingRetention::new();
    let retain = RetainContext::new(store.clone());
    retain
        .retain(&cli("s"), &entry("turn1:bash:0", "first"))
        .await
        .unwrap();
    let second = retain
        .retain(&cli("s"), &entry("turn1:bash:0", "second"))
        .await
        .unwrap();
    assert_eq!(second.id, "turn1:bash:0");
    assert_eq!(
        store.ids_of(&cli("s")),
        vec!["turn1:bash:0", "turn1:bash:0"]
    );
}

#[tokio::test]
async fn retain_returns_the_store_error_and_issues_no_receipt() {
    let store = JournalingRetention::new();
    *store.fail_append.lock().unwrap() = true;
    let retain = RetainContext::new(store.clone());

    let err = retain
        .retain(&cli("s"), &entry("turn1:bash:0", "x"))
        .await
        .expect_err("a failed append is reported");

    assert!(err.to_string().contains("append failed"), "{err}");
    assert!(store.ids_of(&cli("s")).is_empty(), "nothing retained");
}

#[tokio::test]
async fn retain_deduplicated_keeps_a_free_base_id_verbatim() {
    let store = JournalingRetention::seeded(&cli("s"), &["turn1:bash:0", "turn2:msg:user"]);
    let retain = RetainContext::new(store.clone());
    let mut e = entry("turn1:msg:assistant", "hello");

    let receipt = retain.retain_deduplicated(&cli("s"), &mut e).await.unwrap();

    assert_eq!(receipt.id, "turn1:msg:assistant");
    assert_eq!(e.id, "turn1:msg:assistant");
    assert_eq!(
        store.journal(),
        vec!["list cli:s", "append turn1:msg:assistant cli:s"],
        "one index read and one append, no rescan"
    );
}

#[tokio::test]
async fn retain_deduplicated_suffixes_a_present_base_id_from_two() {
    let store = JournalingRetention::seeded(&cli("s"), &["turn1:msg:assistant"]);
    let retain = RetainContext::new(store.clone());
    let mut e = entry("turn1:msg:assistant", "again");

    let receipt = retain.retain_deduplicated(&cli("s"), &mut e).await.unwrap();

    assert_eq!(receipt.id, "turn1:msg:assistant:2");
    assert_eq!(
        e.id, receipt.id,
        "the entry carries the id it was retained under"
    );
    assert_eq!(
        store.ids_of(&cli("s")),
        vec!["turn1:msg:assistant", "turn1:msg:assistant:2"]
    );
}

#[tokio::test]
async fn retain_deduplicated_takes_the_highest_suffix_plus_one() {
    let store = JournalingRetention::seeded(
        &cli("s"),
        &[
            "turn1:msg:user",
            "turn1:msg:user:2",
            "turn1:msg:user:7",
            "turn1:msg:user:3",
        ],
    );
    let retain = RetainContext::new(store.clone());
    let mut e = entry("turn1:msg:user", "x");
    let receipt = retain.retain_deduplicated(&cli("s"), &mut e).await.unwrap();
    assert_eq!(receipt.id, "turn1:msg:user:8");
}

#[tokio::test]
async fn retain_deduplicated_counts_only_exact_base_and_numeric_suffixes() {
    // `turn1:msg:user` must not be confused with `turn1:msg:userx`,
    // `turn1:msg:user:` or `turn1:msg:user:abc`; a lone suffixed id without
    // its bare base still takes the next number.
    let store = JournalingRetention::seeded(
        &cli("s"),
        &[
            "turn1:msg:userx",
            "turn1:msg:user:",
            "turn1:msg:user:abc",
            "turn1:msg:user:4",
        ],
    );
    let retain = RetainContext::new(store.clone());
    let mut e = entry("turn1:msg:user", "x");
    let receipt = retain.retain_deduplicated(&cli("s"), &mut e).await.unwrap();
    assert_eq!(receipt.id, "turn1:msg:user:5");
}

#[tokio::test]
async fn retain_deduplicated_is_scoped_to_the_identity() {
    let store = JournalingRetention::seeded(&cli("a"), &["turn1:msg:user"]);
    let retain = RetainContext::new(store.clone());
    let mut e = entry("turn1:msg:user", "x");
    let receipt = retain.retain_deduplicated(&cli("b"), &mut e).await.unwrap();
    assert_eq!(
        receipt.id, "turn1:msg:user",
        "another session's ids do not collide"
    );
    assert_eq!(store.ids_of(&cli("b")), vec!["turn1:msg:user"]);
    assert_eq!(store.ids_of(&cli("a")), vec!["turn1:msg:user"]);
}

#[tokio::test]
async fn retain_deduplicated_appends_verbatim_when_the_index_cannot_be_read() {
    let store = JournalingRetention::seeded(&cli("s"), &["turn1:msg:user"]);
    *store.fail_list.lock().unwrap() = true;
    let retain = RetainContext::new(store.clone());
    let mut e = entry("turn1:msg:user", "x");
    let receipt = retain.retain_deduplicated(&cli("s"), &mut e).await.unwrap();
    assert_eq!(receipt.id, "turn1:msg:user");
}

#[tokio::test]
async fn retain_deduplicated_reports_an_append_failure_with_no_receipt() {
    let store = JournalingRetention::new();
    *store.fail_append.lock().unwrap() = true;
    let retain = RetainContext::new(store.clone());
    let mut e = entry("turn1:msg:user", "x");
    assert!(retain.retain_deduplicated(&cli("s"), &mut e).await.is_err());
    assert!(store.ids_of(&cli("s")).is_empty());
}

#[tokio::test]
async fn list_retained_context_reads_the_index_in_append_order_and_its_presence() {
    let store = JournalingRetention::seeded(&cli("s"), &["c", "a", "b"]);
    let list = ListRetainedContext::new(store.clone());

    let index = list.list(&cli("s")).await.unwrap();
    let ids: Vec<&str> = index.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, vec!["c", "a", "b"], "oldest first, never sorted");
    assert!(list.has_entries(&cli("s")).await.unwrap());
    assert!(!list.has_entries(&cli("other")).await.unwrap());
    assert_eq!(
        store.journal(),
        vec!["list cli:s", "has_entries cli:s", "has_entries cli:other"]
    );
}

#[tokio::test]
async fn list_retained_context_propagates_store_errors() {
    let store = JournalingRetention::new();
    *store.fail_list.lock().unwrap() = true;
    let list = ListRetainedContext::new(store.clone());
    assert!(list.list(&cli("s")).await.is_err());
    assert!(list.has_entries(&cli("s")).await.is_err());
}

#[test]
fn handles_debug_without_exposing_the_store() {
    let store = JournalingRetention::new();
    assert_eq!(
        format!("{:?}", RetainContext::new(store.clone())),
        "RetainContext { .. }"
    );
    assert_eq!(
        format!("{:?}", ListRetainedContext::new(store)),
        "ListRetainedContext { .. }"
    );
}
