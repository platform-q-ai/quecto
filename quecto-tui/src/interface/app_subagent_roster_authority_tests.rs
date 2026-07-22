use super::app_subagents_tests::{harness, info, info_with_parent};
#[tokio::test]
async fn source_scoped_roster_accepts_recursive_descendants_in_one_event() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running")]);

    a.update_subagent_bar_from_source(
        Some("a"),
        vec![
            info_with_parent("a1", "running", "a"),
            info_with_parent("g1", "running", "a1"),
        ],
    );

    assert_eq!(
        a.subagents.tracked["g1"].info.parent_id.as_deref(),
        Some("a1")
    );
}

#[tokio::test]
async fn source_scoped_roster_cannot_reparent_existing_root() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running"), info("b", "running")]);

    a.update_subagent_bar_from_source(Some("a"), vec![info_with_parent("b", "idle", "a")]);

    assert_eq!(a.subagents.tracked["b"].info.parent_id, None);
}

#[tokio::test]
async fn direct_child_metadata_survives_later_master_snapshot() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running")]);
    a.update_subagent_bar_from_source(Some("a"), vec![info_with_parent("a1", "idle", "a")]);

    a.update_subagent_bar(vec![
        info("a", "running"),
        info_with_parent("a1", "running", "a"),
    ]);

    assert_eq!(a.subagents.tracked["a1"].info.status, "idle");
}
