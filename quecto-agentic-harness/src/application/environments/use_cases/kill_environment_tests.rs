//! `kill_container` over fakes (#1369 slice 2, #1939): members are asked
//! first and the retained kill runs exactly once, only after every member
//! settled; every other path leaves a truthful, retryable state.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::application::environments::ports::{
    EnvironmentMemberShutdown, EnvironmentProcessCommands, MemberShutdownReport,
    MemberShutdownResult, PortFuture, SettledMember, UnsettledMember,
};
use crate::application::environments::use_cases::{KillEnvironment, KillEnvironmentError};
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, EnvironmentTarget,
};

/// What ran, in order: `members(<uuids>)` and `kill`.
type Journal = Arc<Mutex<Vec<String>>>;

struct SpyCommands {
    journal: Journal,
    kills: AtomicUsize,
    fail_kills: Mutex<usize>,
    /// Parks inside the kill until released, so a second caller can
    /// overlap an in-flight kill.
    gate: Option<Arc<tokio::sync::Notify>>,
}

impl SpyCommands {
    fn new(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            kills: AtomicUsize::new(0),
            fail_kills: Mutex::new(0),
            gate: None,
        }
    }

    fn failing(journal: &Journal, times: usize) -> Self {
        let commands = Self::new(journal);
        *commands.fail_kills.lock().unwrap() = times;
        commands
    }
}

impl EnvironmentProcessCommands for SpyCommands {
    fn run_retained_inspect<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { panic!("kill_container never inspects") })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async move {
            assert_eq!(argv, ["kill.sh".to_string()], "the retained argv");
            assert!(environment_id.starts_with("runtime-"));
            self.journal.lock().unwrap().push("kill".into());
            self.kills.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            let mut failures = self.fail_kills.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                Err("kill.sh exited 1".to_string())
            } else {
                Ok(())
            }
        })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, ()> {
        Box::pin(async { panic!("kill_container never runs cleanup") })
    }
}

/// Settles every member it is asked about, except those listed as
/// unsettleable; removes settled members from the registry the way the
/// production compensation does (the environment is already claimed, so
/// the removal never mints a second claim).
struct FakeMembers {
    journal: Journal,
    registry: EnvironmentRegistry,
    env_ref: String,
    unsettleable: Vec<String>,
    asked: Mutex<Vec<Vec<String>>>,
}

impl EnvironmentMemberShutdown for FakeMembers {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        Box::pin(async move {
            self.journal
                .lock()
                .unwrap()
                .push(format!("members({})", members.join(",")));
            self.asked.lock().unwrap().push(members.to_vec());
            let mut report = MemberShutdownReport::default();
            for member in members {
                if self.unsettleable.contains(member) {
                    report.unsettled.push(UnsettledMember {
                        member: member.clone(),
                        detail: "exit not observed within the bound".into(),
                    });
                    continue;
                }
                let claim = self.registry.remove_member(&self.env_ref, member).unwrap();
                assert!(
                    claim.is_none(),
                    "a claimed environment mints no second claim"
                );
                report.settled.push(SettledMember {
                    member: member.clone(),
                    result: MemberShutdownResult::Graceful,
                });
            }
            report
        })
    }
}

fn committed_env(reg: &EnvironmentRegistry, members: &[&str]) -> String {
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
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: crate::domain::environment_registry::EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    });
    env_ref
}

struct Fixture {
    reg: EnvironmentRegistry,
    env_ref: String,
    journal: Journal,
    commands: Arc<SpyCommands>,
    members: Arc<FakeMembers>,
}

impl Fixture {
    fn new(members: &[&str], unsettleable: &[&str], commands: fn(&Journal) -> SpyCommands) -> Self {
        let reg = EnvironmentRegistry::new();
        let env_ref = committed_env(&reg, members);
        let journal: Journal = Arc::default();
        let commands = Arc::new(commands(&journal));
        let members = Arc::new(FakeMembers {
            journal: journal.clone(),
            registry: reg.clone(),
            env_ref: env_ref.clone(),
            unsettleable: unsettleable.iter().map(|m| m.to_string()).collect(),
            asked: Mutex::new(Vec::new()),
        });
        Self {
            reg,
            env_ref,
            journal,
            commands,
            members,
        }
    }

    fn use_case(&self) -> KillEnvironment {
        KillEnvironment::new(
            self.reg.clone(),
            self.members.clone(),
            self.commands.clone(),
        )
    }

    fn target(&self) -> EnvironmentTarget {
        EnvironmentTarget::Ref(self.env_ref.clone())
    }

    fn status(&self) -> EnvironmentStatus {
        self.reg.get(&self.env_ref).unwrap().status
    }

    fn journal(&self) -> Vec<String> {
        self.journal.lock().unwrap().clone()
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn members_are_asked_before_the_retained_kill_runs_exactly_once() {
    let fx = Fixture::new(&["a", "b"], &[], SpyCommands::new);
    let killed = block_on(fx.use_case().kill_container(&fx.target())).unwrap();
    assert_eq!(fx.journal(), ["members(a,b)", "kill"]);
    assert_eq!(fx.commands.kills.load(Ordering::SeqCst), 1);
    assert_eq!(fx.status(), EnvironmentStatus::Stopped);
    assert_eq!(killed.record.environment_ref, fx.env_ref);
    assert_eq!(
        killed.record.members,
        ["a", "b"],
        "the members as of the claim are reported"
    );
    assert_eq!(killed.members.settled.len(), 2);
    assert!(killed.members.all_settled());
    assert!(fx.reg.get(&fx.env_ref).unwrap().members.is_empty());
}

#[test]
fn an_unsettled_member_withholds_the_kill_and_leaves_a_retryable_state() {
    let fx = Fixture::new(&["a", "hung"], &["hung"], SpyCommands::new);
    let err = block_on(fx.use_case().kill_container(&fx.target())).unwrap_err();
    match &err {
        KillEnvironmentError::MembersUnsettled {
            environment_ref,
            members,
        } => {
            assert_eq!(environment_ref, &fx.env_ref);
            assert_eq!(members.len(), 1);
            assert_eq!(members[0].0, "hung");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.journal(), ["members(a,hung)"], "no kill ran");
    assert_eq!(fx.commands.kills.load(Ordering::SeqCst), 0);
    let record = fx.reg.get(&fx.env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::CleanupFailed);
    assert!(
        record.last_error.as_deref().unwrap().contains("hung"),
        "{record:?}"
    );
    assert_eq!(
        record.members,
        ["hung"],
        "the settled member left, the unsettled one stays for the retry"
    );
    assert!(err.to_string().contains("retry kill_container"), "{err}");
}

#[test]
fn a_retry_after_an_unsettled_member_asks_again_and_kills_once() {
    let fx = Fixture::new(&["a", "hung"], &["hung"], SpyCommands::new);
    let uc = fx.use_case();
    block_on(uc.kill_container(&fx.target())).unwrap_err();
    // The member settles this time.
    let members = Arc::new(FakeMembers {
        journal: fx.journal.clone(),
        registry: fx.reg.clone(),
        env_ref: fx.env_ref.clone(),
        unsettleable: vec![],
        asked: Mutex::new(Vec::new()),
    });
    let uc = KillEnvironment::new(fx.reg.clone(), members.clone(), fx.commands.clone());
    block_on(uc.kill_container(&fx.target())).unwrap();
    assert_eq!(
        fx.journal(),
        ["members(a,hung)", "members(hung)", "kill"],
        "only the member still recorded is asked again; one kill in total"
    );
    assert_eq!(fx.status(), EnvironmentStatus::Stopped);
}

#[test]
fn kill_failure_persists_retryable_state_and_the_retry_runs_the_kill_again() {
    let fx = Fixture::new(&["a"], &[], |journal| SpyCommands::failing(journal, 1));
    let uc = fx.use_case();
    let err = block_on(uc.kill_container(&fx.target())).unwrap_err();
    assert!(
        matches!(err, KillEnvironmentError::KillFailed { .. }),
        "{err:?}"
    );
    assert!(err.to_string().contains("kill.sh exited 1"), "{err}");
    let rec = fx.reg.get(&fx.env_ref).unwrap();
    assert_eq!(rec.status, EnvironmentStatus::CleanupFailed);
    assert_eq!(rec.last_error.as_deref(), Some("kill.sh exited 1"));
    // Retry succeeds and only then commits stopped: the kill ran once per
    // claim, twice in total.
    block_on(uc.kill_container(&fx.target())).unwrap();
    assert_eq!(fx.commands.kills.load(Ordering::SeqCst), 2);
    assert_eq!(fx.journal(), ["members(a)", "kill", "members()", "kill"]);
    assert_eq!(fx.status(), EnvironmentStatus::Stopped);
}

#[test]
fn unknown_ref_is_refused_without_asking_anyone() {
    let fx = Fixture::new(&["a"], &[], SpyCommands::new);
    let err = block_on(
        fx.use_case()
            .kill_container(&EnvironmentTarget::Ref("C9".to_string())),
    )
    .unwrap_err();
    assert!(matches!(err, KillEnvironmentError::Refused(_)), "{err:?}");
    assert!(err.to_string().contains("C9"), "{err}");
    assert!(fx.journal().is_empty());
    assert_eq!(fx.status(), EnvironmentStatus::Running);
}

#[test]
fn a_stopped_environment_is_refused_without_a_second_kill() {
    let fx = Fixture::new(&[], &[], SpyCommands::new);
    let uc = fx.use_case();
    block_on(uc.kill_container(&fx.target())).unwrap();
    let err = block_on(uc.kill_container(&fx.target())).unwrap_err();
    assert!(err.to_string().contains("stopped"), "{err}");
    assert_eq!(fx.commands.kills.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_kill_container_calls_cannot_double_kill() {
    let fx = Fixture::new(&["a"], &[], SpyCommands::new);
    let gate = Arc::new(tokio::sync::Notify::new());
    let commands = Arc::new(SpyCommands {
        gate: Some(gate.clone()),
        ..SpyCommands::new(&fx.journal)
    });
    let uc = Arc::new(KillEnvironment::new(
        fx.reg.clone(),
        fx.members.clone(),
        commands.clone(),
    ));
    let (a, b) = block_on(async {
        let ua = uc.clone();
        let ub = uc.clone();
        let ra = fx.target();
        let rb = fx.target();
        tokio::join!(ua.kill_container(&ra), async {
            // On this current-thread runtime the first caller has already
            // claimed the kill, asked its members and parked inside the
            // kill when this runs, so the second call races an IN-FLIGHT
            // kill, not a finished one.
            let second = ub.kill_container(&rb).await;
            gate.notify_one();
            second
        })
    });
    assert_eq!(commands.kills.load(Ordering::SeqCst), 1, "exactly one kill");
    assert!(a.is_ok(), "the claim holder completes its kill");
    let err = b.unwrap_err();
    assert!(
        err.to_string().contains("stale"),
        "the overlapping caller is refused while the claim is outstanding: {err}"
    );
    assert_eq!(
        fx.members.asked.lock().unwrap().len(),
        1,
        "members are asked once"
    );
    assert_eq!(fx.status(), EnvironmentStatus::Stopped);
}

/// A final-member exit that claims the environment first (its removal
/// empties a running environment) owns the end: a `kill_container`
/// arriving while that claim is outstanding is refused, and a
/// `kill_container` that claims first leaves the exiting member's removal
/// claimless.
#[test]
fn kill_container_and_a_final_member_exit_converge_on_one_claim() {
    let fx = Fixture::new(&["a"], &[], SpyCommands::new);
    // The exit claims first.
    let claim = fx.reg.remove_member(&fx.env_ref, "a").unwrap().unwrap();
    let err = block_on(fx.use_case().kill_container(&fx.target())).unwrap_err();
    assert!(err.to_string().contains("stale"), "{err}");
    assert!(
        fx.journal().is_empty(),
        "nothing ran under someone else's claim"
    );
    fx.reg.complete_kill(claim);
    assert_eq!(fx.status(), EnvironmentStatus::Stopped);

    // The kill claims first.
    let fx = Fixture::new(&["a"], &[], SpyCommands::new);
    let gate = Arc::new(tokio::sync::Notify::new());
    let commands = Arc::new(SpyCommands {
        gate: Some(gate.clone()),
        ..SpyCommands::new(&fx.journal)
    });
    let uc = KillEnvironment::new(fx.reg.clone(), fx.members.clone(), commands.clone());
    let target = fx.target();
    let (result, exit_claim) = block_on(async {
        tokio::join!(uc.kill_container(&target), async {
            // The kill has claimed and parked; the member's exit removes
            // its membership but mints no claim.
            let claim = fx.reg.remove_member(&fx.env_ref, "a").unwrap();
            gate.notify_one();
            claim
        })
    });
    assert!(result.is_ok());
    assert!(exit_claim.is_none(), "the exit found the kill's claim");
    assert_eq!(commands.kills.load(Ordering::SeqCst), 1);
    assert_eq!(fx.status(), EnvironmentStatus::Stopped);
}

#[test]
fn resolve_target_supports_names_for_control_operations() {
    let fx = Fixture::new(&[], &[], SpyCommands::new);
    let mut record = fx.reg.get(&fx.env_ref).unwrap();
    record.name = Some("review-env".to_string());
    fx.reg.commit(record);
    block_on(
        fx.use_case()
            .kill_container(&EnvironmentTarget::Name("review-env".to_string())),
    )
    .unwrap();
    assert_eq!(fx.commands.kills.load(Ordering::SeqCst), 1);
    assert_eq!(fx.status(), EnvironmentStatus::Stopped);
}

#[test]
fn kill_container_without_retained_kill_argv_fails_before_any_claim() {
    let fx = Fixture::new(&["agent-a"], &[], SpyCommands::new);
    let mut record = fx.reg.get(&fx.env_ref).unwrap();
    record.retained_kill_argv = vec![];
    record.retained_cleanup_argv = vec!["cleanup.sh".to_string()];
    fx.reg.commit(record);

    let err = block_on(fx.use_case().kill_container(&fx.target())).unwrap_err();
    assert!(err.to_string().contains("no retained kill argv"), "{err}");
    assert!(fx.journal().is_empty(), "no member was asked");

    // No state transition: still Running with its member, so joins keep
    // working and the final-member exit cleanup fallback stays reachable.
    let record = fx.reg.get(&fx.env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Running);
    assert_eq!(record.members, vec!["agent-a".to_string()]);
    assert!(
        fx.reg.begin_kill(&fx.env_ref).is_ok(),
        "no claim was left behind"
    );
}
