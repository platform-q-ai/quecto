//! Regression coverage for effective policy restoration and canonical provider loans.
use super::*;
use crate::application::sessions::ports::SessionStore;
use crate::domain::agents::value_objects::turn_outcome::DECLARE_GUIDANCE;
use crate::domain::conversation::value_objects::message::Role;
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::config::{Config, ToolPolicyEntryConfig};
use std::sync::OnceLock;

#[derive(Debug)]
struct OwnershipProvider {
    seen: Mutex<Vec<(bool, String)>>,
    entered: Notify,
    release: Notify,
}
static RELOAD_PROVIDER: OnceLock<Arc<OwnershipProvider>> = OnceLock::new();
fn rebuilt(
    _: &Config,
    _: &std::path::Path,
    _: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    Ok(RELOAD_PROVIDER.get().unwrap().clone())
}
impl LlmProvider for OwnershipProvider {
    fn name(&self) -> &str {
        "run23-ownership"
    }
    fn route_order(&self) -> Vec<String> {
        vec![self.name().into()]
    }
    fn chat<'a>(
        &'a self,
        _: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async { panic!("real socket uses streaming") })
    }
    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        let head = request
            .messages
            .iter()
            .find(|m| m.role == Role::System && !m.is_manifest)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let offered = request
            .tools
            .iter()
            .any(|t| t.name == "declare_turn_outcome");
        let n = {
            let mut seen = self.seen.lock().unwrap();
            seen.push((offered, head));
            seen.len()
        };
        assert!(n <= 5, "bounded exact script; unexpected request {n}");
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            if n == 1 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            let mut result = reply(if n == 2 { "" } else { "Bounded answer" });
            if n == 1 {
                result.tool_calls.push(ToolCall {
                    id: "ownership-declare".into(),
                    name: "declare_turn_outcome".into(),
                    arguments: json!({"outcome":"done","reason":"original authority"}).to_string(),
                });
            }
            tx.send(StreamEvent::Done(result)).await.unwrap();
            rx
        })
    }
}
fn write_config(path: &std::path::Path, stable: &str, enabled: bool) {
    let mut config = Config::default();
    config.tools.policy.entries.insert(
        stable.into(),
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
async fn run24_concurrent_real_policy_config_composed_head_ownership() {
    if !isolated(
        concat!(
            module_path!(),
            "::run24_concurrent_real_policy_config_composed_head_ownership"
        ),
        &["unset", "1"],
    ) {
        return;
    }
    let _signal = crate::interface::uds::test_support::SIMULATED_SIGNAL
        .read()
        .await;
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("ownership.sock");
    let config_path = dir.path().join("config-probe.json");
    let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let composed = crate::interface::cli::controllers::active_runtime_2708_tests::compose(true);
    let stable = composed
        .registry
        .catalogue_entries()
        .into_iter()
        .find(|e| e.name == "declare_turn_outcome")
        .unwrap()
        .stable_id
        .into_owned();
    write_config(&config_path, &stable, true);
    let canonical = crate::interface::cli::controllers::shared::build_agent_system_prompt(
        Some("AGENTS owner"),
        Some("config owner"),
        false,
        &composed.snippets,
    );
    let provider = Arc::new(OwnershipProvider {
        seen: Mutex::new(vec![]),
        entered: Notify::new(),
        release: Notify::new(),
    });
    RELOAD_PROVIDER.set(provider.clone()).unwrap();
    let (ap, base, path, prompt) = (
        provider.clone(),
        dir.path().to_path_buf(),
        config_path.clone(),
        canonical.clone(),
    );
    let thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let agent = AgentLoopImpl::new(AgentLoopConfig { provider: ap, tool_registry: Box::new(composed.registry), model: "stub".into(), max_tokens: 32, temperature: 0.0, retention: None, session_key: KEY.into(), max_context_tokens: 190_000, progress_callback: None, streaming: true, effort: None, audit_log: None, pin_recent_turns: 2, context_marks: Default::default(), model_context_window: None, tool_profile_context: crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent });
            let mut inputs = crate::interface::uds::controllers::uds::dispatch_session_roster_tests::loop_inputs(&base, KEY);
            inputs.system_prompt = prompt.clone();
            let store = crate::composition::sessions::build_session_handles(inputs);
            let mut catalogue = crate::composition::catalogue::build_catalogue_handles(&base, None);
            let load = path.clone();
            let source = crate::infrastructure::config::runtime_configuration::FileRuntimeConfiguration::seeded(vec![crate::infrastructure::config::reload::ReloadSource::new(path)], base.clone(), Arc::new(move || serde_json::from_str(&std::fs::read_to_string(&load).map_err(|e| e.to_string())?).map_err(|e| e.to_string())), reqwest::Client::new(), rebuilt);
            catalogue.reload = Arc::new(crate::application::catalogue::use_cases::ReloadRuntimeConfiguration::new(Box::new(source)));
            let args = MultiClientArgs { parent_outcome: composed.runtime, extensions: Default::default(), agent, workspace: &base, messages: Vec::new(), model: "stub".into(), admission_slots: Vec::new(), session_key: KEY.into(), system_prompt: prompt, ext_registry: None, lifetime: crate::domain::agents::value_objects::harness_lifetime::HarnessLifetime::UntilLastClientDisconnects, notification_rx: None, subagent_registry: None, harness_lifecycle: None, workflow_state: None, workflow_config: None, broadcast_tx: None, parent_control: None, teardown_graph: None, turn_session: crate::composition::bootstrap::agent_turn::build_turn_session, environment_control: None };
            multi_client_loop(args, tokio::net::UnixListener::from_std(listener).unwrap(), &store, &catalogue).await
        })
    });
    let client = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let (reader, mut writer) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(reader).lines();
    let mut frames = vec![];
    send(
        &mut writer,
        json!({"type":"prompt","id":"ownership-first","message":"Do work"}),
    )
    .await;
    tokio::time::timeout(STEP, provider.entered.notified())
        .await
        .unwrap();
    // The real provider holds the guidance/image loan while both external inputs arrive.
    write_config(&config_path, &stable, false);
    send(&mut writer, json!({"type":"set_tool_policy","id":"ownership-remove","operation":"replace","unlistedScope":"none","mutations":[],"mode":"immediateIfIdle","persist":false})).await;
    send(
        &mut writer,
        json!({"type":"reload","id":"ownership-reload"}),
    )
    .await;
    provider.release.notify_one();
    assert!(
        collect_until(&mut lines, &mut frames, STEP, |f| response(
            f,
            "ownership-reload"
        ))
        .await
    );
    send(
        &mut writer,
        json!({"type":"prompt","id":"ownership-disabled","message":"unguarded"}),
    )
    .await;
    send(
        &mut writer,
        json!({"type":"set_effort","id":"ownership-fence1","effort":"low"}),
    )
    .await;
    assert!(
        collect_until(&mut lines, &mut frames, STEP, |f| response(
            f,
            "ownership-fence1"
        ))
        .await
    );
    write_config(&config_path, &stable, true);
    send(
        &mut writer,
        json!({"type":"reload","id":"ownership-restore"}),
    )
    .await;
    assert!(
        collect_until(&mut lines, &mut frames, STEP, |f| response(
            f,
            "ownership-restore"
        ))
        .await
    );
    send(&mut writer, json!({"type":"set_tool_policy","id":"ownership-restore-session","operation":"replace","unlistedScope":"both","mutations":[],"mode":"immediateIfIdle","persist":false})).await;
    assert!(
        collect_until(&mut lines, &mut frames, STEP, |f| response(
            f,
            "ownership-restore-session"
        ))
        .await
    );
    send(
        &mut writer,
        json!({"type":"prompt","id":"ownership-restored","message":"guarded"}),
    )
    .await;
    send(
        &mut writer,
        json!({"type":"set_effort","id":"ownership-fence2","effort":"low"}),
    )
    .await;
    assert!(
        collect_until(&mut lines, &mut frames, STEP, |f| response(
            f,
            "ownership-fence2"
        ))
        .await
    );
    drop(writer);
    drop(lines);
    assert_eq!(
        tokio::time::timeout(STEP, tokio::task::spawn_blocking(move || thread.join()))
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        0
    );
    let seen = provider.seen.lock().unwrap().clone();
    let store = crate::composition::sessions::build_file_session_store(dir.path());
    let saved = store
        .load(&SessionIdentity::from_persisted_key(KEY))
        .await
        .unwrap()
        .unwrap();
    println!(
        "RUN24 requests={} offered={:?} saved={} frames={}",
        seen.len(),
        seen.iter().map(|s| s.0).collect::<Vec<_>>(),
        saved.messages.len(),
        frames.len()
    );
    dir.close().unwrap();
    assert_eq!(
        seen.len(),
        5,
        "first declaration/drain, disabled once, restored/correction"
    );
    assert!(seen[0].0);
    assert!(seen[1].0);
    assert!(!seen[2].0);
    assert!(seen[3].0);
    assert!(seen[4].0);
    for (offered, head) in &seen {
        assert_eq!(
            head.contains(DECLARE_GUIDANCE),
            *offered,
            "guidance matches effective tool policy"
        );
        assert!(head.contains("AGENTS owner") && head.contains("config owner"));
        let (_, framed) = head
            .rsplit_once("\n\n<extensions>\nContent length: ")
            .unwrap();
        let (length, body) = framed.split_once(" bytes\n\n").unwrap();
        assert_eq!(
            length.parse::<usize>().unwrap(),
            body.strip_suffix("\n\n</extensions>").unwrap().len()
        );
    }
    assert!(
        saved.messages.iter().all(|m| m.role != Role::System),
        "canonical composed head must not leak into durable history"
    );
    assert_eq!(seen[0].1, canonical);
    assert_eq!(seen[3].1, canonical);
    assert!(
        frames
            .iter()
            .any(|f| response(f, "ownership-reload") && f["success"] == true)
    );
    assert!(
        frames
            .iter()
            .any(|f| response(f, "ownership-restore") && f["success"] == true)
    );
    for id in ["ownership-remove", "ownership-restore-session"] {
        assert!(
            frames
                .iter()
                .any(|f| response(f, id) && f["success"] == true),
            "actual policy transition succeeded: {id}"
        );
    }
}

#[derive(Debug)]
struct PendingLoan {
    entered: Notify,
    seen: Mutex<Option<Vec<(String, usize, usize)>>>,
}
impl LlmProvider for PendingLoan {
    fn name(&self) -> &str {
        "pending-loan"
    }
    fn route_order(&self) -> Vec<String> {
        vec![self.name().into()]
    }
    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        *self.seen.lock().unwrap() = Some(
            request
                .messages
                .iter()
                .map(|m| {
                    (
                        m.content.clone(),
                        m.user_image_blocks.len(),
                        m.estimated_tokens(),
                    )
                })
                .collect(),
        );
        Box::pin(async move {
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn run24_cancel_nested_guidance_images_restores_content_and_token_cache() {
    use crate::application::agent_turn::ports::AgentLoop;
    use crate::domain::conversation::value_objects::message::{Message, UserImageBlock};
    use crate::domain::tool_policy::value_objects::tool::{
        ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyRequest,
    };
    if !isolated(
        concat!(
            module_path!(),
            "::run24_cancel_nested_guidance_images_restores_content_and_token_cache"
        ),
        &["unset", "1"],
    ) {
        return;
    }
    let composed = crate::interface::cli::controllers::active_runtime_2708_tests::compose(true);
    let canonical = crate::interface::cli::controllers::shared::build_agent_system_prompt(
        Some("AGENTS cancellation"),
        None,
        false,
        &composed.snippets,
    );
    let provider = Arc::new(PendingLoan {
        entered: Notify::new(),
        seen: Mutex::new(None),
    });
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(composed.registry),
        model: "stub".into(),
        max_tokens: 32,
        temperature: 0.0,
        retention: None,
        session_key: KEY.into(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    });
    let result = agent
        .request_tool_policy(
            ToolPolicyRequest::patch(vec![ToolPolicyMutation::disable(
                "declare_turn_outcome",
                "actual policy makes guidance projection differ",
            )]),
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .unwrap();
    assert_eq!(result.results.len(), 1);
    assert!(!agent.is_tool_model_visible("declare_turn_outcome"));
    let mut user = Message::user("inspect image");
    user.user_image_blocks
        .push(UserImageBlock::sample(quecto_image::ImageMime::Png));
    let mut messages = vec![Message::system(canonical.clone()), user];
    agent.bind_canonical_prompt(&messages, &canonical);
    let original: Vec<_> = messages
        .iter()
        .map(|m| {
            (
                m.id(),
                m.content.clone(),
                m.user_image_blocks.clone(),
                m.estimated_tokens(),
            )
        })
        .collect();
    {
        let run = agent.process(&mut messages);
        tokio::pin!(run);
        tokio::select! {
            _ = provider.entered.notified() => {},
            _ = &mut run => panic!("provider unexpectedly completed"),
            _ = tokio::time::sleep(STEP) => panic!("provider never reached actual nested projection"),
        }
        // Dropping the actual pending process/request future exercises both nested destructors.
    }
    let seen = provider.seen.lock().unwrap().clone().unwrap();
    assert!(
        !seen[0].0.contains(DECLARE_GUIDANCE),
        "actual provider gets changed guidance view"
    );
    assert_eq!(
        seen[1].1, 0,
        "actual nested image projection withholds image"
    );
    assert!(seen[1].0.contains("[image not sent:"));
    assert_ne!(
        seen[0].2, original[0].3,
        "projected cache recomputed after removing guidance"
    );
    for (message, (id, text, images, tokens)) in messages.iter().zip(original) {
        assert_eq!(message.id(), id);
        assert_eq!(message.content, text, "drop restores canonical content");
        assert_eq!(message.user_image_blocks, images, "drop restores images");
        assert_eq!(
            message.estimated_tokens(),
            tokens,
            "drop invalidates projected token cache"
        );
        assert_eq!(
            message.estimated_tokens(),
            message.clone().estimated_tokens(),
            "cache equals fresh canonical estimate"
        );
    }
    println!(
        "RUN24 nested cancellation restored {} messages, provider request=1",
        messages.len()
    );
}
