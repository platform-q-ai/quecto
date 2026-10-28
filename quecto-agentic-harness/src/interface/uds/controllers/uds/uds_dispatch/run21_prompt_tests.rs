//! Real production composition and durable session roundtrip, round2 HIGH.
use super::fixture_tests::Fixture;
use super::{dispatch_command, handle_resume_session};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::sessions::ports::SessionStore;
use crate::domain::conversation::services::conversation_view::{
    inject_system_prompt, user_visible_messages,
};
use crate::domain::conversation::value_objects::message::{LlmResponse, Role};
use crate::domain::sessions::entities::{session::Session, session_identity::SessionIdentity};
use crate::domain::shared::error::DomainError;
use crate::interface::cli::controllers::active_runtime_2708_tests::{compose, isolated};
use crate::interface::cli::controllers::shared::build_agent_system_prompt;
use crate::interface::uds::controllers::uds::dispatch_test_env::make_dispatch_test_agent_with_registry;
use std::sync::Arc;

#[derive(Debug)]
struct FiniteText;
impl LlmProvider for FiniteText {
    fn name(&self) -> &str {
        "run21-composed"
    }
    fn route_order(&self) -> Vec<String> {
        vec![self.name().into()]
    }
    fn chat(
        &self,
        _: ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
    > {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("No declaration.".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

fn assert_extension_length(prompt: &str, snippets: &str) {
    assert!(
        prompt.contains(&format!(
            "<extensions>\nContent length: {} bytes\n\n{snippets}\n\n</extensions>",
            snippets.len()
        )),
        "actual composition retains accurate extensions framing"
    );
}

#[tokio::test]
async fn run21_real_composed_prompt_save_resume_roundtrip() {
    if !isolated(
        concat!(
            module_path!(),
            "::run21_real_composed_prompt_save_resume_roundtrip"
        ),
        &["unset", "1"],
    ) {
        return;
    }
    let composed = compose(true);
    let agents_dir = tempfile::tempdir().unwrap();
    let agents_path = agents_dir.path().join("AGENTS.md");
    std::fs::write(&agents_path, "AGENTS old").unwrap();
    let old_agents = crate::infrastructure::config::agents_instructions::load_agents_instructions(
        agents_dir.path(),
    )
    .unwrap()
    .unwrap();
    let prompt = build_agent_system_prompt(
        Some(&old_agents),
        Some("config old"),
        false,
        &composed.snippets,
    );
    assert_extension_length(&prompt, &composed.snippets);
    let mut fx = Fixture::new().with_system_prompt(prompt.clone());
    fx.set_agent(make_dispatch_test_agent_with_registry(
        Arc::new(FiniteText),
        composed.registry,
    ));
    fx.session.bind_parent_outcome(composed.runtime);
    inject_system_prompt(&mut fx.messages, &prompt);
    let command = serde_json::from_value(
        serde_json::json!({"type":"prompt", "id":"composed-first", "message":"Work"}),
    )
    .unwrap();
    dispatch_command(command, &mut fx.ctx()).await;
    let live_head = fx.messages.first().unwrap().content.clone();
    let visible = user_visible_messages(&fx.messages, &prompt);
    let saved = fx
        .store
        .load(&SessionIdentity::from_persisted_key("cli:test"))
        .await
        .unwrap()
        .unwrap();
    // Complete a real save/load/resume before checking all head invariants.
    let target = SessionIdentity::named_cli("round21-saved").unwrap();
    fx.store
        .save(&Session {
            key: target,
            messages: saved.messages.clone(),
            workflow_run: None,
            subagent_roster: vec![],
        })
        .await
        .unwrap();
    std::fs::write(&agents_path, "AGENTS refreshed").unwrap();
    let refreshed_agents =
        crate::infrastructure::config::agents_instructions::load_agents_instructions(
            agents_dir.path(),
        )
        .unwrap()
        .unwrap();
    let refreshed = build_agent_system_prompt(
        Some(&refreshed_agents),
        Some("config refreshed"),
        false,
        &composed.snippets,
    );
    // Reconstruct a fresh process view from durable, head-free history.
    fx.messages = saved.messages.clone();
    fx = fx.with_system_prompt(refreshed.clone());
    inject_system_prompt(&mut fx.messages, &refreshed);
    let (tx, mut rx) = tokio::sync::broadcast::channel(32);
    let mut ctx = fx.ctx();
    ctx.broadcast_tx = Some(tx);
    handle_resume_session(
        &mut ctx,
        Some("resume-real"),
        "resume_session",
        "round21-saved".into(),
    )
    .await;
    let frames: Vec<serde_json::Value> = std::iter::from_fn(|| rx.try_recv().ok())
        .map(|line| serde_json::from_str(line.trim()).unwrap())
        .collect();
    assert!(
        frames.iter().any(|frame| frame["type"] == "response"
            && frame["id"] == "resume-real"
            && frame["success"] == true),
        "actual resume must succeed: {frames:?}"
    );
    assert_eq!(
        fx.current_session_key(),
        SessionIdentity::named_cli("round21-saved")
            .unwrap()
            .runtime_key()
    );
    let resumed_head = fx.messages.first().unwrap().content.clone();
    assert_eq!(
        live_head, prompt,
        "enabled real composed head remains canonical, not moved outside extensions"
    );
    assert!(
        visible.iter().all(|m| m.role != Role::System),
        "live transcript excludes composed prompt"
    );
    assert!(
        saved.messages.iter().all(|m| m.role != Role::System),
        "real save never persists modified composed head"
    );
    assert_eq!(
        resumed_head, refreshed,
        "resume picks up refreshed AGENTS and config composition"
    );
    assert_extension_length(&resumed_head, &composed.snippets);
}

#[tokio::test]
async fn run21_no_head_does_not_inject_bare_guidance() {
    if !isolated(
        concat!(
            module_path!(),
            "::run21_no_head_does_not_inject_bare_guidance"
        ),
        &["unset"],
    ) {
        return;
    }
    let composed = compose(true);
    let mut fx = Fixture::new();
    fx.set_agent(make_dispatch_test_agent_with_registry(
        Arc::new(FiniteText),
        composed.registry,
    ));
    fx.session.bind_parent_outcome(composed.runtime);
    dispatch_command(
        serde_json::from_value(
            serde_json::json!({"type":"prompt", "id":"no-head", "message":"Work"}),
        )
        .unwrap(),
        &mut fx.ctx(),
    )
    .await;
    assert!(
        fx.messages.iter().all(|m| m.role != Role::System),
        "no synthetic bare guidance prompt"
    );
}
