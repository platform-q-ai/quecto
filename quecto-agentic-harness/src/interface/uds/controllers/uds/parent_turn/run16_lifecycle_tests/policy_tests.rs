//! Effective Replace/reload and internal-boundary authority regressions.
use super::*;

#[tokio::test]
async fn run16_m2_replace_removal_stands_down_without_correction_or_stale_guidance() {
    if isolated(concat!(
        module_path!(),
        "::run16_m2_replace_removal_stands_down_without_correction_or_stale_guidance"
    )) {
        let mut rig = Rig::new(vec![
            Reply::Text("Policy disabled."),
            Reply::Text("Must not correct."),
        ]);
        rig.policy(false, false).await;
        assert!(
            !rig.env.agent.is_tool_model_visible("declare_turn_outcome"),
            "actual Replace is effective"
        );
        rig.frames();
        rig.prompt("disabled").await;
        let frames = rig.frames();
        let requests = rig.provider.requests();
        assert_eq!(
            requests.len(),
            1,
            "disabled declaration authority must not correct/stall"
        );
        assert!(
            !requests[0]
                .tools
                .iter()
                .any(|name| name == "declare_turn_outcome")
        );
        assert!(
            !requests[0].guidance,
            "removed capability must not advertise active guard"
        );
        assert!(of_type(&frames, "turn_admitted").is_empty());
        assert!(of_type(&frames, "turn_outcome").is_empty());
    }
}

#[tokio::test]
async fn run16_m2_replace_revokes_captured_identity_and_active_turn_across_restore() {
    if isolated(concat!(
        module_path!(),
        "::run16_m2_replace_revokes_captured_identity_and_active_turn_across_restore"
    )) {
        let mut rig = Rig::new(vec![
            Reply::Text("No declaration."),
            Reply::Text("No declaration."),
        ]);
        let runtime = rig
            .runtime
            .clone()
            .expect("production authorized candidate");
        let active = runtime
            .admit(
                "cli:test",
                TurnAdmission::External {
                    episode_id: "capture".into(),
                },
                "captured-turn",
            )
            .unwrap();
        let old = runtime.capture().unwrap();
        runtime
            .declare(
                LaunchAuthority::TopLevelParent,
                &old,
                TurnIntent::Done,
                "Before removal",
            )
            .unwrap();
        rig.policy(false, false).await;
        assert!(!rig.env.agent.is_tool_model_visible("declare_turn_outcome"));
        assert!(
            runtime.capture().is_none(),
            "effective removal immediately revokes declaration capture"
        );
        assert!(
            runtime
                .declare(
                    LaunchAuthority::TopLevelParent,
                    &old,
                    TurnIntent::Done,
                    "Stale"
                )
                .is_err()
        );
        rig.policy(true, false).await;
        assert!(rig.env.agent.is_tool_model_visible("declare_turn_outcome"));
        assert!(
            runtime.capture().is_none(),
            "visibility restore does not itself enter an outer turn"
        );
        assert!(
            runtime
                .declare(
                    LaunchAuthority::TopLevelParent,
                    &old,
                    TurnIntent::Done,
                    "Still stale"
                )
                .is_err()
        );
        assert!(matches!(
            runtime.complete(active, TurnEnd::Completed(WorkLiveness::Empty)),
            ParentVerdict::Unsettled
        ));
        rig.frames();
        rig.prompt("restored").await;
        let frames = rig.frames();
        assert_eq!(
            rig.provider.requests().len(),
            2,
            "next outer boundary re-arms with one correction"
        );
        assert_ne!(
            rig.provider.requests()[0]
                .identity
                .as_ref()
                .unwrap()
                .outcome_epoch,
            old.outcome_epoch
        );
        assert_eq!(of_type(&frames, "turn_outcome")[0]["outcome"], "stalled");
    }
}

#[tokio::test]
async fn run16_m2_internal_queued_disable_restore_never_rearms_current_tool_turn() {
    if isolated(concat!(
        module_path!(),
        "::run16_m2_internal_queued_disable_restore_never_rearms_current_tool_turn"
    )) {
        let mut replies = done();
        replies.extend([Reply::Text("Next outer."), Reply::Text("No declaration.")]);
        let mut rig = Rig::new(replies);
        rig.env
            .agent
            .queue_tool_policy_request(ToolPolicyRequest::patch(vec![
                ToolPolicyMutation::disable("declare_turn_outcome", "internal remove"),
            ]));
        rig.env
            .agent
            .queue_tool_policy_request(ToolPolicyRequest::patch(vec![ToolPolicyMutation::enable(
                "declare_turn_outcome",
                "internal restore",
            )]));
        assert!(
            rig.env.agent.is_tool_model_visible("declare_turn_outcome"),
            "queued-unapplied is not effective yet"
        );
        rig.prompt("internal").await;
        let requests = rig.provider.requests();
        assert_eq!(
            requests.len(),
            2,
            "actual declaration/tool result continuation"
        );
        assert!(
            requests[0].identity.is_some(),
            "first provider request is guarded"
        );
        assert!(
            requests[1]
                .tools
                .iter()
                .any(|name| name == "declare_turn_outcome"),
            "actual internal drain restored visibility"
        );
        assert!(
            requests[1].identity.is_none(),
            "internal restore must not re-arm in-flight capture"
        );
        let old = requests[0].identity.as_ref().unwrap();
        let runtime = rig.runtime.as_ref().unwrap();
        assert!(
            runtime
                .declare(
                    LaunchAuthority::TopLevelParent,
                    old,
                    TurnIntent::Done,
                    "Stale queued invocation"
                )
                .is_err()
        );
        assert!(
            of_type(&rig.frames(), "turn_outcome").is_empty(),
            "revoked old declaration cannot publish done"
        );
        rig.prompt("outer-after-restore").await;
        let next = rig.provider.requests();
        assert_ne!(
            next[2].identity.as_ref().unwrap().outcome_epoch,
            old.outcome_epoch
        );
        assert_eq!(next[3].last_user, CORRECTION_NOTE);
        assert_eq!(
            of_type(&rig.frames(), "turn_outcome")[0]["outcome"],
            "stalled"
        );
    }
}

// Provider injection only: the file source still reads the actual temporary
// Config and applies its persisted baseline through the real reload use case.
static RELOAD_PROVIDER: OnceLock<Arc<Probe>> = OnceLock::new();
fn reload_provider(
    _: &Config,
    _: &std::path::Path,
    _: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    RELOAD_PROVIDER
        .get()
        .cloned()
        .map(|provider| provider as Arc<dyn LlmProvider>)
        .ok_or_else(|| "reload test provider not installed".into())
}

fn install_reload(rig: &mut Rig, enabled: bool) -> std::path::PathBuf {
    use crate::infrastructure::config::runtime_configuration::FileRuntimeConfiguration;
    let path = rig._home.path().join("lifecycle-config.json");
    write_reload_config(rig, &path, enabled);
    let load_path = path.clone();
    RELOAD_PROVIDER
        .set(rig.provider.clone())
        .expect("one provider per isolated probe");
    let source = FileRuntimeConfiguration::seeded(
        vec![crate::infrastructure::config::reload::ReloadSource::new(
            path.clone(),
        )],
        rig._home.path().into(),
        Arc::new(move || {
            let text = std::fs::read_to_string(&load_path).map_err(|error| error.to_string())?;
            serde_json::from_str(&text).map_err(|error| error.to_string())
        }),
        reqwest::Client::new(),
        reload_provider,
    );
    rig.reload = Some(Arc::new(
        crate::application::catalogue::use_cases::ReloadRuntimeConfiguration::new(Box::new(source)),
    ));
    path
}
fn declaration_id(rig: &Rig) -> String {
    rig.env
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "declare_turn_outcome")
        .expect("production declaration catalogue entry")
        .stable_id
        .into_owned()
}
fn write_reload_config(rig: &Rig, path: &std::path::Path, enabled: bool) {
    let mut config = Config::default();
    config.tools.policy.entries.insert(
        declaration_id(rig),
        ToolPolicyEntryConfig {
            scope: if enabled {
                ProfileAvailabilityScope::Both
            } else {
                ProfileAvailabilityScope::None
            },
        },
    );
    std::fs::write(path, serde_json::to_string(&config).unwrap()).unwrap();
}

#[tokio::test]
async fn run16_m2_real_file_reload_removal_revokes_guard() {
    if isolated(concat!(
        module_path!(),
        "::run16_m2_real_file_reload_removal_revokes_guard"
    )) {
        let mut rig = Rig::new(vec![
            Reply::Text("Reload disabled."),
            Reply::Text("No correction expected."),
        ]);
        install_reload(&mut rig, false);
        rig.send(r#"{"type":"reload","id":"remove"}"#).await;
        assert!(
            !rig.env.agent.is_tool_model_visible("declare_turn_outcome"),
            "real file reload must apply persisted None"
        );
        rig.frames();
        rig.prompt("after-reload").await;
        assert_eq!(
            rig.provider.requests().len(),
            1,
            "reload removal must stand down, not correct/stall"
        );
        assert!(!rig.provider.requests()[0].guidance);
        assert!(of_type(&rig.frames(), "turn_outcome").is_empty());
    }
}

#[tokio::test]
async fn run16_m2_startup_withheld_candidate_restores_on_next_outer_reload() {
    if isolated(concat!(
        module_path!(),
        "::run16_m2_startup_withheld_candidate_restores_on_next_outer_reload"
    )) {
        let active = Rig::new(vec![]);
        let id = declaration_id(&active);
        drop(active);
        let mut config = Config::default();
        config.tools.policy.entries.insert(
            id,
            ToolPolicyEntryConfig {
                scope: ProfileAvailabilityScope::None,
            },
        );
        let composed = compose_with_config(true, |_| {}, config);
        assert!(
            !composed.snippets.contains(DECLARE_GUIDANCE),
            "startup dormant must not advertise active guard"
        );
        let mut rig = Rig::composed(
            composed,
            vec![
                Reply::Text("No declaration."),
                Reply::Text("No declaration."),
            ],
        );
        rig.snippets = crate::interface::cli::controllers::shared::build_agent_system_prompt(
            None,
            None,
            false,
            &rig.snippets,
        );
        crate::domain::conversation::services::conversation_view::inject_system_prompt(
            &mut rig.env.messages,
            &rig.snippets,
        );
        install_reload(&mut rig, true);
        rig.send(r#"{"type":"reload","id":"restore"}"#).await;
        assert!(
            rig.env.agent.is_tool_model_visible("declare_turn_outcome"),
            "real reload restored production-bound declaration tool"
        );
        rig.frames();
        rig.prompt("restored").await;
        let frames = rig.frames();
        assert_eq!(
            rig.provider.requests().len(),
            2,
            "authorized startup-withheld root re-arms at next OUTER boundary"
        );
        assert_eq!(of_type(&frames, "turn_admitted").len(), 2);
        assert_eq!(of_type(&frames, "turn_outcome")[0]["outcome"], "stalled");
        assert!(
            rig.provider.requests()[0].guidance,
            "restored active guard must advertise declaration guidance"
        );
    }
}

#[tokio::test]
async fn run16_m2_role_excluded_policy_restore_cannot_create_authority() {
    if isolated(concat!(
        module_path!(),
        "::run16_m2_role_excluded_policy_restore_cannot_create_authority"
    )) {
        for role in ["child", "coordinator", "participant", "legacy"] {
            let composed = compose_with(true, |flags| match role {
                "child" => flags.spawned = true,
                "coordinator" => flags.coordinator = true,
                "participant" => {
                    flags.swarm_participation =
                        crate::infrastructure::tools::swarm::swarm_bridge::Participation::Fixed(
                            true,
                        )
                }
                "legacy" => flags.parent_outcome = None,
                _ => unreachable!(),
            });
            let mut rig = Rig::composed(composed, vec![Reply::Text("Uncomposed.")]);
            rig.policy(true, false).await;
            rig.frames();
            rig.prompt(role).await;
            let frames = rig.frames();
            assert_eq!(
                rig.provider.requests().len(),
                1,
                "{role}: no guard/correction"
            );
            assert!(!rig.provider.requests()[0].guidance, "{role}");
            assert!(
                !rig.env.agent.is_tool_model_visible("declare_turn_outcome"),
                "{role}: visibility cannot mint authority"
            );
            assert!(of_type(&frames, "turn_admitted").is_empty(), "{role}");
            assert!(of_type(&frames, "turn_outcome").is_empty(), "{role}");
        }
    }
}

// Run17 preservation supplement: failed/no-op effective changes must not
// revoke an existing declaration handle or suppress the next guarded turn.
#[derive(Clone, Copy, Debug)]
enum Preserve {
    Noop,
    PersistenceFailure,
    ReloadFailure,
}

async fn preserves_guard(case: Preserve) {
    use crate::domain::tool_policy::value_objects::tool::{
        ToolPolicyApplyMode, ToolPolicyMutationStatus,
    };
    let mut rig = Rig::new(done());
    // DispatchTestEnv deliberately starts without a system message. Match
    // multi_client_loop startup before asserting retained production guidance.
    crate::domain::conversation::services::conversation_view::inject_system_prompt(
        &mut rig.env.messages,
        &rig.snippets,
    );
    let runtime = rig.runtime.clone().expect("production guarded root");
    let active = runtime
        .admit(
            "cli:test",
            TurnAdmission::External {
                episode_id: "preserve".into(),
            },
            "preserve-turn",
        )
        .unwrap();
    let before = runtime.capture().unwrap();
    let operation_valid = match case {
        Preserve::Noop => {
            rig.send(r#"{"type":"set_tool_policy","id":"noop","operation":"patch","mutations":[],"mode":"immediateIfIdle","persist":false}"#).await;
            let frames = rig.frames();
            frames
                .iter()
                .any(|f| f["type"] == "response" && f["id"] == "noop" && f["success"] == true)
        }
        Preserve::PersistenceFailure => {
            rig.env
                .agent
                .set_tool_policy_persistence(Some(Arc::new(|_| {
                    Err("run17 deterministic write failure".into())
                })));
            let mut request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::disable(
                "declare_turn_outcome",
                "attempted durable revoke",
            )]);
            request.persist = true;
            let reconciliation = rig
                .env
                .agent
                .request_tool_policy(request, ToolPolicyApplyMode::ImmediateIfIdle)
                .expect("real idle policy port result");
            reconciliation.results.len() == 1
                && reconciliation.results[0].status == ToolPolicyMutationStatus::PersistenceFailed
        }
        Preserve::ReloadFailure => {
            let path = install_reload(&mut rig, true);
            std::fs::write(path, "{ invalid temporary config").unwrap();
            rig.send(r#"{"type":"reload","id":"failed-reload"}"#).await;
            let frames = rig.frames();
            frames.iter().any(|f| {
                f["type"] == "response" && f["id"] == "failed-reload" && f["success"] == false
            })
        }
    };
    let after = runtime.capture();
    let visible = rig.env.agent.is_tool_model_visible("declare_turn_outcome");
    let declaration = runtime.declare(
        LaunchAuthority::TopLevelParent,
        &before,
        TurnIntent::Done,
        "Preserved declaration authority",
    );
    let verdict = runtime.complete(active, TurnEnd::Completed(WorkLiveness::Empty));
    rig.frames();
    rig.prompt("next-guarded").await;
    let requests = rig.provider.requests();
    let frames = rig.frames();
    // No services/loops in this rig. Close file source/registry scratch before
    // making any preservation assertions, including an unexpectedly failed op.
    drop(rig);
    println!(
        "RUN17 policy teardown complete case={case:?} operation_valid={operation_valid} requests={}",
        requests.len()
    );
    assert!(
        operation_valid,
        "actual no-op/failure result must be observed"
    );
    assert!(visible, "last-good declaration visibility retained");
    assert_eq!(
        after.as_ref(),
        Some(&before),
        "unchanged effective policy retains exact epoch/nonce capture"
    );
    assert!(
        declaration.is_ok(),
        "old in-flight bound handle remains authoritative"
    );
    assert!(
        matches!(verdict, ParentVerdict::Published(_)),
        "preserved declaration settles without correction: {verdict:?}"
    );
    assert_eq!(
        requests.len(),
        2,
        "next outer turn declares then finishes, without correction"
    );
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|name| name == "declare_turn_outcome")
    );
    assert!(requests[0].guidance, "last-good guard guidance preserved");
    assert!(requests[0].identity.is_some());
    assert_eq!(of_type(&frames, "turn_admitted").len(), 1);
    assert_eq!(of_type(&frames, "turn_outcome").len(), 1);
    assert_eq!(of_type(&frames, "turn_outcome")[0]["outcome"], "done");
}
#[tokio::test]
async fn run17_m2_noop_preserves_effective_guard() {
    if isolated(concat!(
        module_path!(),
        "::run17_m2_noop_preserves_effective_guard"
    )) {
        preserves_guard(Preserve::Noop).await;
    }
}
#[tokio::test]
async fn run17_m2_failed_persistence_preserves_effective_guard() {
    if isolated(concat!(
        module_path!(),
        "::run17_m2_failed_persistence_preserves_effective_guard"
    )) {
        preserves_guard(Preserve::PersistenceFailure).await;
    }
}
#[tokio::test]
async fn run17_m2_failed_reload_preserves_effective_guard() {
    if isolated(concat!(
        module_path!(),
        "::run17_m2_failed_reload_preserves_effective_guard"
    )) {
        preserves_guard(Preserve::ReloadFailure).await;
    }
}

#[tokio::test]
async fn run21_first_post_restore_validates_id_before_execution() {
    if !isolated(concat!(
        module_path!(),
        "::run21_first_post_restore_validates_id_before_execution"
    )) {
        return;
    }
    let mut rig = Rig::new(done());
    rig.policy(false, false).await;
    rig.policy(true, false).await;
    assert!(
        rig.env.session.parent_outcome().is_none(),
        "restore must await next outer boundary"
    );
    rig.frames();
    rig.prompt(&"x".repeat(513)).await;
    let refused = rig.frames();
    assert_eq!(
        rig.provider.requests().len(),
        0,
        "FIRST post-restore invalid ID is refused before execution"
    );
    assert!(
        refused
            .iter()
            .any(|f| f["type"] == "response" && f["success"] == false)
    );
    rig.prompt("valid-next").await;
    let accepted = rig.frames();
    assert_eq!(rig.provider.requests().len(), 2);
    assert_eq!(
        of_type(&accepted, "turn_admitted")[0]["episodeId"],
        "valid-next"
    );
    assert_eq!(of_type(&accepted, "turn_outcome")[0]["outcome"], "done");
}

#[tokio::test]
async fn run21_first_post_restore_boundary_holds_busy_before_dispatch() {
    if !isolated(concat!(
        module_path!(),
        "::run21_first_post_restore_boundary_holds_busy_before_dispatch"
    )) {
        return;
    }
    let mut rig = Rig::new(done());
    rig.policy(false, false).await;
    rig.policy(true, false).await;
    let mut ctx = rig.env.ctx();
    let id = super::super::episode_id(&mut ctx, Some("valid-first".into()), "prompt")
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("valid-first"));
    let held = super::super::busy_across_turns(&ctx);
    assert!(
        held.is_some(),
        "FIRST post-restore admission re-arms BEFORE busy_across_turns, not inside execution"
    );
    drop(held);
}

// Mixed persisted/runtime states are independent transaction inputs.
#[tokio::test]
async fn run26_mixed_policy_failed_persistence_restores_runtime_snapshot() {
    if isolated(concat!(
        module_path!(),
        "::run26_mixed_policy_failed_persistence_restores_runtime_snapshot"
    )) {
        mixed_repair(false, true).await;
    }
}

#[tokio::test]
async fn run26_mixed_policy_queued_failure_restores_runtime_snapshot() {
    if isolated(concat!(
        module_path!(),
        "::run26_mixed_policy_queued_failure_restores_runtime_snapshot"
    )) {
        mixed_repair(true, true).await;
    }
}

#[tokio::test]
async fn run26_mixed_policy_success_and_noop_controls() {
    if isolated(concat!(
        module_path!(),
        "::run26_mixed_policy_success_and_noop_controls"
    )) {
        mixed_repair(false, false).await;
    }
}

#[tokio::test]
async fn run26_mixed_policy_queued_success_and_noop_controls() {
    if isolated(concat!(
        module_path!(),
        "::run26_mixed_policy_queued_success_and_noop_controls"
    )) {
        mixed_repair(true, false).await;
    }
}

async fn mixed_repair(queued: bool, fail: bool) {
    use crate::domain::tool_policy::value_objects::tool::{
        ToolPolicyApplyMode, ToolPolicyMutationStatus,
    };
    let mut rig = Rig::new(vec![]);
    rig.policy(false, false).await;
    install_reload(&mut rig, true);
    rig.send(r#"{"type":"reload","id":"mixed-restore"}"#).await;
    let entry = |rig: &Rig| {
        rig.env
            .agent
            .tool_catalogue_entries()
            .into_iter()
            .find(|e| e.name == "declare_turn_outcome")
            .unwrap()
    };
    let before = entry(&rig);
    assert_eq!(before.profile_scope, Some(ProfileAvailabilityScope::Both));
    assert!(!before.runtime_availability.is_enabled());
    assert!(!rig.env.agent.is_tool_model_visible("declare_turn_outcome"));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    rig.env
        .agent
        .set_tool_policy_persistence(Some(Arc::new(move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fail {
                Err("run26 injected persistence-port failure".into())
            } else {
                Ok(())
            }
        })));
    let mut request = ToolPolicyRequest::replace(Vec::new(), ProfileAvailabilityScope::Both);
    request.persist = true;
    let mode = if queued {
        ToolPolicyApplyMode::AtNextTurnBoundary
    } else {
        ToolPolicyApplyMode::ImmediateIfIdle
    };
    let immediate = rig.env.agent.request_tool_policy(request.clone(), mode);
    let reconciliation = if queued {
        assert!(immediate.is_none());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(entry(&rig), before, "queued is not applied before boundary");
        rig.env
            .agent
            .drain_tool_policy_mutations_at_boundary()
            .unwrap()
    } else {
        immediate.unwrap()
    };
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        rig.env
            .agent
            .drain_tool_policy_mutations_at_boundary()
            .is_none(),
        "drains exactly once"
    );
    let result = reconciliation
        .results
        .iter()
        .find(|r| r.name == "declare_turn_outcome")
        .unwrap();
    let after = entry(&rig);
    println!(
        "RUN26_MIXED_ROLLBACK queued={queued} fail={fail} before={before:?} after={after:?} status={:?}",
        result.status
    );
    if fail {
        assert_eq!(result.status, ToolPolicyMutationStatus::PersistenceFailed);
        assert_eq!(
            after, before,
            "failed durable repair must restore actual independent runtime/profile state"
        );
        assert!(!rig.env.agent.is_tool_model_visible("declare_turn_outcome"));
    } else {
        assert_eq!(result.status, ToolPolicyMutationStatus::Applied);
        assert!(after.runtime_availability.is_enabled());
        assert_eq!(after.profile_scope, before.profile_scope);
        assert!(rig.env.agent.is_tool_model_visible("declare_turn_outcome"));
        let noop = rig
            .env
            .agent
            .request_tool_policy(request, ToolPolicyApplyMode::ImmediateIfIdle)
            .unwrap();
        let noop = noop
            .results
            .iter()
            .find(|r| r.name == "declare_turn_outcome")
            .unwrap();
        assert_eq!(noop.status, ToolPolicyMutationStatus::AlreadyInState);
        assert_eq!(entry(&rig), after, "equal replacement is an exact no-op");
    }
}
