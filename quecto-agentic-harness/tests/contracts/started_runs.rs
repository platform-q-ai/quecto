//! The started runs port's contract (#2655), run against the spawn-backed
//! coordinator launcher: it names the runs its launches started, so a
//! session's end can end those it leaves open; a refused launch started
//! none.
use std::sync::Arc;

use super::coordinator_launch::{RUNNING, launched, launched_child, launcher, spawn, spawned_with};
use quecto::application::project_board::ports::StartedRuns;
use quecto::domain::project_board::value_objects::slug::Slug;

/// #2655: the launcher names the runs its launches started, so a
/// session's end can end those it leaves open; a launch refused started
/// nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_runs_a_launcher_started_are_the_ones_it_launched() {
    let spawned = spawned_with(RUNNING, false, Some(&launched_child(Some("C4"))));
    let (launching, _registry) = launcher(spawned, true);
    let launching = Arc::new(launching);
    assert!(launching.started().is_empty(), "nothing launched yet");
    assert!(launched(Arc::clone(&launching)).await.is_ok());
    assert_eq!(
        launching.started(),
        [(Slug::parse("demo-hello").unwrap(), 1)]
    );
    let (refusing, _registry) = launcher(spawn("no container runtime", true), true);
    let refusing = Arc::new(refusing);
    assert!(launched(Arc::clone(&refusing)).await.is_err());
    assert!(
        refusing.started().is_empty(),
        "a refused launch started none"
    );
}
