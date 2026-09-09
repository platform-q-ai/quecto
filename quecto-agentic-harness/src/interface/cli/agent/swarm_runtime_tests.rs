use super::*;

#[test]
fn swarm_cli_disables_default_workflow_and_rejects_explicit_activation() {
    for flag in [
        None,
        Some("--workflow"),
        Some("--workflow-guards"),
        Some("--workflow-spec"),
    ] {
        let mut args = vec!["--mode".to_string(), "uds".to_string()];
        if let Some(flag) = flag {
            args.push(flag.into());
            if flag == "--workflow-spec" {
                args.push("/unread/spec.json".into());
            }
        }
        let mut flags = super::super::parse_agent_flags(&args, &mut String::new()).unwrap();
        let result = disable_workflow(&mut flags);
        if flag.is_some() {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("workflow is unavailable for swarm agents")
            );
        } else {
            result.unwrap();
            assert!(flags.workflow_disabled);
            assert!(!flags.workflow_guards);
        }
    }
}

/// A container store at its bootstrap placeholder: what every container
/// carries before anyone creates a run.
fn store_context(checkout: &std::path::Path) -> SwarmContext {
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let context = SwarmContext {
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    context.join(&identity(), None, None).unwrap();
    context
}

fn identity() -> crate::domain::swarm::ProcessIdentity {
    crate::domain::swarm::ProcessIdentity {
        pid: std::process::id(),
        started: swarm_bridge::process_start(std::process::id()).unwrap(),
    }
}

/// #1715: an ordinary container (bootstrap placeholder run) keeps its
/// workflow; once the run has been created the same startup is refused.
#[test]
fn startup_workflow_follows_swarm_participation_not_containerization() {
    let tmp = tempfile::tempdir().unwrap();
    let context = store_context(tmp.path());
    let mut flags = super::super::parse_agent_flags(
        &["--mode".into(), "uds".into(), "--workflow".into()],
        &mut String::new(),
    )
    .unwrap();
    let mut stderr = String::new();
    assert!(
        admit_with(Some(context.clone()), &mut flags, &mut stderr),
        "{stderr}"
    );
    assert!(
        !flags.workflow_disabled,
        "ordinary container keeps workflow"
    );
    assert!(flags.workflow);
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    context
        .create_run(
            &serde_json::json!({"goal":"g","constraints":[],
                "criteria":[{"id":"t","kind":"command","description":"pass"}],
                "member_limit":3,"deadline":deadline}),
            &identity(),
            None,
        )
        .unwrap();
    let mut flags = super::super::parse_agent_flags(
        &["--mode".into(), "uds".into(), "--workflow".into()],
        &mut String::new(),
    )
    .unwrap();
    let mut stderr = String::new();
    assert!(!admit_with(Some(context.clone()), &mut flags, &mut stderr));
    assert!(
        stderr.contains("swarm admission rejected")
            && stderr.contains("workflow is unavailable for swarm agents"),
        "{stderr}"
    );
    // Without a workflow request the join into the swarm proceeds, workflow
    // disabled, and the process is a participant.
    let mut flags =
        super::super::parse_agent_flags(&["--mode".into(), "uds".into()], &mut String::new())
            .unwrap();
    let mut stderr = String::new();
    assert!(
        admit_with(Some(context), &mut flags, &mut stderr),
        "{stderr}"
    );
    assert!(flags.workflow_disabled);
    // No container context at all: nothing changes.
    let mut flags = super::super::parse_agent_flags(
        &["--mode".into(), "uds".into(), "--workflow".into()],
        &mut String::new(),
    )
    .unwrap();
    assert!(admit_with(None, &mut flags, &mut String::new()));
    assert!(!flags.workflow_disabled);
}
