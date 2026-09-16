//! Issue #2001 RED acceptance at the existing CLI/UDS runtime boundary.
//!
//! Fixtures save through the real agent command from explicit execution
//! directories. The UDS fixture is then launched from the asserted current
//! execution directory and observed only through public wire events.

use super::*;

fn base_dir(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .base_dir
        .clone()
        .expect("temp base directory must be configured")
}

fn select_execution_folder(world: &mut QuectoWorld, folder: &str) -> PathBuf {
    assert!(
        !folder.is_empty()
            && folder.split('/').all(|component| {
                !component.is_empty()
                    && component
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            }),
        "BDD folder fixture only accepts non-empty relative ASCII alphanumeric/hyphen path components"
    );
    let workspace = base_dir(world).join(folder);
    std::fs::create_dir_all(&workspace).expect("create execution folder");
    world.cli_context.cwd = Some(workspace.clone());
    workspace
}

#[given(expr = "the current execution folder is {string}")]
fn given_current_execution_folder(world: &mut QuectoWorld, folder: String) {
    select_execution_folder(world, &folder);
}

#[given(expr = "a Git repository {string} with execution directories {string} and {string}")]
fn given_git_repository_with_execution_directories(
    world: &mut QuectoWorld,
    repository: String,
    first: String,
    second: String,
) {
    let repository_path = select_execution_folder(world, &repository);
    for relative in [&first, &second] {
        assert!(
            !relative.is_empty()
                && relative
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
            "repository execution-directory fixture requires allowlisted relative names"
        );
        std::fs::create_dir_all(repository_path.join(relative))
            .expect("create repository execution directory");
    }
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&repository_path)
        .status()
        .expect("launch git init");
    assert!(status.success(), "create fixture Git repository");
}

#[given(expr = "the real agent runtime saves session {string} from execution folder {string}")]
fn given_agent_saves_from_folder(world: &mut QuectoWorld, session: String, folder: String) {
    assert!(
        !session.is_empty()
            && session
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
        "BDD session fixture only accepts a production-valid CLI session name"
    );
    let workspace = select_execution_folder(world, &folder);
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

#[then(expr = "the UDS workspace event should announce execution folder {string}")]
fn then_workspace_event_announces_folder(world: &mut QuectoWorld, folder: String) {
    let expected = base_dir(world).join(folder);
    let workspace = world
        .agent_events
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["type"] == "workspace")
        .unwrap_or_else(|| {
            panic!(
                "UDS launch emitted no workspace event: {:#?}",
                world.agent_events
            )
        });
    assert_eq!(
        workspace["path"].as_str(),
        Some(expected.to_string_lossy().as_ref()),
        "public workspace event must report the authoritative UDS launch directory"
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
        "fieldless list_sessions must default to the current execution folder; unrelated global sessions were exposed"
    );
}

fn resume_response(world: &QuectoWorld, id: &str) -> serde_json::Value {
    let response = uds_steps::find_agent_response_by_id(world, id)
        .unwrap_or_else(|| panic!("missing resume_session response {id:?}"));
    assert_eq!(response["command"], "resume_session", "{response:#?}");
    assert_eq!(response["success"], true, "{response:#?}");
    response
}

#[then(
    expr = "the resume_session response with id {string} should successfully select session key {string}"
)]
fn then_resume_successfully_selects_key(world: &mut QuectoWorld, id: String, key: String) {
    let response = resume_response(world, &id);
    assert_eq!(response["data"]["sessionKey"], key, "{response:#?}");
}

fn assert_decision_required(
    world: &QuectoWorld,
    id: &str,
    expected_choices: &str,
    allow_additional: bool,
) {
    let response = resume_response(world, id);
    assert_eq!(
        response["data"]["status"], "decision_required",
        "exact lookup must distinguish planning from completed resume: {response:#?}"
    );
    let decision_id = response["data"]["decisionId"]
        .as_str()
        .expect("decision_required must carry an opaque decisionId");
    assert!(!decision_id.is_empty(), "decisionId must be non-empty");
    let actual: std::collections::BTreeSet<&str> = response["data"]["choices"]
        .as_array()
        .expect("decision_required must carry an affirmative choices array")
        .iter()
        .map(|choice| {
            choice
                .as_str()
                .expect("each eligible resume action must be a string")
        })
        .collect();
    let expected: std::collections::BTreeSet<&str> =
        expected_choices.split(',').map(str::trim).collect();
    if allow_additional {
        assert!(
            expected.is_subset(&actual),
            "decision choices omit a required action: actual={actual:?}, expected at least={expected:?}"
        );
    } else {
        assert_eq!(
            actual, expected,
            "foreign existing-directory choice allowlist must be exact"
        );
    }
}

#[then(
    expr = "the resume_session response with id {string} should require a decision allowing exactly {string}"
)]
fn then_resume_requires_exact_decision(world: &mut QuectoWorld, id: String, choices: String) {
    let response = resume_response(world, &id);
    assert_eq!(
        response["data"]["status"], "decision_required",
        "foreign exact lookup must return an explicit disposition plan: {response:#?}"
    );
    assert_decision_required(world, &id, &choices, false);
}

#[then(
    expr = "the resume_session response with id {string} should require a decision allowing at least {string}"
)]
fn then_resume_requires_decision(world: &mut QuectoWorld, id: String, choices: String) {
    let response = resume_response(world, &id);
    assert_eq!(
        response["data"]["status"], "decision_required",
        "legacy exact lookup must return an explicit association plan: {response:#?}"
    );
    assert_decision_required(world, &id, &choices, true);
}

#[then(expr = "the message histories of responses {string} and {string} should match")]
fn then_message_histories_match(world: &mut QuectoWorld, first: String, second: String) {
    let first_response = uds_steps::find_agent_response_by_id(world, &first)
        .unwrap_or_else(|| panic!("missing get_messages response {first:?}"));
    let second_response = uds_steps::find_agent_response_by_id(world, &second)
        .unwrap_or_else(|| panic!("missing get_messages response {second:?}"));
    assert_eq!(first_response["command"], "get_messages");
    assert_eq!(second_response["command"], "get_messages");
    assert_eq!(
        first_response["data"]["messages"], second_response["data"]["messages"],
        "decision_required planning must not replace or mutate current history"
    );
}
