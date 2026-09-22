//! The owner's exit ends exactly this session's emptied `retained`
//! environments (#2070): not a running one, not a restored one, not one
//! that still has a member — and nothing at all while the slot is empty.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::application::environments::ports::{
    EnvironmentMemberShutdown, EnvironmentProcessCommands, MemberShutdownReport, PortFuture,
};
use crate::application::environments::use_cases::{KillEnvironment, ListEnvironmentsQuery};
use crate::domain::environment_registry::EnvironmentRegistry;
use crate::infrastructure::tools::agent_cmd_containers::EnvironmentControl;

struct Commands {
    killed: Mutex<Vec<String>>,
    refuse: Option<String>,
    /// A kill that takes 300 ms (a slow runtime).
    hang: Option<String>,
}

impl EnvironmentProcessCommands for Commands {
    fn run_retained_inspect<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { panic!("never inspects") })
    }

    fn run_retained_cleanup<'a>(&'a self, _: &'a str, _: &'a [String]) -> PortFuture<'a, ()> {
        Box::pin(async { panic!("never cleans up: every record retains a kill") })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        _: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.killed.lock().unwrap().push(environment_id.to_string());
            if let Some(slow) = &self.hang
                && environment_id.ends_with(slow)
            {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            match &self.refuse {
                Some(detail) if environment_id.ends_with(detail) => Err("kill refused".into()),
                _ => Ok(()),
            }
        })
    }
}

struct NoMembers(AtomicUsize);

impl EnvironmentMemberShutdown for NoMembers {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        self.0.fetch_add(members.len(), Ordering::SeqCst);
        Box::pin(async { MemberShutdownReport::default() })
    }
}

fn record(
    reg: &EnvironmentRegistry,
    status: EnvironmentStatus,
    origin: EnvironmentOrigin,
    members: &[&str],
) -> String {
    let env_ref = reg.mint_ref().unwrap();
    reg.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: format!("runtime-{env_ref}"),
        environment_uuid: format!("uuid-{env_ref}"),
        name: None,
        workspace_path: std::path::PathBuf::from(format!("/ws/{env_ref}")),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv: vec!["kill.sh".to_string()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: members.iter().map(|m| m.to_string()).collect(),
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin,
        created_by: String::new(),
        created_at: None,
    });
    env_ref
}

fn control(reg: &EnvironmentRegistry, commands: Arc<Commands>) -> EnvironmentControl {
    EnvironmentControl {
        list: Arc::new(ListEnvironmentsQuery::new(reg.clone())),
        kill: Arc::new(KillEnvironment::new(
            reg.clone(),
            Arc::new(NoMembers(AtomicUsize::new(0))),
            commands,
        )),
    }
}

#[tokio::test]
async fn only_this_sessions_emptied_retained_environments_are_ended() {
    use EnvironmentOrigin::{Created, Restored};
    use EnvironmentStatus::{Retained, Running, Stopped};
    let reg = EnvironmentRegistry::new();
    let running = record(&reg, Running, Created, &["a"]);
    let emptied = record(&reg, Retained, Created, &[]);
    let restored = record(&reg, Retained, Restored, &[]);
    let stopped = record(&reg, Stopped, Created, &[]);
    let refused = record(&reg, Retained, Created, &[]);
    let commands = Arc::new(Commands {
        killed: Mutex::new(vec![]),
        refuse: Some(refused.clone()),
        hang: None,
    });
    let slot = EnvironmentControlSlot::default();
    assert!(slot.install(control(&reg, commands.clone())));

    let ended = SlotRetainedEnvironmentTeardown::new(slot)
        .end_emptied_retained()
        .await;
    assert_eq!(
        ended,
        [
            (emptied.clone(), Ok(())),
            (
                refused.clone(),
                Err(format!(
                    "environment {refused} cleanup failed: kill refused; state is cleanup-failed, retry kill_container"
                ))
            )
        ]
    );
    assert_eq!(
        *commands.killed.lock().unwrap(),
        [format!("runtime-{emptied}"), format!("runtime-{refused}")]
    );
    assert_eq!(reg.get(&emptied).unwrap().status, Stopped);
    assert_eq!(reg.get(&running).unwrap().status, Running);
    assert_eq!(reg.get(&restored).unwrap().status, Retained);
    assert_eq!(reg.get(&stopped).unwrap().status, Stopped);
}

#[tokio::test]
async fn an_empty_slot_ends_nothing() {
    let ended = SlotRetainedEnvironmentTeardown::new(EnvironmentControlSlot::default())
        .end_emptied_retained()
        .await;
    assert!(ended.is_empty());
}

#[test]
fn the_rule_is_created_retained_and_empty() {
    use EnvironmentOrigin::{Created, Restored};
    use EnvironmentStatus::{Retained, Running};
    let reg = EnvironmentRegistry::new();
    for (status, origin, members, expected) in [
        (Retained, Created, &[][..], true),
        (Retained, Created, &["m"][..], false),
        (Retained, Restored, &[][..], false),
        (Running, Created, &[][..], false),
    ] {
        let label = format!("{status:?} {origin:?} {members:?}");
        let env_ref = record(&reg, status, origin, members);
        assert_eq!(
            ends_on_owner_exit(&reg.get(&env_ref).unwrap()),
            expected,
            "{label}"
        );
    }
}

#[tokio::test]
async fn the_kills_run_concurrently_and_a_failed_one_leaves_a_truthful_record() {
    use EnvironmentOrigin::Created;
    use EnvironmentStatus::Retained;
    let reg = EnvironmentRegistry::new();
    let slow = record(&reg, Retained, Created, &[]);
    let refused = record(&reg, Retained, Created, &[]);
    let commands = Arc::new(Commands {
        killed: Mutex::new(vec![]),
        refuse: Some(refused.clone()),
        hang: Some(slow.clone()),
    });
    let slot = EnvironmentControlSlot::default();
    assert!(slot.install(control(&reg, commands.clone())));
    let ended = tokio::time::timeout(
        Duration::from_secs(5),
        SlotRetainedEnvironmentTeardown::new(slot).end_emptied_retained(),
    )
    .await
    .expect("a slow kill does not serialise the others");
    assert_eq!(ended.len(), 2);
    assert_eq!(ended[0].0, slow);
    assert!(ended[0].1.is_ok(), "{:?}", ended[0]);
    assert!(ended[1].1.is_err(), "{:?}", ended[1]);
    assert_eq!(reg.get(&slow).unwrap().status, EnvironmentStatus::Stopped);
    assert_eq!(
        reg.get(&refused).unwrap().status,
        EnvironmentStatus::CleanupFailed
    );
}
