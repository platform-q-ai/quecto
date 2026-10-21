//! #2681: a kill reports the environments its teardown kept.
use std::sync::Arc;

use super::tests::{bounded, kill, rig, tree_registry};
use crate::application::subagents::ports::kept_environments::{KeptEnvironment, KeptEnvironments};
use crate::application::subagents::ports::owned_child_termination::TerminationConclusion;
use crate::domain::identity::ids::AgentUuid;

use super::super::teardown_fakes::root_tree;

/// The session's environments as a kill's teardown leaves them: the
/// memberships asked for, and the environments asked about.
struct FakeKept {
    memberships: Vec<String>,
    kept: Vec<KeptEnvironment>,
    asked: std::sync::Mutex<Vec<String>>,
}

impl KeptEnvironments for FakeKept {
    fn memberships(&self, agents: &[AgentUuid]) -> Vec<String> {
        let agents: Vec<&str> = agents.iter().map(AgentUuid::as_str).collect();
        self.asked
            .lock()
            .unwrap()
            .push(format!("memberships {}", agents.join(",")));
        self.memberships.clone()
    }

    fn kept(&self, environments: &[String]) -> Vec<KeptEnvironment> {
        self.asked
            .lock()
            .unwrap()
            .push(format!("kept {}", environments.join(",")));
        self.kept.clone()
    }
}

/// #2681: a kill whose teardown kept an environment of the target's
/// subtree reports it, with the reason its record holds; the subtree's
/// memberships are read before the teardown takes them.
#[tokio::test]
async fn a_kill_reports_the_environments_its_teardown_kept() {
    let kept = KeptEnvironment {
        environment_ref: "C6".into(),
        reason: "kept so its work can be pushed".into(),
    };
    let environments = Arc::new(FakeKept {
        memberships: vec!["C6".into()],
        kept: vec![kept.clone()],
        asked: std::sync::Mutex::new(Vec::new()),
    });
    let reporting = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::ExitedAfterProtocol,
    );
    let use_case = reporting.use_case.reporting_kept(environments.clone());
    let outcome = bounded(use_case.execute(kill("A"))).await.unwrap();
    assert_eq!(outcome.kept, [kept]);
    assert_eq!(
        environments.asked.lock().unwrap().as_slice(),
        ["memberships A,B,C", "kept C6"]
    );
}

/// A subtree that was a member of no environment asks nothing more, and
/// a kill with no report composed reports nothing.
#[tokio::test]
async fn a_kill_of_agents_in_no_environment_reports_none_kept() {
    let environments = Arc::new(FakeKept {
        memberships: Vec::new(),
        kept: Vec::new(),
        asked: std::sync::Mutex::new(Vec::new()),
    });
    let reporting = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::ExitedAfterProtocol,
    );
    let use_case = reporting.use_case.reporting_kept(environments.clone());
    let outcome = bounded(use_case.execute(kill("D"))).await.unwrap();
    assert!(outcome.kept.is_empty());
    assert_eq!(
        environments.asked.lock().unwrap().as_slice(),
        ["memberships D"]
    );
    let silent = rig(
        tree_registry(),
        root_tree(),
        TerminationConclusion::ExitedAfterProtocol,
    );
    let outcome = bounded(silent.use_case.execute(kill("A"))).await.unwrap();
    assert!(outcome.kept.is_empty());
}
