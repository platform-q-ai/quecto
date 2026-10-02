//! PR #1048 follow-up: the CLI build path (`build_agent_from_config`) must
//! thread the context-management knobs (#1044/#1045/#1046) into the agent so
//! non-default user config is never silently ignored at a construction site.
//! Pattern mirrors `agent_935_clamp_tests.rs`. Kept in its own file for the
//! 750-line source gate.

use super::build_tests::selection_for_test;
use super::*;

fn flags_for_wiring_test() -> AgentFlags {
    AgentFlags {
        inherited_context_mode: None,
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: Some("fireworks/small-window".into()),
        max_iterations: Some(5),
        max_time: None,
        uds_mode: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::none(),
        workflow_spec_path: None,
        inherited_tool_policy: None,
        parent_id: None,
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        kill_tool: None,
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        admission_context: None,
        parent_control: None,
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        stdin_is_tty: false,
        environment_registry: None,
        backend: None,
    }
}

#[test]
fn build_agent_from_config_threads_context_knobs_into_the_loop() {
    // Non-default context knobs in config.json plus a model with a declared
    // context window smaller than the configured budget: all of them must
    // reach the built loop. Dropping the context-knob wiring (or any future
    // construction site forgetting it) makes this FAIL.
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"fireworks":{"api_key":"k"}},"agents":{"defaults":{"max_context_tokens":200000,"pin_recent_turns":5,"context_collapse_after_messages":7}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"small-window","contextWindow":100000}]}}}"#,
    )
    .unwrap();
    let flags = flags_for_wiring_test();
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags,
        &mut stderr,
        None,
    )
    .expect("agent build should succeed");
    assert_eq!(
        result.agent.effective_max_context_tokens(),
        // #2405: less the default 8192-token reply, the model declaring no cap.
        100_000 - 8_192,
        "the model's declared window must bound the effective budget (#1044)"
    );
    let (pin, collapse_after_messages) = result.agent.context_knob_snapshot();
    assert_eq!(
        pin, 5,
        "a non-default pin_recent_turns in config must reach the loop (#1045)"
    );
    assert_eq!(
        collapse_after_messages, 7,
        "a non-default context_collapse_after_messages must reach the loop (#1046)"
    );
}

/// #2405 final review L2: a built-in OpenAI model's fixed input limit is
/// carried from the registry through the published catalogue to the loop:
/// gpt-5.3-codex's 400k window less its 128k output, less 5% headroom.
/// Under the shared rule (the record's or the catalogue's prompt limit lost
/// on the way) the ceiling would be 300,000.
#[test]
fn build_agent_from_config_carries_an_openai_fixed_input_limit_to_the_loop() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}},"agents":{"defaults":{"max_context_tokens":300000}}}"#,
    )
    .unwrap();
    let mut flags = flags_for_wiring_test();
    flags.model_override = Some("openai-api/gpt-5.3-codex".into());
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags,
        &mut stderr,
        None,
    )
    .expect("agent build should succeed");
    assert_eq!(result.agent.model(), "openai-api/gpt-5.3-codex");
    assert_eq!(
        result.agent.effective_max_context_tokens(),
        258_400,
        "{stderr}"
    );
}

/// #2342: once its process joins a swarm, a member prunes at the swarm
/// ceiling (`swarm_max_context_tokens`), the lower of it and its budget;
/// before that, and in a process that never joins, the budget alone.
#[test]
fn joining_a_swarm_lowers_the_members_pruning_ceiling() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}},"agents":{"defaults":{"max_context_tokens":150000,"swarm_max_context_tokens":40000}}}"#,
    )
    .unwrap();
    let cfg = tmp.path().join("config.json");
    let build = |participation: crate::infrastructure::tools::swarm_bridge::Participation| {
        let mut flags = flags_for_wiring_test();
        flags.model_override = None;
        flags.swarm_participation = participation;
        let mut stderr = String::new();
        build_agent_from_config(
            tmp.path(),
            &selection_for_test(&cfg, false),
            &flags,
            &mut stderr,
            None,
        )
        .expect("agent build should succeed")
    };

    let participation = crate::infrastructure::tools::swarm_bridge::Participation::shared();
    let member = build(participation.clone());
    assert_eq!(member.agent.effective_max_context_tokens(), 150_000);
    let captured = CapturedLog::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    participation.set(true);
    drop(guard);
    let log = captured.0.lock().unwrap().clone();
    assert!(
        log.contains("quecto::swarm_board")
            && log.contains("swarm member context ceiling engaged")
            && log.contains("ceiling_tokens=40000"),
        "the moment the cap engages is a swarm telemetry event: {log}"
    );
    assert_eq!(
        member.agent.effective_max_context_tokens(),
        40_000,
        "a swarm member's ceiling applies from the moment it joins"
    );

    let loner = build(crate::infrastructure::tools::swarm_bridge::Participation::none());
    assert_eq!(loner.agent.effective_max_context_tokens(), 150_000);
}

/// A tracing writer capturing what the subscriber writes.
#[derive(Clone, Default)]
struct CapturedLog(std::sync::Arc<std::sync::Mutex<String>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(bytes));
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedLog {
    type Writer = Self;
    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

/// #2348: the size-aware collapse's dials reach the built loop.
#[test]
fn build_agent_from_config_threads_the_size_aware_collapse_into_the_loop() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"fireworks":{"api_key":"k"}},"agents":{"defaults":{"context_collapse_large_result_tokens":4000,"context_collapse_large_result_after_turns":5}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"small-window","contextWindow":100000}]}}}"#,
    )
    .unwrap();
    let flags = flags_for_wiring_test();
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags,
        &mut stderr,
        None,
    )
    .expect("agent build should succeed");
    assert_eq!(
        result.agent.large_result_switch().dial(),
        crate::domain::large_result_collapse::LargeResultCollapse {
            over_tokens: 4_000,
            after_turns: 5,
        }
    );
}

/// #2348 review M1: unset, the size-aware collapse is off for an ordinary
/// agent and engages, at the swarm default, when its process joins a swarm.
#[test]
fn joining_a_swarm_engages_the_size_aware_collapse() {
    use crate::domain::large_result_collapse::LargeResultCollapse;
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let cfg = tmp.path().join("config.json");
    let participation = crate::infrastructure::tools::swarm_bridge::Participation::shared();
    let mut flags = flags_for_wiring_test();
    flags.model_override = None;
    flags.swarm_participation = participation.clone();
    let mut stderr = String::new();
    let member = build_agent_from_config(
        tmp.path(),
        &selection_for_test(&cfg, false),
        &flags,
        &mut stderr,
        None,
    )
    .expect("agent build should succeed");
    assert_eq!(
        member.agent.large_result_switch().dial(),
        LargeResultCollapse::DISABLED,
        "off for an agent that takes part in no swarm"
    );
    participation.set(true);
    assert_eq!(
        member.agent.large_result_switch().dial(),
        LargeResultCollapse {
            over_tokens: 2_000,
            after_turns: 3,
        },
        "on at the swarm default from the moment the process joins"
    );
}

/// #2414: watermark is the only context mode. Every construction (a CLI
/// run, a UDS parent, a sub-agent, a swarm member once it joins) runs the
/// watermark pass at the configured marks: a context over the high mark is
/// cut once, behind one archive stub, and nothing is collapsed in place.
#[test]
fn every_construction_runs_the_watermark_pass() {
    use crate::domain::conversation::UserKind;
    use crate::domain::message::Message;
    use crate::domain::turn_origin::prompt;
    use crate::infrastructure::tools::swarm_bridge::Participation;
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"fireworks":{"api_key":"k"}},"agents":{"defaults":{"context_high_tokens":20000,"context_low_tokens":6000}}}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"small-window","contextWindow":100000}]}}}"#,
    )
    .unwrap();
    let cfg = tmp.path().join("config.json");
    let prose = |tag: &str, tokens: usize| {
        let mut text = format!("{tag} ");
        while Message::estimate_tokens(&text) < tokens {
            text.push_str("lorem ipsum dolor sit amet ");
        }
        text
    };
    let member = Participation::shared();
    let constructions: [(&str, Box<dyn Fn(&mut AgentFlags)>); 4] = [
        ("a CLI run", Box::new(|_| {})),
        ("a UDS parent", Box::new(|flags| flags.uds_mode = true)),
        (
            "a sub-agent",
            Box::new(|flags| {
                flags.spawned = true;
                flags.parent_id = Some("parent".into());
            }),
        ),
        (
            "a swarm member",
            Box::new(|flags| flags.swarm_participation = member.clone()),
        ),
    ];
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (construction, adapt) in constructions {
        let mut flags = flags_for_wiring_test();
        adapt(&mut flags);
        let mut stderr = String::new();
        let built = build_agent_from_config(
            tmp.path(),
            &selection_for_test(&cfg, false),
            &flags,
            &mut stderr,
            None,
        )
        .unwrap_or_else(|| panic!("{construction}: {stderr}"));
        member.set(true);
        let mut messages = vec![
            Message::system(prose("system", 300)),
            prompt(prose("brief", 200)),
        ];
        for n in 0..12 {
            let mut answer = Message::assistant(prose(&format!("answer {n}"), 2_000), vec![]);
            answer.turn = Some(1);
            messages.push(answer);
            messages.push(prompt(prose(&format!("prompt {n}"), 50)));
        }
        runtime.block_on(built.agent.prune_resumed_context(&mut messages));
        let stubs = messages
            .iter()
            .filter(|m| m.user_kind == UserKind::ArchiveStub)
            .count();
        assert_eq!(stubs, 1, "{construction}: one cut, one stub");
        assert!(
            messages.iter().all(|m| !m.is_collapsed),
            "{construction}: nothing was collapsed in place"
        );
    }
}

/// #2414: `--inherited-context-mode` is gone with the mode it handed down:
/// an agent started with it is refused, the flag named.
#[test]
fn the_inherited_context_mode_flag_is_refused() {
    let args = [
        "-m".to_string(),
        "hi".to_string(),
        "--inherited-context-mode".to_string(),
        "watermark:256000:70000".to_string(),
    ];
    let mut stderr = String::new();
    assert!(parse_agent_flags(&args, &mut stderr).is_none(), "refused");
    assert!(
        stderr.contains("unknown flag '--inherited-context-mode'"),
        "{stderr}"
    );
}
