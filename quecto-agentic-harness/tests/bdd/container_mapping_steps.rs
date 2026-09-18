//! Repo → container mapping (#2024 S4a): `spawn container: true` selects
//! the container config the launching agent's checkout binds through its
//! trusted `.quecto/config.json` overlay, resolved by the configuration
//! capability (one overlay, one trust record) and reached by the launch
//! policy through its own port. The rig is the real `SpawnTool`, composed
//! for a checkout the way the agent build composes it for its working
//! directory, with fake container scripts that record the argv they got.
use super::*;

use crate::spawn_tool_steps::{execute_spawn_json_without_config, given_script_spawn};

/// The checkout the launching agent works in: the world's hermetic cwd,
/// which `quecto config set --local` writes the overlay into.
fn checkout(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .cwd
        .clone()
        .expect("BDD world should pin a hermetic cwd")
}

/// The fake create/cleanup scripts the global default entry points at, so
/// an overlay entry can reuse them with its own `--repo` argv.
fn fake_scripts(world: &QuectoWorld) -> (String, String) {
    let config_path = world.config_path.clone().expect("config path");
    let global: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
    let entry = &global["container_configs"]["default"];
    (
        entry["create"][0].as_str().unwrap().to_string(),
        entry["cleanup"][0].as_str().unwrap().to_string(),
    )
}

fn overlay_entry(world: &QuectoWorld, repo: &str, default: bool) -> serde_json::Value {
    let (create, cleanup) = fake_scripts(world);
    let mut entry = serde_json::json!({
        "create": [create, "--repo", repo],
        "cleanup": [cleanup],
    });
    if default {
        entry["default"] = serde_json::json!(true);
    }
    entry
}

/// `quecto config set --local container_configs.<name> '<entry>'`, the
/// agent-shaped command: it writes the overlay and records its trust.
fn config_set_local(world: &mut QuectoWorld, name: &str, entry: &serde_json::Value) {
    let output = cli::run_with_output(
        vec![
            "quecto".to_string(),
            "config".to_string(),
            "set".to_string(),
            "--local".to_string(),
            format!("container_configs.{name}"),
            entry.to_string(),
        ],
        &world.cli_context,
    );
    assert_eq!(
        output.exit_code, 0,
        "quecto config set --local failed:\nstdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
}

#[given(
    expr = "script-managed subagent spawning is available from a checkout with global default script {string}"
)]
fn given_script_spawn_from_checkout(world: &mut QuectoWorld, script: String) {
    given_script_spawn(world, script, None, None);
    let base = base_path(world);
    let parent_config = PathBuf::from(world.config_path.clone().expect("config path"));
    let registry = world
        .agent_cmd_registry
        .clone()
        .expect("registry from live spawn setup");
    let checkout = checkout(world);
    world.spawn_tool = Some(
        quecto::composition::subagent_lifecycle::compose_launcher_in_checkout(
            SpawnTool::with_base_dir(vec![], base.clone())
                .with_socket_dir(base.join("sockets"))
                .with_registry(registry)
                .with_parent_config_path(Some(parent_config)),
            &checkout,
        ),
    );
}

#[given(
    expr = "the checkout binds itself to container config {string} with repository {string} through quecto config set --local"
)]
fn given_checkout_binds_default(world: &mut QuectoWorld, name: String, repo: String) {
    let entry = overlay_entry(world, &repo, true);
    config_set_local(world, &name, &entry);
}

#[given(
    expr = "the checkout adds non-default container config {string} with repository {string} through quecto config set --local"
)]
fn given_checkout_adds_named(world: &mut QuectoWorld, name: String, repo: String) {
    let entry = overlay_entry(world, &repo, false);
    config_set_local(world, &name, &entry);
}

#[given(
    expr = "the checkout carries an untrusted overlay binding container config {string} with repository {string}"
)]
fn given_checkout_untrusted_overlay(world: &mut QuectoWorld, name: String, repo: String) {
    let entry = overlay_entry(world, &repo, true);
    let overlay = checkout(world).join(".quecto").join("config.json");
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    std::fs::write(
        &overlay,
        serde_json::to_string_pretty(&serde_json::json!({"container_configs": {name: entry}}))
            .unwrap(),
    )
    .unwrap();
    assert!(
        !base_path(world).join("config-overlay-trust.json").exists(),
        "the overlay must not be trusted"
    );
}

#[when(
    expr = "I spawn script-managed subagent {string} with default selection and no config argument and task {string}"
)]
fn when_spawn_default_no_config(world: &mut QuectoWorld, agent_id: String, task: String) {
    execute_spawn_json_without_config(
        world,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":true,"read_only":true}),
    );
}

#[when(
    expr = "I spawn script-managed subagent {string} with script {string} and no config argument and task {string}"
)]
fn when_spawn_named_no_config(
    world: &mut QuectoWorld,
    agent_id: String,
    script: String,
    task: String,
) {
    execute_spawn_json_without_config(
        world,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":{"mode":"new","container_config":script},"read_only":true}),
    );
}

/// The overlay the checkout carries, as the diagnostics name it.
fn overlay_path(world: &QuectoWorld) -> String {
    checkout(world)
        .join(".quecto")
        .join("config.json")
        .display()
        .to_string()
}

#[given(
    expr = "the checkout carries a trusted overlay labelling both {string} and {string} as default container configs"
)]
fn given_checkout_trusted_invalid_merge(world: &mut QuectoWorld, first: String, second: String) {
    // `config set --local` refuses a merge that is not valid, so the file is
    // written by hand and approved with `quecto config trust`: the trust
    // record is honest, the merge is not.
    let overlay = checkout(world).join(".quecto").join("config.json");
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    let entries = serde_json::json!({
        first: overlay_entry(world, "https://example.test/repo-1", true),
        second: overlay_entry(world, "https://example.test/repo-2", true),
    });
    std::fs::write(
        &overlay,
        serde_json::to_string_pretty(&serde_json::json!({"container_configs": entries})).unwrap(),
    )
    .unwrap();
    let output = cli::run_with_output(
        vec!["quecto".into(), "config".into(), "trust".into()],
        &world.cli_context,
    );
    assert_eq!(
        output.exit_code, 0,
        "quecto config trust failed:\nstdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
}

#[then(expr = "the spawn result should name container config {string}")]
fn then_spawn_names_container_config(world: &mut QuectoWorld, name: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    let expected = format!(" container_config={name} ");
    assert!(
        !result.is_error && result.content.contains(&expected),
        "expected {expected:?} in: {}",
        result.content
    );
}

#[then("the spawn result should carry no configuration diagnostics")]
fn then_spawn_carries_no_diagnostics(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        !result.content.contains("Configuration diagnostics")
            && !result.content.contains("was not applied"),
        "{}",
        result.content
    );
}

#[then("the spawn result should carry the configuration diagnostic naming the checkout's overlay")]
fn then_spawn_carries_overlay_diagnostic(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    let overlay = overlay_path(world);
    let diagnostic = format!("repo-local config overlay {overlay} is not trusted (sha256 ");
    assert!(
        result.content.contains(&diagnostic)
            && result.content.contains(
                "and was not applied; review it, then run `quecto config trust` from this directory"
            ),
        "expected the diagnostic {diagnostic:?}… in the tool result: {}",
        result.content
    );
    if !result.is_error {
        assert!(
            result
                .content
                .contains("\nConfiguration diagnostics:\nrepo-local config overlay"),
            "a successful spawn lists the diagnostics after the launch line: {}",
            result.content
        );
    }
}

#[then("the spawn result should fail naming the checkout's overlay merged over the global file")]
fn then_spawn_fails_naming_merge(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    let expected = format!(
        "failed to load config {} merged over {}: invalid container_configs: ",
        overlay_path(world),
        world.config_path.clone().expect("config path")
    );
    assert!(
        result.is_error && result.content.contains(&expected),
        "expected {expected:?} in: {}",
        result.content
    );
}

#[then(expr = "the spawn result should fail with {string}")]
fn then_spawn_fails_with(world: &mut QuectoWorld, expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        result.is_error && result.content.contains(&expected),
        "expected an error containing {expected:?}, got: {}",
        result.content
    );
}

#[given(
    expr = "the checkout unbinds container config {string} through quecto config unset --local"
)]
fn given_checkout_unbinds(world: &mut QuectoWorld, name: String) {
    let output = cli::run_with_output(
        vec![
            "quecto".to_string(),
            "config".to_string(),
            "unset".to_string(),
            "--local".to_string(),
            format!("container_configs.{name}"),
        ],
        &world.cli_context,
    );
    assert_eq!(
        output.exit_code, 0,
        "quecto config unset --local failed:\nstdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
}
