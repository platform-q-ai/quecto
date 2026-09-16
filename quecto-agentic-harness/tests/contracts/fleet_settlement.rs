//! Contract for the `FleetSettlement` port (#1938, D7 #1976) over the
//! production fleet teardown: an empty fleet settles with nothing removed;
//! a launched child whose endpoint is dead and whose process is not owned
//! settles unobserved and is pruned (one row removed); a record-only row
//! is never a direct child and is left alone. Built only from the public
//! crate surface.
use std::sync::{Arc, Mutex};

use quecto::application::sessions::dto::FleetSettlementOutcome;
use quecto::application::sessions::ports::FleetSettlement;
use quecto::composition::subagent_teardown::{FleetTeardownWiring, build_fleet_teardown};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::LaunchGeneration;
use quecto::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle;
use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};

fn under_test(registry: SubagentRegistry) -> Arc<dyn FleetSettlement> {
    build_fleet_teardown(FleetTeardownWiring {
        owner: AgentUuid::new("me"),
        registry,
        broadcast_tx: None,
        notify_tx: None,
        harness_lifecycle: new_shared_harness_lifecycle(),
    })
}

#[tokio::test]
async fn an_empty_fleet_settles_with_nothing_removed() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    let FleetSettlementOutcome::Settled(settled) = under_test(registry).settle_fleet().await else {
        panic!("an empty fleet settles");
    };
    assert_eq!(settled.settled, 0);
    assert_eq!(settled.pruned, 0);
    assert_eq!(settled.removed, 0);
    assert!(!settled.joined);
}

#[tokio::test]
async fn a_launched_child_with_a_dead_endpoint_settles_and_is_pruned() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    {
        let mut entries = registry.lock().unwrap();
        let mut launched = SubagentEntry::with_identity(
            AgentUuid::new("kid"),
            "kid".into(),
            std::env::temp_dir().join("q-fs-no-such-child.sock"),
            0,
        );
        launched.launch_generation = Some(LaunchGeneration::new(3));
        entries.insert("kid".into(), launched);
        entries.insert(
            "restored".into(),
            SubagentEntry::new("/tmp/restored.sock".into(), 99),
        );
    }
    let FleetSettlementOutcome::Settled(settled) =
        under_test(registry.clone()).settle_fleet().await
    else {
        panic!("a dead endpoint settles unobserved");
    };
    assert_eq!(settled.settled, 1);
    assert_eq!(settled.removed, 1);
    let entries = registry.lock().unwrap();
    assert!(!entries.contains_key("kid"), "settled and pruned");
    assert!(entries.contains_key("restored"), "never a direct child");
}
