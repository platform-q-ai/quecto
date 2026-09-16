use super::*;
use crate::application::sessions::dto::retained_context::{RecallOutcome, RecallQuery};
use crate::domain::session::SpillEntry;
use crate::domain::session_identity::{SessionIdentity, SpillId};

fn entry(id: &str) -> SpillEntry {
    SpillEntry {
        id: id.to_string(),
        tool: "bash".to_string(),
        input_preview: "ls".to_string(),
        tokens: 3,
        content: format!("content of {id}"),
    }
}

#[tokio::test]
async fn the_file_retention_graph_shares_one_store_across_writer_reader_and_recall() {
    let tmp = tempfile::tempdir().unwrap();
    let handles = super::super::sessions::build_retention_handles(tmp.path());
    let identity = SessionIdentity::named_cli("composed").unwrap();

    let receipt = handles
        .context
        .retain
        .retain(&identity, &entry("turn1:bash:0"))
        .await
        .unwrap();
    assert_eq!(receipt.id, "turn1:bash:0");
    assert!(
        handles
            .context
            .list
            .retains_entries(&identity)
            .await
            .unwrap()
    );
    let recalled = handles
        .recall
        .recall(&identity, &RecallQuery::Entry(SpillId::new("turn1:bash:0")))
        .await
        .unwrap();
    assert!(matches!(recalled, RecallOutcome::Entry(e) if e.content == "content of turn1:bash:0"));
    assert!(
        handles
            .store
            .recall(&identity, &SpillId::new("turn1:bash:0"))
            .await
            .unwrap()
            .is_some(),
        "the store handle is the same store the graph writes"
    );
    assert!(
        tmp.path()
            .join("sessions/cli_composed/spill.jsonl")
            .is_file(),
        "the composed retention store writes the flat layout"
    );

    handles.recall.clear(&identity).await.unwrap();
    assert!(
        !handles
            .context
            .list
            .retains_entries(&identity)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn the_graph_over_a_supplied_store_uses_it_as_is() {
    let tmp = tempfile::tempdir().unwrap();
    let store: Arc<dyn ContextSpillStore> = Arc::new(
        crate::infrastructure::persistence::context_spill::FileContextSpillStore::new(
            crate::infrastructure::persistence::session_layout::FlatSessionLayout::new(tmp.path()),
        ),
    );
    let handles = retention_handles_over(store.clone());
    let ephemeral = SessionIdentity::ephemeral();
    handles
        .context
        .retain
        .retain(&ephemeral, &entry("turn1:bash:0"))
        .await
        .unwrap();
    assert!(store.has_entries(&ephemeral).await.unwrap());
    assert!(tmp.path().join("sessions/key_/spill.jsonl").is_file());

    handles.recall.scrub_ephemeral(true);
    assert!(
        !tmp.path().join("sessions/key_").exists(),
        "the ephemeral namespace is scrubbed through the composed graph"
    );
    assert_eq!(format!("{handles:?}"), "RetentionHandles { .. }");
}
