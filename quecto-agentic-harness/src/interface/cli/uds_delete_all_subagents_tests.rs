//! `delete_all_subagents` presents the fleet teardown (#1938): the use case
//! is invoked once and its outcome mapped onto the wire; nothing is drained
//! here.
use std::sync::Arc;

use crate::application::subagents::use_cases::teardown_fakes::*;
use crate::application::subagents::use_cases::{TerminateAllDelegatedAgents, lifecycle_fakes};

fn fleet_with(
    lineage: crate::domain::subagent_teardown::LineageSnapshot,
) -> (
    Arc<TerminateAllDelegatedAgents>,
    Arc<lifecycle_fakes::FakeRegistry>,
) {
    let lifecycle = FakeLifecycle::new(lineage);
    let routing = FakeRouting::new();
    let fleet = fake_fleet(lifecycle, routing, FakeSpawner::new());
    (fleet.fleet, fleet.registry)
}

#[tokio::test]
async fn presents_every_settled_child_and_the_removed_count() {
    let (fleet, registry) = fleet_with(root_tree());
    let event = super::respond(Some(&fleet), Some("del-1")).await;
    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["command"], "delete_all_subagents");
    assert_eq!(value["id"], "del-1");
    assert_eq!(value["success"], true, "{value}");
    // A and D settled and their tombstones were pruned: two rows moved.
    assert_eq!(value["data"]["removed"], 2);
    assert_eq!(value["data"]["joined"], false);
    let settled = value["data"]["settled"].as_array().unwrap();
    assert_eq!(settled.len(), 2);
    assert_eq!(settled[0]["agent"], "A");
    assert_eq!(settled[0]["result"], "graceful");
    assert_eq!(settled[1]["agent"], "D");
    assert_eq!(value["data"]["unsettled"].as_array().map(Vec::len), Some(0));
    assert!(registry.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_empty_fleet_reports_nothing_removed() {
    let (fleet, _) = fleet_with(crate::domain::subagent_teardown::LineageSnapshot {
        owner: crate::domain::ids::AgentUuid::new("root"),
        records: Vec::new(),
    });
    let event = super::respond(Some(&fleet), None).await;
    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["success"], true);
    assert_eq!(value["data"]["removed"], 0);
}

#[tokio::test]
async fn an_unsettled_child_is_presented_with_its_detail() {
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let fleet = fake_fleet(lifecycle, routing, FakeSpawner::new());
    fleet.termination.conclude_child_with(
        "A",
        crate::application::subagents::ports::TerminationConclusion::StillRunning(
            "would not die".into(),
        ),
    );
    let event = super::respond(Some(&fleet.fleet), Some("del-2")).await;
    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["success"], true);
    let unsettled = value["data"]["unsettled"].as_array().unwrap();
    assert_eq!(unsettled.len(), 1);
    assert_eq!(unsettled[0]["agent"], "A");
    assert_eq!(unsettled[0]["detail"], "would not die");
    assert_eq!(value["data"]["removed"], 1, "D settled and was pruned");
}

#[tokio::test]
async fn without_a_fleet_teardown_the_error_is_correlated() {
    let event = super::respond(None, Some("del-3")).await;
    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["success"], false);
    assert_eq!(value["id"], "del-3");
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("no sub-agent registry"),
        "{value}"
    );
}

#[tokio::test]
async fn an_interrupted_run_is_a_correlated_error() {
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let spawner = FakeSpawner::new();
    spawner
        .drop_next
        .store(1, std::sync::atomic::Ordering::SeqCst);
    let fleet = fake_fleet(lifecycle, routing, spawner);
    let event = super::respond(Some(&fleet.fleet), Some("del-4")).await;
    let value = serde_json::to_value(&event).unwrap();
    assert_eq!(value["success"], false);
    assert!(
        value["error"].as_str().unwrap().contains("interrupted"),
        "{value}"
    );
}
