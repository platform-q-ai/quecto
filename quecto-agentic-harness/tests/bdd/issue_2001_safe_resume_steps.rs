//! Issue #2001 safe-resume RED acceptance through the public UDS wire.
//!
//! The new protocol is exercised as raw JSON.  This keeps the RED suite
//! source-compatible with today's command enum while still driving the real
//! parser, dispatcher, session store, ownership locks, and response envelope.
//! No prospective Rust DTO or internal use-case API is imported here.

use super::*;
use quecto::application::sessions::ports::SessionStore;
use quecto::domain::session::{
    PersistedSubagentRosterEntry, Session, SubagentLiveness, SubagentRestoreReason,
};
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::interface::cli::uds::{UdsLoopArgs, run_uds_loop};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn base(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .base_dir
        .clone()
        .expect("temp base directory must be configured")
}

fn response_by_id(world: &QuectoWorld, id: &str) -> serde_json::Value {
    let response = uds_steps::find_agent_response_by_id(world, id).unwrap_or_else(|| {
        panic!(
            "the public UDS runtime returned no response with id {id:?}; events={:#?}",
            world.agent_events
        )
    });
    assert_ne!(
        response["command"], "parse_error",
        "approved raw JSON request reached only a parse/protocol error instead of the intended behavior: {response:#}"
    );
    response
}

fn session_key(response: &serde_json::Value) -> &str {
    response["data"]["sessionKey"]
        .as_str()
        .unwrap_or_else(|| panic!("response carries no data.sessionKey: {response:#}"))
}

fn strings_in(value: &serde_json::Value, output: &mut Vec<String>) {
    match value {
        serde_json::Value::String(value) => output.push(value.clone()),
        serde_json::Value::Array(values) => {
            for value in values {
                strings_in(value, output);
            }
        }
        serde_json::Value::Object(fields) => {
            for value in fields.values() {
                strings_in(value, output);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

/// Attach through any connectable UDS route advertised by the Open outcome.
/// Field names and launcher implementation remain unconstrained: the only
/// contract is that successful Open is actually enterable through a public
/// runtime boundary, not a decorative receipt.
fn attach_to_opened_runtime(response: &serde_json::Value) -> (UnixStream, BufReader<UnixStream>) {
    let mut candidates = Vec::new();
    strings_in(response, &mut candidates);
    for candidate in candidates {
        let Ok(writer) = UnixStream::connect(&candidate) else {
            continue;
        };
        let reader_stream = writer.try_clone().expect("clone attached target UDS");
        reader_stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("set target UDS read timeout");
        return (writer, BufReader::new(reader_stream));
    }
    panic!(
        "successful Open did not advertise any connectable public runtime route; response={response:#}"
    );
}

fn decision_id(response: &serde_json::Value) -> Option<&str> {
    response["data"]["decisionId"].as_str()
}

/// Every resolution outcome depends on successful, non-mutating planning.
/// Assert that acceptance precondition before looking for the resolution
/// response. On today's implementation unsafe lookup resumes immediately, so
/// outcome scenarios fail here at the genuine safety contract instead of at a
/// missing-response/setup panic.
fn assert_resolution_was_planned(world: &QuectoWorld) -> serde_json::Value {
    let plan = response_by_id(world, "resume-plan");
    assert_eq!(
        plan["success"], true,
        "exact opaque-key lookup must succeed before disposition: {plan:#}"
    );
    assert_eq!(
        plan["data"]["status"], "decision_required",
        "unsafe lookup must yield a decision before any disposition is sent: {plan:#}"
    );
    let decision = decision_id(&plan)
        .expect("decision_required must expose an opaque, state-bound decisionId");
    assert!(!decision.trim().is_empty(), "decisionId must be non-empty");
    plan
}

fn resolution_response(world: &QuectoWorld, id: &str) -> serde_json::Value {
    let _plan = assert_resolution_was_planned(world);
    if let Some(response) = uds_steps::find_agent_response_by_id(world, id) {
        assert_eq!(
            response["command"], "resolve_resume",
            "resolution must use the approved public command envelope: {response:#}"
        );
        return response;
    }
    if let Some(parse_error) = world.agent_events.iter().rev().find_map(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .filter(|event| event["command"] == "parse_error")
    }) {
        assert!(
            false,
            "approved resolve_resume raw JSON reached the real UDS parser but was rejected instead of producing the correlated action response: {parse_error:#}"
        );
    }
    panic!(
        "approved resolve_resume produced no correlated public response after planning succeeded; events={:#?}",
        world.agent_events
    )
}

fn send_and_read(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    events: &mut Vec<String>,
    request: serde_json::Value,
    id: &str,
) -> Option<serde_json::Value> {
    writeln!(writer, "{request}").expect("write raw JSON UDS request");
    writer.flush().expect("flush raw JSON UDS request");
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        let line = line.trim_end().to_string();
        if line.is_empty() {
            continue;
        }
        let parsed = serde_json::from_str::<serde_json::Value>(&line).ok();
        events.push(line);
        if parsed.as_ref().and_then(|value| value["id"].as_str()) == Some(id) {
            return parsed;
        }
        if parsed
            .as_ref()
            .is_some_and(|value| value["command"] == "parse_error")
        {
            // Return the genuine uncorrelated protocol failure to the driver.
            // Never manufacture the requested id: outcome assertions must not
            // mistake parser rejection for an action response.
            return parsed;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ResolutionFaults {
    change_current_state: bool,
    lock_source_after_plan: bool,
    remove_target_folder: bool,
    block_authoritative_store: bool,
    block_authoritative_store_and_cleanup: bool,
}

/// Drive the approved additive protocol over one real, persistent UDS
/// connection. Resolution is only sent when the real planning response yields
/// an opaque decision id. Thus today's implementation reaches a parsed,
/// correlated `resume_session` response and fails at the intended behavioral
/// assertion; it never fails setup merely because `resolve_resume` is not yet
/// in the production Rust command enum.
fn drive_resume_resolution(
    world: &mut QuectoWorld,
    target: &str,
    action: Option<&str>,
    locate_path: Option<&Path>,
    replay: bool,
    faults: ResolutionFaults,
) {
    assert!(
        world.uds_exit_code.is_none(),
        "scenario may drive UDS only once"
    );
    let base = base(world);
    let ctx = uds_steps::build_uds_agent(world, &base).expect("compose real UDS agent");
    let uds_steps::UdsAgentContext {
        agent,
        model,
        session_key: active_key,
        ephemeral,
        ext_registry,
        persist: _,
        workflow_state,
        workflow_config,
        broadcast_tx: _,
        mut provider_reload,
        provider_reload_inputs,
    } = ctx;
    let workspace = world
        .cli_context
        .cwd
        .clone()
        .expect("effective execution directory");
    assert!(workspace.is_dir(), "execution directory must exist");
    let socket_path = base.join("safe-resume.sock");
    let (server, mut writer) = UnixStream::pair().expect("UDS pair");
    let mut reader = BufReader::new(writer.try_clone().expect("clone UDS client"));
    reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set UDS read timeout");
    let base_for_loop = base.clone();
    let workspace_for_loop = workspace.clone();
    let handle = std::thread::spawn(move || {
        run_uds_loop(UdsLoopArgs {
            agent,
            retention: None,
            base_dir: &base_for_loop,
            workspace: &workspace_for_loop,
            identity: SessionIdentity::from_persisted_key(&active_key),
            model,
            ephemeral,
            system_prompt: String::new(),
            socket_path,
            socket_override: Some(server),
            sessions: quecto::composition::sessions::build_session_handles,
            session_store_override: None,
            ext_registry: Some(ext_registry),
            lifetime: quecto::domain::harness_lifetime::HarnessLifetime::UntilLastClientDisconnects,
            notification_rx: None,
            subagent_registry: None,
            harness_lifecycle: None,
            workflow_state,
            workflow_config,
            broadcast_tx: None,
            provider_reload: Some(&mut provider_reload),
            provider_reload_inputs: Some(&provider_reload_inputs),
            parent_control: None,
            teardown_graph: None,
        })
    });

    let mut events = Vec::new();
    let _ = send_and_read(
        &mut writer,
        &mut reader,
        &mut events,
        serde_json::json!({"type":"get_state","id":"state-before"}),
        "state-before",
    );
    let _ = send_and_read(
        &mut writer,
        &mut reader,
        &mut events,
        serde_json::json!({"type":"get_messages","id":"messages-before"}),
        "messages-before",
    );
    let plan = send_and_read(
        &mut writer,
        &mut reader,
        &mut events,
        serde_json::json!({"type":"resume_session","id":"resume-plan","session":target}),
        "resume-plan",
    )
    .expect("resume_session must return its existing public response envelope");

    let mut hidden_sessions_dir = None;
    if let (Some(action), Some(decision)) = (action, decision_id(&plan)) {
        if faults.lock_source_after_plan {
            use std::os::unix::fs::OpenOptionsExt;
            let identity = SessionIdentity::from_persisted_key(target);
            let layout = FlatSessionLayout::new(base.clone());
            let stamp = layout.ownership_stamp(&identity);
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(stamp)
                .expect("open post-plan ownership stamp");
            file.try_lock().expect("hold post-plan ownership conflict");
            // Keep the competing claim live through resolution.
            std::mem::forget(file);
        }
        if faults.change_current_state {
            let _ = send_and_read(
                &mut writer,
                &mut reader,
                &mut events,
                serde_json::json!({"type":"new_session","id":"intervening-state-change"}),
                "intervening-state-change",
            );
        }
        if faults.remove_target_folder {
            let target_dir = base.join("project-b");
            if target_dir.exists() {
                std::fs::remove_dir_all(&target_dir).expect("remove target after planning");
            }
        }
        if faults.block_authoritative_store || faults.block_authoritative_store_and_cleanup {
            let sessions = base.join("sessions");
            let hidden = base.join("sessions-authoritative-backup");
            std::fs::rename(&sessions, &hidden).expect("hide authoritative sessions directory");
            std::fs::write(&sessions, b"not a directory").expect("block authoritative writes");
            if faults.block_authoritative_store_and_cleanup {
                std::fs::set_permissions(
                    &base,
                    std::os::unix::fs::PermissionsExt::from_mode(0o500),
                )
                .expect("deny rollback cleanup fixture");
            }
            hidden_sessions_dir = Some(hidden);
        }
        let mut resolution = serde_json::json!({
            "type":"resolve_resume",
            "id":"resolve-1",
            "decisionId":decision,
            "action":action
        });
        if let Some(path) = locate_path {
            resolution["path"] = serde_json::Value::String(path.to_string_lossy().into_owned());
        }
        let _resolution_response = send_and_read(
            &mut writer,
            &mut reader,
            &mut events,
            resolution.clone(),
            "resolve-1",
        );
        if replay {
            resolution["id"] = serde_json::Value::String("resolve-replay".into());
            let _replay_response = send_and_read(
                &mut writer,
                &mut reader,
                &mut events,
                resolution,
                "resolve-replay",
            );
        }
        if let Some(hidden) = hidden_sessions_dir {
            if faults.block_authoritative_store_and_cleanup {
                std::fs::set_permissions(
                    &base,
                    std::os::unix::fs::PermissionsExt::from_mode(0o700),
                )
                .expect("restore fixture directory permissions");
            }
            std::fs::remove_file(base.join("sessions")).expect("remove write blocker");
            std::fs::rename(hidden, base.join("sessions")).expect("restore authoritative records");
        }
    }

    let _ = send_and_read(
        &mut writer,
        &mut reader,
        &mut events,
        serde_json::json!({"type":"get_state","id":"state-after"}),
        "state-after",
    );
    let _ = send_and_read(
        &mut writer,
        &mut reader,
        &mut events,
        serde_json::json!({"type":"get_messages","id":"messages-after"}),
        "messages-after",
    );
    writer.shutdown(Shutdown::Write).expect("close UDS writer");
    let mut tail = String::new();
    let _ = reader.read_to_string(&mut tail);
    events.extend(
        tail.lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned),
    );
    world.uds_exit_code = Some(handle.join().expect("UDS loop thread"));
    world.agent_events = events;
}

#[given(expr = "legacy session {string} contains transcript and unsafe runtime state")]
fn given_legacy_session_with_unsafe_state(world: &mut QuectoWorld, key: String) {
    let identity = SessionIdentity::from_persisted_key(&key);
    let mut tool_result = Message::tool("unsafe-call", "unsafe tool output");
    tool_result.tool_name = Some("bash".into());
    let session = Session {
        key: identity.clone(),
        messages: vec![
            Message::user("safe user context"),
            Message::assistant("safe assistant context", vec![]),
            tool_result,
        ],
        workflow_run: Some(quecto::domain::workflow::WorkflowRunPersisted {
            template_id: Some("unsafe-workflow".into()),
            done: vec![true, false],
            active_issue: None,
        }),
        subagent_roster: vec![PersistedSubagentRosterEntry {
            agent_uuid: "unsafe-child".into(),
            display_name: "unsafe child".into(),
            session_key: "cli:unsafe-child".into(),
            liveness: SubagentLiveness::Live,
            restore_reason: SubagentRestoreReason::LegacyUnspecified,
            parent_id: None,
            read_only: false,
            status: Some("running".into()),
            delivered_message_ordinal: None,
            pending_message_reports: std::collections::VecDeque::new(),
        }],
    };
    let store = FileSessionStore::new(FlatSessionLayout::new(base(world)));
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(store.save(&session))
        .expect("save unsafe source fixture");
    store.release(&identity);
}

#[given(expr = "execution folder {string} exists")]
fn given_execution_folder_exists(world: &mut QuectoWorld, folder: String) {
    std::fs::create_dir_all(base(world).join(folder)).expect("create execution folder");
}

#[given(expr = "execution folder {string} is removed")]
fn given_execution_folder_removed(world: &mut QuectoWorld, folder: String) {
    let directory = base(world).join(folder);
    if directory.exists() {
        std::fs::remove_dir_all(directory).expect("remove execution folder");
    }
}

#[given(expr = "execution folder {string} enables folder-local web_fetch")]
fn given_folder_enables_web_fetch(world: &mut QuectoWorld, folder: String) {
    let directory = base(world).join(folder).join(".quecto");
    std::fs::create_dir_all(&directory).expect("create target config directory");
    std::fs::write(
        directory.join("config.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tools": {"web": {"fetch": {"enabled": true}}}
        }))
        .expect("serialize target configuration"),
    )
    .expect("write target configuration");
}

#[given(expr = "execution folder {string} has invalid folder-local configuration")]
fn given_invalid_folder_config(world: &mut QuectoWorld, folder: String) {
    let directory = base(world).join(folder).join(".quecto");
    std::fs::create_dir_all(&directory).expect("create target config directory");
    std::fs::write(directory.join("config.json"), "{ invalid target config")
        .expect("write invalid target configuration");
}

#[when(expr = "I request exact resume {string} without choosing a disposition")]
fn when_plan_only(world: &mut QuectoWorld, target: String) {
    drive_resume_resolution(
        world,
        &target,
        None,
        None,
        false,
        ResolutionFaults::default(),
    );
}

#[when(expr = "I request exact resume {string} and choose {string}")]
fn when_resolve(world: &mut QuectoWorld, target: String, action: String) {
    drive_resume_resolution(
        world,
        &target,
        Some(&action),
        None,
        false,
        ResolutionFaults::default(),
    );
}

#[when(expr = "I request exact resume {string} and choose locate at execution folder {string}")]
fn when_locate(world: &mut QuectoWorld, target: String, folder: String) {
    let path = base(world).join(folder);
    drive_resume_resolution(
        world,
        &target,
        Some("locate"),
        Some(&path),
        false,
        ResolutionFaults::default(),
    );
}

#[when(expr = "I request exact resume {string}, choose {string}, and replay the decision")]
fn when_resolve_and_replay(world: &mut QuectoWorld, target: String, action: String) {
    drive_resume_resolution(
        world,
        &target,
        Some(&action),
        None,
        true,
        ResolutionFaults::default(),
    );
}

#[when(
    expr = "I request exact resume {string}, lock its source after planning, and choose {string}"
)]
fn when_resolve_ownership_conflict(world: &mut QuectoWorld, target: String, action: String) {
    drive_resume_resolution(
        world,
        &target,
        Some(&action),
        None,
        false,
        ResolutionFaults {
            lock_source_after_plan: true,
            ..ResolutionFaults::default()
        },
    );
}

#[when(expr = "I request exact resume {string}, change the active session, and choose {string}")]
fn when_resolve_stale(world: &mut QuectoWorld, target: String, action: String) {
    drive_resume_resolution(
        world,
        &target,
        Some(&action),
        None,
        false,
        ResolutionFaults {
            change_current_state: true,
            ..ResolutionFaults::default()
        },
    );
}

#[when(expr = "I request exact resume {string}, remove its folder, and choose {string}")]
fn when_remove_then_resolve(world: &mut QuectoWorld, target: String, action: String) {
    drive_resume_resolution(
        world,
        &target,
        Some(&action),
        None,
        false,
        ResolutionFaults {
            remove_target_folder: true,
            ..ResolutionFaults::default()
        },
    );
}

#[when(expr = "I request exact resume {string} and fork while authoritative writes fail")]
fn when_fork_write_fails(world: &mut QuectoWorld, target: String) {
    drive_resume_resolution(
        world,
        &target,
        Some("fork_current"),
        None,
        false,
        ResolutionFaults {
            block_authoritative_store: true,
            ..ResolutionFaults::default()
        },
    );
}

#[when(expr = "I request exact resume {string} and fork while rollback cleanup also fails")]
fn when_fork_rollback_fails(world: &mut QuectoWorld, target: String) {
    drive_resume_resolution(
        world,
        &target,
        Some("fork_current"),
        None,
        false,
        ResolutionFaults {
            block_authoritative_store_and_cleanup: true,
            ..ResolutionFaults::default()
        },
    );
}

#[then(expr = "the resume plan should immediately resume opaque key {string}")]
fn then_immediate_resume(world: &mut QuectoWorld, key: String) {
    let response = response_by_id(world, "resume-plan");
    assert_eq!(response["success"], true, "{response:#}");
    // Same-directory exact resume is unchanged compatibility. The bounded
    // contract allows, but does not require, an additive `status` field.
    assert_eq!(response["data"]["sessionKey"], key, "{response:#}");
    assert!(
        response["data"]["messageCount"].is_number(),
        "compatible resume must retain its existing messageCount: {response:#}"
    );
}

#[then(expr = "the resume plan should require exactly the choices {string}")]
fn then_plan_choices(world: &mut QuectoWorld, expected: String) {
    let response = response_by_id(world, "resume-plan");
    assert_eq!(
        response["success"], true,
        "global exact lookup failed: {response:#}"
    );
    assert_eq!(
        response["data"]["status"], "decision_required",
        "exact lookup silently resumed instead of planning: {response:#}"
    );
    let decision =
        decision_id(&response).expect("decision_required must return an opaque decisionId");
    assert!(
        !decision.trim().is_empty(),
        "decisionId must be opaque and non-empty"
    );
    let mut actual: Vec<&str> = response["data"]["choices"]
        .as_array()
        .expect("decision_required must return an affirmative choices array")
        .iter()
        .map(|choice| choice.as_str().expect("choice must be a string"))
        .collect();
    let mut wanted: Vec<&str> = expected.split(',').map(str::trim).collect();
    actual.sort_unstable();
    wanted.sort_unstable();
    assert_eq!(
        actual, wanted,
        "authoritative action allowlist differs: {response:#}"
    );
}

#[then(expr = "the resume plan should require the legacy choices {string}")]
fn then_plan_legacy_choices(world: &mut QuectoWorld, required: String) {
    let response = response_by_id(world, "resume-plan");
    assert_eq!(
        response["success"], true,
        "global exact lookup failed: {response:#}"
    );
    assert_eq!(
        response["data"]["status"], "decision_required",
        "legacy lookup silently resumed instead of planning: {response:#}"
    );
    let decision = decision_id(&response).expect("decision_required must return decisionId");
    assert!(!decision.trim().is_empty(), "decisionId must be non-empty");
    let actual: Vec<&str> = response["data"]["choices"]
        .as_array()
        .expect("decision_required must return choices")
        .iter()
        .map(|choice| choice.as_str().expect("choice must be string"))
        .collect();
    for wanted in required.split(',').map(str::trim) {
        assert!(
            actual.contains(&wanted),
            "legacy plan omits required {wanted:?}: {response:#}"
        );
    }
    assert!(
        actual.iter().all(|choice| matches!(
            *choice,
            "associate_current" | "fork_current" | "cancel" | "locate"
        )),
        "legacy plan returned an action outside the approved affirmative allowlist: {response:#}"
    );
}

#[then("planning should preserve the active session key and history")]
fn then_planning_preserves_state(world: &mut QuectoWorld) {
    let before = response_by_id(world, "state-before");
    let after = response_by_id(world, "state-after");
    assert_eq!(
        session_key(&before),
        session_key(&after),
        "planning changed active session: before={before:#} after={after:#}"
    );
    let before_messages = response_by_id(world, "messages-before");
    let after_messages = response_by_id(world, "messages-after");
    assert_eq!(
        before_messages["data"]["messages"], after_messages["data"]["messages"],
        "planning changed active history"
    );
}

#[then(expr = "resolution should succeed with status {string}")]
fn then_resolution_succeeds(world: &mut QuectoWorld, status: String) {
    let response = resolution_response(world, "resolve-1");
    assert_eq!(response["success"], true, "resolution failed: {response:#}");
    assert_eq!(
        response["data"]["status"], status,
        "unexpected resolution outcome: {response:#}"
    );
}

#[then("resolution should fail without replacing the active conversation")]
fn then_resolution_fails_safely(world: &mut QuectoWorld) {
    let response = resolution_response(world, "resolve-1");
    assert_eq!(
        response["success"], false,
        "unsafe resolution reported success: {response:#}"
    );
    then_planning_preserves_state(world);
}

#[then("replaying the consumed decision should fail without another mutation")]
fn then_replay_fails(world: &mut QuectoWorld) {
    let replay = resolution_response(world, "resolve-replay");
    assert_eq!(
        replay["success"], false,
        "single-use decision replay succeeded: {replay:#}"
    );
}

#[then("the fork should use a new opaque key and retain only safe transcript state")]
fn then_fork_is_sanitized(world: &mut QuectoWorld) {
    let plan = response_by_id(world, "resume-plan");
    let resolved = resolution_response(world, "resolve-1");
    assert_eq!(resolved["success"], true, "{resolved:#}");
    assert_eq!(resolved["data"]["status"], "forked", "{resolved:#}");
    let new_key = session_key(&resolved);
    assert_ne!(
        new_key,
        plan["data"]["session"].as_str().unwrap_or_default(),
        "fork reused source identity"
    );
    assert_ne!(
        new_key,
        session_key(&response_by_id(world, "state-before")),
        "fork reused current identity"
    );
    assert_eq!(
        session_key(&response_by_id(world, "state-after")),
        new_key,
        "fork did not become active"
    );
    let identity = SessionIdentity::from_persisted_key(new_key);
    let store = FileSessionStore::new(FlatSessionLayout::new(base(world)));
    let fork = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(store.load(&identity))
        .expect("load fork")
        .expect("successful fork must be durably authoritative");
    assert!(
        fork.workflow_run.is_none(),
        "fork restored workflow execution state"
    );
    assert!(
        fork.subagent_roster.is_empty(),
        "fork restored child ownership/roster state"
    );
    assert!(
        fork.messages
            .iter()
            .any(|message| message.content == "safe user context")
    );
    assert!(
        fork.messages
            .iter()
            .any(|message| message.content == "safe assistant context")
    );
    assert!(
        fork.messages
            .iter()
            .all(|message| message.role != Role::Tool),
        "fork retained unsafe tool runtime state: {:#?}",
        fork.messages
    );
}

#[then("failed fork should publish no phantom destination and retain the current session")]
fn then_failed_fork_has_no_phantom(world: &mut QuectoWorld) {
    let response = resolution_response(world, "resolve-1");
    assert_eq!(
        response["success"], false,
        "authoritative write failure reported success: {response:#}"
    );
    let before = response_by_id(world, "state-before");
    let after = response_by_id(world, "state-after");
    assert_eq!(
        session_key(&before),
        session_key(&after),
        "failed fork replaced current session"
    );
    let records = std::fs::read_dir(base(world).join("sessions"))
        .expect("restored authoritative directory")
        .filter_map(Result::ok)
        .filter(|entry| FlatSessionLayout::is_session_record(&entry.path()))
        .count();
    assert_eq!(
        records, 1,
        "failed fork left a phantom authoritative destination"
    );
}

#[then("rollback cleanup failure should be reported explicitly with recovery information")]
fn then_rollback_failure_is_explicit(world: &mut QuectoWorld) {
    let response = resolution_response(world, "resolve-1");
    assert_eq!(
        response["success"], false,
        "rollback-failure path reported success: {response:#}"
    );
    let diagnostic = response["error"]
        .as_str()
        .filter(|diagnostic| !diagnostic.trim().is_empty());
    assert!(
        diagnostic.is_some(),
        "rollback/cleanup failure must return an observable diagnostic instead of false or silent success: {response:#}"
    );
    let retained = response["data"]["retainedArtifacts"].as_array();
    let recovery = response["data"]["recovery"].as_str();
    assert!(
        retained.is_some_and(|artifacts| !artifacts.is_empty())
            || recovery.is_some_and(|guidance| !guidance.trim().is_empty()),
        "rollback failure must expose retained artifacts or recovery guidance without prescribing error vocabulary: {response:#}"
    );
}

#[then(expr = "open original should be attachably ready in execution folder {string}")]
fn then_open_is_ready(world: &mut QuectoWorld, folder: String) {
    let response = resolution_response(world, "resolve-1");
    assert_eq!(
        response["success"], true,
        "open-original failed: {response:#}"
    );
    let (mut writer, mut reader) = attach_to_opened_runtime(&response);
    let mut attached_events = Vec::new();
    let state = send_and_read(
        &mut writer,
        &mut reader,
        &mut attached_events,
        serde_json::json!({"type":"get_state","id":"opened-state"}),
        "opened-state",
    )
    .expect("attachably ready target must answer get_state");
    assert_eq!(
        state["success"], true,
        "target runtime is not enterable: {state:#}"
    );
    let workspace = attached_events.iter().find_map(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .filter(|event| event["type"] == "workspace")
            .and_then(|event| event["path"].as_str().map(str::to_owned))
    });
    let expected = base(world)
        .join(folder)
        .canonicalize()
        .expect("target canonical path");
    let actual = PathBuf::from(workspace.expect("target attachment must publish workspace"))
        .canonicalize()
        .expect("opened workspace canonical path");
    assert_eq!(
        actual, expected,
        "Open attached to the wrong execution directory"
    );
}

#[then("locate should return a coherent same-directory resume outcome")]
fn then_locate_current_is_coherent(world: &mut QuectoWorld) {
    let response = resolution_response(world, "resolve-1");
    assert_eq!(
        response["success"], true,
        "locate-current failed: {response:#}"
    );
    let status = response["data"]["status"].as_str().unwrap_or_default();
    assert!(
        matches!(status, "resumed" | "decision_required"),
        "locate-current returned a blindly foreign plan: {response:#}"
    );
    if status == "decision_required" {
        let choices = response["data"]["choices"]
            .as_array()
            .expect("fresh plan choices");
        assert!(
            !choices.iter().any(|choice| choice == "open_original"),
            "same-directory locate offered open_original: {response:#}"
        );
    }
}

#[then("locate should return a fresh foreign plan without an implicit launch")]
fn then_locate_elsewhere_replans(world: &mut QuectoWorld) {
    let response = resolution_response(world, "resolve-1");
    assert_eq!(
        response["success"], true,
        "locate elsewhere failed: {response:#}"
    );
    assert_eq!(
        response["data"]["status"], "decision_required",
        "locate elsewhere implicitly launched or resumed: {response:#}"
    );
    let choices = response["data"]["choices"]
        .as_array()
        .expect("fresh foreign choices");
    for expected in ["open_original", "fork_current", "cancel"] {
        assert!(
            choices.iter().any(|choice| choice == expected),
            "fresh foreign plan omits {expected}: {response:#}"
        );
    }
    assert_ne!(
        response["data"]["decisionId"],
        response_by_id(world, "resume-plan")["data"]["decisionId"],
        "Locate reused the stale pre-Locate decision"
    );
    then_planning_preserves_state(world);
}

#[then(expr = "execution folder {string} should remain absent")]
fn then_folder_remains_absent(world: &mut QuectoWorld, folder: String) {
    assert!(
        !base(world).join(folder).exists(),
        "Locate created an operator-selected missing directory"
    );
}

#[then(expr = "the entered target runtime should rediscover tool {string}")]
fn then_target_tool_is_rediscovered(world: &mut QuectoWorld, tool: String) {
    let response = resolution_response(world, "resolve-1");
    let (mut writer, mut reader) = attach_to_opened_runtime(&response);
    let mut attached_events = Vec::new();
    let catalogue = send_and_read(
        &mut writer,
        &mut reader,
        &mut attached_events,
        serde_json::json!({"type":"get_tool_catalogue","id":"opened-tools"}),
        "opened-tools",
    )
    .expect("entered target runtime must answer get_tool_catalogue");
    assert_eq!(
        catalogue["success"], true,
        "target tool query failed: {catalogue:#}"
    );
    let tools = catalogue["data"]["tools"]
        .as_array()
        .expect("target catalogue must expose tools");
    assert!(
        tools
            .iter()
            .any(|entry| entry["name"] == tool || entry == &tool),
        "entered target runtime did not rediscover folder-local tool {tool:?}: {catalogue:#}"
    );
}

#[then("the stale decision should fail after the active state changes")]
fn then_stale_fails(world: &mut QuectoWorld) {
    let response = resolution_response(world, "resolve-1");
    assert_eq!(
        response["success"], false,
        "state-bound stale decision succeeded: {response:#}"
    );
    let changed = response_by_id(world, "intervening-state-change");
    assert_eq!(
        session_key(&response_by_id(world, "state-after")),
        session_key(&changed),
        "stale resolution displaced the intervening active session"
    );
}
