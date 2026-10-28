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
