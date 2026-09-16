//! Issue #2001 RED acceptance slice at the existing CLI/UDS runtime boundary.
//!
//! The fixture creates sessions by running the real agent command after changing
//! only its public configured workspace, then observes the existing fieldless
//! `list_sessions` command. No scope metadata or prospective production API is
//! assumed here.

use super::*;

fn base_dir(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .base_dir
        .clone()
        .expect("temp base directory must be configured")
}

fn configure_execution_folder(world: &mut QuectoWorld, folder: &str) -> PathBuf {
    assert!(
        !folder.is_empty()
            && folder
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "BDD folder fixture only accepts non-empty ASCII alphanumeric/hyphen names"
    );
    let base = base_dir(world);
    let workspace = base.join(folder);
    std::fs::create_dir_all(&workspace).expect("create configured execution folder");

    let config_path = base.join("config.json");
    let raw = std::fs::read_to_string(&config_path).expect("read configured mock provider");
    let mut config: serde_json::Value =
        serde_json::from_str(&raw).expect("mock provider config should be JSON");
    config["agents"]["defaults"]["workspace"] =
        serde_json::Value::String(workspace.display().to_string());
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&config).expect("serialize workspace config"),
    )
    .expect("write workspace config");
    world.cli_context.cwd = Some(workspace.clone());
    workspace
}

#[given(expr = "the configured execution folder is {string}")]
fn given_configured_execution_folder(world: &mut QuectoWorld, folder: String) {
    configure_execution_folder(world, &folder);
}

#[given(expr = "the real agent runtime saves session {string} from configured folder {string}")]
fn given_agent_saves_from_folder(world: &mut QuectoWorld, session: String, folder: String) {
    assert!(
        !session.is_empty()
            && session
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
        "BDD session fixture only accepts a production-valid CLI session name"
    );
    let workspace = configure_execution_folder(world, &folder);
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session.clone(),
        "-m".to_string(),
        format!("message from {}", workspace.display()),
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    assert_eq!(
        output.exit_code, 0,
        "real agent runtime should save fixture session {session:?}; stderr: {}",
        output.stderr
    );
}

#[then(expr = "the folder-local list_sessions response should contain exactly {string}")]
fn then_folder_local_list_contains_exactly(world: &mut QuectoWorld, keys: String) {
    let response = uds_steps::find_agent_response_by_id(world, "local-list")
        .expect("the real UDS runtime should answer list_sessions");
    assert_eq!(
        response["success"], true,
        "list_sessions response: {response:#?}"
    );
    let actual: Vec<&str> = response["data"]["sessions"]
        .as_array()
        .expect("list_sessions data.sessions should be an array")
        .iter()
        .map(|session| {
            session["key"]
                .as_str()
                .expect("each listed session should carry an opaque string key")
        })
        .collect();
    let expected: Vec<&str> = keys.split(',').map(str::trim).collect();
    assert_eq!(
        actual, expected,
        "fieldless list_sessions must default to the configured folder; unrelated global sessions were exposed"
    );
}
