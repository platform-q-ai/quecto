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

/// A container store at its bootstrap placeholder, created by its
/// coordinator: what every container carries before anyone creates a run.
fn bootstrapped(checkout: &std::path::Path) -> SwarmContext {
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let coordinator = member(checkout, "coordinator");
    coordinator.join(&identity(), None, None).unwrap();
    coordinator
}

fn member(checkout: &std::path::Path, name: &str) -> SwarmContext {
    SwarmContext {
        checkout: checkout.to_path_buf(),
        member: name.into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    }
}

fn identity() -> crate::domain::swarm::ProcessIdentity {
    crate::domain::swarm::ProcessIdentity {
        pid: std::process::id(),
        started: swarm_bridge::process_start(std::process::id()).unwrap(),
    }
}

fn flags_with(args: &[&str]) -> AgentFlags {
    let mut all = vec!["--mode".to_string(), "uds".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    super::super::parse_agent_flags(&all, &mut String::new()).unwrap()
}

fn create_run(context: &SwarmContext) {
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
}

/// #1715: a newcomer joining an ordinary container (bootstrap placeholder
/// run) keeps its workflow; joining a container whose run has been created
/// is refused with a workflow request and joins workflow-disabled without.
/// The newcomer is not a member before the join, as a host-side join is not.
#[test]
fn startup_workflow_follows_swarm_participation_not_containerization() {
    let ordinary = tempfile::tempdir().unwrap();
    bootstrapped(ordinary.path());
    let mut flags = flags_with(&["--workflow"]);
    let mut stderr = String::new();
    assert!(
        admit_with(
            Some(member(ordinary.path(), "newcomer")),
            false,
            &mut flags,
            &mut stderr
        ),
        "{stderr}"
    );
    assert!(
        !flags.workflow_disabled,
        "ordinary container keeps workflow"
    );
    assert!(flags.workflow);
    assert!(!flags.swarm_participation.participating());

    let swarm = tempfile::tempdir().unwrap();
    create_run(&bootstrapped(swarm.path()));
    let mut flags = flags_with(&["--workflow"]);
    let mut stderr = String::new();
    assert!(!admit_with(
        Some(member(swarm.path(), "late-with-workflow")),
        false,
        &mut flags,
        &mut stderr
    ));
    assert!(
        stderr.contains("swarm admission rejected")
            && stderr.contains("workflow is unavailable for swarm agents"),
        "{stderr}"
    );
    let mut flags = flags_with(&[]);
    let mut stderr = String::new();
    assert!(
        admit_with(
            Some(member(swarm.path(), "late")),
            false,
            &mut flags,
            &mut stderr
        ),
        "{stderr}"
    );
    assert!(flags.workflow_disabled);
    assert!(flags.swarm_participation.participating());
    // No container context at all: nothing changes.
    let mut flags = flags_with(&["--workflow"]);
    assert!(admit_with(None, false, &mut flags, &mut String::new()));
    assert!(!flags.workflow_disabled);
}

/// Outside any container there is no swarm context: admission is a no-op.
#[test]
fn admit_without_a_container_context_changes_nothing() {
    let mut flags = flags_with(&["--workflow"]);
    let mut stderr = String::new();
    assert!(admit(&mut flags, &mut stderr), "{stderr}");
    assert!(!flags.workflow_disabled && !flags.swarm_participation.participating());
}

/// #2145: the creator claims the store's place in the checkout's git
/// directory before anything reads the store, and its join creates the store
/// there; a later member neither claims nor, without a store, joins.
#[test]
fn the_creator_claims_the_store_location_before_it_is_read() {
    let checkout = tempfile::tempdir().unwrap();
    std::fs::create_dir(checkout.path().join(".git")).unwrap();
    let mut flags = flags_with(&[]);
    let mut stderr = String::new();
    assert!(!admit_with(
        Some(member(checkout.path(), "late")),
        false,
        &mut flags,
        &mut stderr,
    ));
    assert!(stderr.contains("store missing"), "{stderr}");
    assert!(!checkout.path().join(".git/quecto").exists());
    let creator = member(checkout.path(), "creator");
    let mut stderr = String::new();
    assert!(
        admit_with(Some(creator.clone()), true, &mut flags, &mut stderr),
        "{stderr}"
    );
    assert_eq!(
        creator.database(),
        checkout.path().join(".git/quecto/swarm.sqlite")
    );
    assert!(creator.database().is_file());
}
