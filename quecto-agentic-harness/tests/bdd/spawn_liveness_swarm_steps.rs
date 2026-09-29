//! #1924: a lost coordinator retains a resumable swarm's environment, and
//! an orderly end is recorded on the container listing.
use super::*;

/// A real coordination store with a running run (coordinator "coordinator")
/// under the scenario's base path, advertised to the supervising session
/// through the create result's `metadata.checkout` exactly as the Docker
/// adapter reports it.
#[given("the next created environment hosts a running swarm run coordinated by its member")]
fn given_next_environment_hosts_running_swarm(world: &mut QuectoWorld) {
    host_swarm_run(world, None);
}

/// Same, but the coordinator has already ended the run holding `outcome`.
#[given(
    expr = "the next created environment hosts a swarm run its coordinator ended holding {string}"
)]
fn given_next_environment_hosts_ended_swarm(world: &mut QuectoWorld, outcome: String) {
    host_swarm_run(world, Some(outcome));
}

fn host_swarm_run(world: &mut QuectoWorld, ended_holding: Option<String>) {
    let checkout = base_path(world).join("swarm-checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    let workspace = Arc::new(checkout.clone());
    let sandbox = Arc::new(quecto::infrastructure::security::sandbox::Sandbox::new(
        Some(checkout.clone()),
    ));
    // Building the test-support tool creates the running run in the store.
    let tool = quecto::infrastructure::tools::swarm_test_support::tool(
        workspace,
        sandbox,
        quecto::infrastructure::tools::swarm::SwarmConfig::default(),
        quecto::composition::swarm::swarm_board(),
    );
    if let Some(outcome) = ended_holding {
        use quecto::application::tools::ports::Tool;
        let args = serde_json::json!({"op":"stop","status":outcome,
            "reason":"ended by the coordinator"})
        .to_string();
        let result = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { tool.execute(&args).await })
            .expect("swarm tool executes");
        assert!(!result.is_error, "{}", result.content);
    }
    world.swarm_workspace = Some(checkout.clone());

    let create = base_path(world).join("env-create-live.sh");
    let script = std::fs::read_to_string(&create).expect("liveness create fixture exists");
    let advertised = format!(r#""metadata":{{"checkout":"{}"}}"#, checkout.display());
    assert!(
        script.contains(r#""metadata":{}"#) && script.contains(r#" "$env_id" "$ws" "#),
        "create fixture reports empty metadata and a per-create workspace before patching"
    );
    // Like a sandbox config, the members' checkout IS the workspace: the
    // host only opens a store under the environment's own workspace.
    let workspace = format!("\"{}\"", checkout.display());
    write_executable(
        &create,
        script.replace(r#""metadata":{}"#, &advertised).replace(
            r#" "$env_id" "$ws" "#,
            &format!(" \"$env_id\" {workspace} "),
        ),
    );
}

fn hosted_swarm_context(
    world: &QuectoWorld,
) -> quecto::infrastructure::tools::swarm_bridge::SwarmContext {
    quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: world
            .swarm_workspace
            .clone()
            .expect("a hosted swarm run was set up"),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    }
}

#[then(
    expr = "the hosted swarm run is paused holding {string} with a resume blocker naming its coordinator"
)]
fn then_hosted_run_paused_with_lost_coordinator(world: &mut QuectoWorld, outcome: String) {
    let context = hosted_swarm_context(world);
    let summary = context.summary().expect("the store survives the loss");
    assert_eq!(summary["status"], "paused", "{summary}");
    assert_eq!(summary["outcome"], outcome, "{summary}");
    let receipt = context.control_status().unwrap();
    let blockers = receipt["resume_blockers"]
        .as_array()
        .expect("resume blockers are reported");
    assert!(
        blockers
            .iter()
            .any(|b| b.as_str().unwrap_or_default().contains("'coordinator'")),
        "resume blockers must name the lost coordinator: {receipt}"
    );
}

#[then(expr = "the container listing entry {string} should explain why it was retained")]
fn then_listing_entry_explains_retention(world: &mut QuectoWorld, env_ref: String) {
    let entry = container_listing_entry(world, &env_ref);
    let reason = entry["metadata"]["retained"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        reason.contains("coordinator") && reason.contains("kill_container"),
        "retention reason should name the lost coordinator and the way to close: {entry}"
    );
}

#[then(expr = "the hosted swarm run is still paused holding {string} with no resume blockers")]
fn then_hosted_run_untouched(world: &mut QuectoWorld, outcome: String) {
    let context = hosted_swarm_context(world);
    let summary = context.summary().expect("the store survives");
    assert_eq!(summary["status"], "paused", "{summary}");
    assert_eq!(summary["outcome"], outcome, "{summary}");
    let receipt = context.control_status().unwrap();
    assert_eq!(
        receipt["resume_blockers"],
        serde_json::json!([]),
        "{receipt}"
    );
}

#[then(expr = "the container listing entry {string} should record an orderly end holding {string}")]
fn then_listing_entry_orderly_end(world: &mut QuectoWorld, env_ref: String, outcome: String) {
    let entry = container_listing_entry(world, &env_ref);
    let reason = entry["metadata"]["retained"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        reason.starts_with(&format!("run ended: {outcome}")) && reason.contains("kill_container"),
        "retention reason should record the orderly end: {entry}"
    );
}
