//! Resume/prompt persistence dispatch regression tests split from
//! `uds_dispatch_cov_tests.rs` to keep coverage files below the line-count gate.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::cov_tests::Fixture;
use super::{dispatch_command, handle_resume_session, persist_current_session};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::context_pruning::build_manifest_text;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role, ToolCall};
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::domain::session::{Session, SessionStore};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::interface::cli::protocol::AgentCommand;
use crate::interface::cli::uds::{inject_system_prompt, remove_injected_system_prompt};
use crate::interface::cli::uds_session::{HISTORY_PAGE_SIZE, messages_page_json};

fn prompt(message: &str) -> AgentCommand {
    AgentCommand::Prompt {
        id: None,
        message: message.into(),
        streaming_behavior: None,
    }
}

fn durable_contents(messages: &[Message]) -> Vec<&str> {
    messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| m.content.as_str())
        .collect()
}

#[tokio::test]
async fn prompt_persists_user_message_before_assistant_reply() {
    // Regression: if a TUI/session is closed or interrupted before the provider
    // returns an assistant message, the first user message must still be
    // durable and visible after /resume.
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        let prompt = crate::domain::message::Message::user("first only");
        super::persist_user_prompt_before_run(&mut ctx, &prompt)
            .await
            .unwrap();
        ctx.messages.push(prompt);
    }

    let loaded = fx.store.load("cli:test").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].role, crate::domain::message::Role::User);
    assert_eq!(loaded.messages[0].content, "first only");

    fx.store
        .save(&Session {
            key: Session::build_key("cli", "saved-one"),
            messages: loaded.messages.clone(),
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();
    {
        let mut ctx = fx.ctx();
        assert!(
            !handle_resume_session(&mut ctx, Some("rs"), "resume_session", "saved-one".into())
                .await
        );
    }
    assert_eq!(fx.messages.len(), 1);
    assert_eq!(fx.messages[0].content, "first only");
}

#[tokio::test]
async fn dispatch_unknown_history_cursor_is_rejected() {
    let mut fx = Fixture::new();
    fx.messages = vec![crate::domain::message::Message::user("newest")];
    assert!(
        !dispatch_command(
            AgentCommand::GetMessages {
                id: Some("stale-page".into()),
                count: None,
                before: Some("unknown-message-id".into()),
                agent_id: None,
            },
            &mut fx.ctx(),
        )
        .await
    );
}

#[tokio::test]
async fn dispatch_agent_targeted_tail_without_registry_emits_error() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::GetMessagesTail {
        id: Some("inspector-tail:worker".into()),
        count: 5,
        agent_id: Some("worker".into()),
    };
    // subagent_registry is None: the early intercept still handles it.
    assert!(!dispatch_command(cmd, &mut fx.ctx()).await);
}

#[tokio::test]
async fn refresh_conversation_snapshot_clones_current_messages() {
    let mut fx = Fixture::new();
    fx.messages = vec![
        crate::domain::message::Message::user("hello"),
        crate::domain::message::Message::assistant("hi", vec![]),
    ];
    let ctx = fx.ctx();
    assert!(
        ctx.conversation_snapshot.read().await.messages.is_empty(),
        "starts empty"
    );
    crate::interface::cli::uds_snapshots::refresh_conversation_snapshot(&ctx).await;
    let snap = ctx.conversation_snapshot.read().await;
    assert_eq!(snap.messages.len(), 2, "snapshot mirrors current messages");
    crate::interface::cli::uds_snapshots::refresh_state_snapshot(&ctx).await;
    assert_eq!(ctx.state_snapshot.read().await.message_count, 2);
}

/// #1322: multi-turn clean-path persist through real `handle_prompt` must keep
/// the full durable history when a non-empty system prompt is injected. Empty
/// system is a control that stays green even with the live/durable skew bug.
#[tokio::test]
async fn multi_turn_persist_resume_restores_full_history_with_system_prompt() {
    const TURNS: usize = 5;
    for system in ["be helpful", ""] {
        let mut fx = Fixture::new().with_system_prompt(system);
        let users: Vec<String> = (0..TURNS).map(|i| format!("user-{i}")).collect();
        // One long-lived ctx so the durable watermark survives across turns
        // (Fixture copies the watermark by value into each `ctx()`).
        {
            let mut ctx = fx.ctx();
            inject_system_prompt(ctx.messages, ctx.system_prompt);
            for user in &users {
                assert!(
                    !dispatch_command(prompt(user), &mut ctx).await,
                    "prompt dispatch should keep the connection open"
                );
            }

            let loaded = ctx
                .session_store
                .load(ctx.session_key)
                .await
                .unwrap()
                .expect("session must be on disk after multi-turn prompts");
            let durable = durable_contents(&loaded.messages);
            let expected: Vec<&str> = users
                .iter()
                .flat_map(|u| [u.as_str(), "stub response"])
                .collect();
            assert_eq!(
                durable, expected,
                "durable load must keep all {TURNS} turns with system_prompt={system:?}; \
                 truncated tip u0,a0,u1 is the #1322 failure mode"
            );
            assert!(
                durable.len() > 3,
                "history must not end at the second user when later turns exist"
            );
            assert_ne!(
                durable.last().copied(),
                Some(users[1].as_str()),
                "second user turn must not be EOF when later assistants exist"
            );

            // Resume into the same live session key via a saved alias, then
            // check tip/content (messageCount is post-inject live len).
            let resume_name = if system.is_empty() {
                "saved-empty-sys"
            } else {
                "saved-with-sys"
            };
            ctx.session_store
                .save(&Session {
                    key: Session::build_key("cli", resume_name),
                    messages: loaded.messages.clone(),
                    workflow_run: None,
                    subagent_roster: Vec::new(),
                })
                .await
                .unwrap();
            assert!(
                !handle_resume_session(&mut ctx, Some("rs"), "resume_session", resume_name.into(),)
                    .await
            );

            let resumed_durable = durable_contents(ctx.messages);
            assert_eq!(
                resumed_durable, expected,
                "resume must restore full durable content (modulo re-injected system)"
            );
            let tip = ctx
                .messages
                .iter()
                .rev()
                .find(|m| m.role != Role::System)
                .expect("resumed tip");
            assert_eq!(tip.role, Role::Assistant);
            assert_eq!(tip.content, "stub response");
            assert_eq!(
                resumed_durable[resumed_durable.len() - 2],
                users[TURNS - 1].as_str(),
                "last user turn must be present before the final assistant"
            );

            let page = messages_page_json(ctx.messages, HISTORY_PAGE_SIZE, None);
            let page_msgs = page["messages"].as_array().expect("messages array");
            let page_tip = page_msgs.last().expect("newest page has a tip");
            assert_eq!(page_tip["role"], "assistant");
            assert_eq!(page_tip["content"], "stub response");
        }
    }
}

/// #1322: `last_persisted_message_index` is a durable (system-stripped) coordinate.
/// After pre-persist it must equal durable len, never live_len+1 while system is
/// injected and the user is not yet pushed into live history.
#[tokio::test]
async fn persist_watermark_matches_durable_len_not_live_len_plus_one() {
    let system = "be helpful";
    let mut fx = Fixture::new().with_system_prompt(system);
    {
        let mut ctx = fx.ctx();
        inject_system_prompt(ctx.messages, ctx.system_prompt);
        // Seed one completed turn so live includes system and durable is non-empty.
        assert!(!dispatch_command(prompt("user-0"), &mut ctx).await);
        let durable_after_turn = {
            let mut stripped = ctx.messages.clone();
            remove_injected_system_prompt(&mut stripped, ctx.system_prompt);
            stripped.len()
        };
        assert_eq!(
            ctx.last_persisted_message_index, durable_after_turn,
            "end-of-turn watermark must match stripped durable len"
        );

        // Mid-turn pre-persist: production must leave watermark in durable space.
        let next = Message::user("user-1");
        let live_len_before_push = ctx.messages.len();
        assert!(
            live_len_before_push > durable_after_turn,
            "scenario setup: injected system must create live/durable skew"
        );
        super::persist_user_prompt_before_run(&mut ctx, &next)
            .await
            .unwrap();
        let expected_durable_wm = durable_after_turn + 1; // prior durable + new user
        assert_eq!(
            ctx.last_persisted_message_index, expected_durable_wm,
            "persist_user_prompt_before_run must set durable watermark"
        );
        assert_ne!(
            ctx.last_persisted_message_index,
            live_len_before_push + 1,
            "watermark must not use live_len+1 while system is injected and user \
             is not yet pushed into live history"
        );

        // Drive the real handle_prompt path (includes the post-before-run
        // watermark assign under test) and require the durable invariant holds
        // after the full turn — wm == load len == stripped live len.
        assert!(!dispatch_command(prompt("user-1"), &mut ctx).await);
        let loaded = ctx
            .session_store
            .load(ctx.session_key)
            .await
            .unwrap()
            .expect("session on disk");
        let mut stripped_live = ctx.messages.clone();
        remove_injected_system_prompt(&mut stripped_live, ctx.system_prompt);
        assert_eq!(
            ctx.last_persisted_message_index,
            stripped_live.len(),
            "after handle_prompt, watermark must equal stripped durable len"
        );
        assert_eq!(
            ctx.last_persisted_message_index,
            loaded.messages.len(),
            "watermark must agree with what load() reconstructs (no doomed append freeze)"
        );
        assert_eq!(
            durable_contents(&loaded.messages),
            ["user-0", "stub response", "user-1", "stub response"],
            "clean-path turn 2 must append assistant; freeze at u0,a0,u1 is #1322"
        );

        // End-of-turn persist is idempotent and keeps the durable coordinate.
        persist_current_session(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.last_persisted_message_index,
            stripped_live.len(),
            "persist_current_session must keep durable watermark"
        );
    }

    // Empty-system control: live == durable, so len()+1 is numerically less
    // dangerous — still require watermark == load len after two turns.
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(prompt("user-0"), &mut ctx).await);
        assert!(!dispatch_command(prompt("user-1"), &mut ctx).await);
        let loaded = ctx
            .session_store
            .load(ctx.session_key)
            .await
            .unwrap()
            .expect("session on disk");
        assert_eq!(ctx.last_persisted_message_index, loaded.messages.len());
        assert_eq!(
            durable_contents(&loaded.messages),
            ["user-0", "stub response", "user-1", "stub response"]
        );
    }
}

/// Provider returning a scripted FIFO of responses (local copy; 1072 helpers
/// are module-private).
#[derive(Debug)]
struct ScriptedProvider {
    responses: Mutex<Vec<LlmResponse>>,
}

impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        let response = self.responses.lock().unwrap().remove(0);
        Box::pin(async move { Ok(response) })
    }
}

struct FixedTool {
    def: ToolDefinition,
    output: String,
}

impl Tool for FixedTool {
    fn definition(&self) -> ToolDefinition {
        self.def.clone()
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        let output = self.output.clone();
        Box::pin(async move {
            Ok(ToolResult {
                content: output,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

fn tool_call_response(name: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{name}"),
            name: name.to_string(),
            arguments: "{}".to_string(),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

/// Reconstruct messages while asserting every append's `start_index` equals
/// the reconstructed length *before* apply. Returns reconstructed messages.
///
/// After `min_appends` is observed (callers pass the expected floor for the
/// turn under test), fails if the file collapsed to a single snapshot — that
/// hides `start_index` bugs via dirty-path compaction.
fn assert_jsonl_start_index_chain(raw: &str, min_appends: usize) -> Vec<serde_json::Value> {
    let mut reconstructed: Vec<serde_json::Value> = Vec::new();
    let mut cur = 0usize;
    let mut append_count = 0usize;
    let mut saw_snapshot = false;

    for (line_no, line) in raw
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let record: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|err| panic!("line {}: invalid JSON ({err}): {line}", line_no + 1));
        let ty = record
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("line {}: missing type: {line}", line_no + 1));
        match ty {
            "snapshot" => {
                let messages = record
                    .get("messages")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!("line {}: snapshot missing messages: {line}", line_no + 1)
                    });
                reconstructed = messages;
                cur = reconstructed.len();
                saw_snapshot = true;
                // A later compacting snapshot resets the chain; append_count
                // stays cumulative so multi-turn clean path can still prove
                // appends happened earlier — but a sole final snapshot after
                // multi-turn is rejected via min_appends below.
            }
            "append" => {
                assert!(
                    saw_snapshot,
                    "line {}: append before any snapshot: {line}",
                    line_no + 1
                );
                let start_index = record
                    .get("start_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| {
                        panic!("line {}: append missing start_index: {line}", line_no + 1)
                    }) as usize;
                assert_eq!(
                    start_index,
                    cur,
                    "line {}: start_index {start_index} != reconstructed len {cur}: {line}",
                    line_no + 1
                );
                let added = record
                    .get("messages")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!("line {}: append missing messages: {line}", line_no + 1)
                    });
                cur += added.len();
                reconstructed.extend(added);
                append_count += 1;
            }
            other => panic!(
                "line {}: unknown record type {other:?}: {line}",
                line_no + 1
            ),
        }
    }

    assert!(
        saw_snapshot,
        "JSONL must contain at least one snapshot record"
    );
    assert!(
        append_count >= min_appends,
        "anti-false-green: expected ≥{min_appends} append records after multi-turn clean \
         path (got {append_count}); a single compacted snapshot hides start_index bugs. \
         raw=\n{raw}"
    );
    reconstructed
}

/// #1326: multi-turn clean-path persist (tools + durable manifest) must keep
/// the on-disk JSONL `start_index` chain contiguous. Content/watermark-only
/// coverage in #1324 does not read raw append coordinates.
#[tokio::test]
async fn multi_turn_jsonl_start_index_chain_contiguous_with_tools_and_manifest() {
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(Arc::new(FixedTool {
        def: ToolDefinition {
            name: "echo_tool".to_string().into(),
            description: "fixed tool for start_index continuity".to_string().into(),
            parameters_schema: r#"{"type":"object"}"#.to_string().into(),
        },
        output: "tool-result-payload".to_string(),
    }));

    // Turn 0: plain text. Turn 1: tool-call then final text. Turn 2: plain text.
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(vec![
            text_response("assistant-0"),
            tool_call_response("echo_tool"),
            text_response("assistant-1-final"),
            text_response("assistant-2"),
        ]),
    });

    let mut fx = Fixture::new().with_system_prompt("be helpful");
    fx.agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });

    // Durable index 0: non-stripped spill manifest (survives inject/strip).
    let mut manifest = Message::system(build_manifest_text());
    manifest.is_manifest = true;
    manifest.is_pinned = true;
    fx.messages.push(manifest);

    let users = ["user-0", "user-1-tools", "user-2"];
    let session_path = {
        let mut ctx = fx.ctx();
        let path = ctx.base_dir.join("sessions/cli_test.json");
        inject_system_prompt(ctx.messages, ctx.system_prompt);
        // Live: [injected system, manifest, …]; durable-on-disk: [manifest, …].
        assert_eq!(ctx.messages.len(), 2);
        assert!(!ctx.messages[0].is_manifest);
        assert!(ctx.messages[1].is_manifest);

        for (turn, user) in users.iter().enumerate() {
            assert!(
                !dispatch_command(prompt(user), &mut ctx).await,
                "prompt dispatch should keep the connection open (turn {turn})"
            );
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("session JSONL missing after turn {turn}: {err}"));
            // After ≥2 turns the clean path must have produced append records;
            // require at least one append once turn index ≥ 1.
            let min_appends = if turn >= 1 { 1 } else { 0 };
            let chain = assert_jsonl_start_index_chain(&raw, min_appends);
            assert!(
                !chain.is_empty(),
                "reconstructed chain empty after turn {turn}"
            );
            assert_eq!(
                chain[0].get("is_manifest").and_then(|v| v.as_bool()),
                Some(true),
                "durable index 0 must remain the manifest after turn {turn}"
            );
            // Injected system prompt must not appear on disk.
            assert!(
                chain.iter().all(|m| {
                    m.get("is_manifest")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                        || m.get("role").and_then(|v| v.as_str()) != Some("system")
                        || m.get("content").and_then(|v| v.as_str()) != Some("be helpful")
                }),
                "injected system must not be durable after turn {turn}"
            );
        }

        path
    };

    // End-state: load agrees with raw chain; tool turn landed; tip matches.
    let raw = std::fs::read_to_string(&session_path).expect("final session JSONL");
    let chain = assert_jsonl_start_index_chain(&raw, 1);
    let loaded = fx
        .store
        .load("cli:test")
        .await
        .unwrap()
        .expect("session must be on disk after multi-turn prompts");
    assert_eq!(
        loaded.messages.len(),
        chain.len(),
        "load() len must equal reconstructed JSONL chain len"
    );
    assert!(
        loaded.messages[0].is_manifest,
        "loaded durable[0] must be the seeded manifest"
    );
    assert!(
        loaded
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.content == "tool-result-payload"),
        "tool result must be durable after the tool-bearing turn"
    );
    assert!(
        loaded.messages.iter().any(|m| {
            m.role == Role::Assistant
                && !m.tool_calls.is_empty()
                && m.tool_calls[0].name == "echo_tool"
        }),
        "tool-call assistant must be durable after the tool-bearing turn"
    );

    let tip = loaded
        .messages
        .iter()
        .rev()
        .find(|m| m.role != Role::System)
        .expect("loaded tip");
    assert_eq!(tip.role, Role::Assistant);
    assert_eq!(tip.content, "assistant-2");

    // Optional resume path (mirrors #1324): full chain survives resume.
    fx.store
        .save(&Session {
            key: Session::build_key("cli", "saved-jsonl-chain"),
            messages: loaded.messages.clone(),
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();
    {
        let mut ctx = fx.ctx();
        assert!(
            !handle_resume_session(
                &mut ctx,
                Some("rs"),
                "resume_session",
                "saved-jsonl-chain".into(),
            )
            .await
        );
        assert!(
            ctx.messages.iter().any(|m| m.is_manifest),
            "resume must keep the durable manifest"
        );
        let resumed_tip = ctx
            .messages
            .iter()
            .rev()
            .find(|m| m.role != Role::System)
            .expect("resumed tip");
        assert_eq!(resumed_tip.role, Role::Assistant);
        assert_eq!(resumed_tip.content, "assistant-2");
        assert!(
            ctx.messages
                .iter()
                .any(|m| m.role == Role::Tool && m.content == "tool-result-payload"),
            "resume must keep the tool result"
        );
    }
}
