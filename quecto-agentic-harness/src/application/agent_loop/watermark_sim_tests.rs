//! #2403: the watermark context never edits history between cuts. A seeded
//! synthetic session runs through the real agent loop and context code;
//! every request is serialized as the Codex provider sends it (its request
//! body builder: instructions, tools, then each input item in order), and
//! each must extend the previous one byte for byte, except the request
//! right after a cut. The head (instructions, tools, the brief) is the
//! same on every request. (The idea and the session come from the spike
//! `spike/watermark-context`; this is its mechanism test, without the
//! cache-rate table.)

use super::ctx_mgmt_tests::MemSpillStore;
use crate::application::agent_loop::tests::MockRegistry;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::tools::ports::Tool;
use crate::domain::conversation::watermark::Watermark;
use crate::domain::conversation::{ContextMode, UserKind};
use crate::domain::error::DomainError;
use crate::domain::large_result_collapse::LargeResultCollapse;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::domain::turn_origin::prompt;
use crate::infrastructure::providers::codex::CodexProvider;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

const SEED: u64 = 0x2403_5eed;

/// splitmix64: a fixed seed gives the same session every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn between(&mut self, low: usize, high: usize) -> usize {
        low + (self.next() % (high - low + 1) as u64) as usize
    }
}

const WORDS: &[&str] = &[
    "the", "context", "result", "module", "function", "returns", "value", "error", "handler",
    "request", "session", "cache", "prefix", "message", "tool", "call", "file", "line",
];

/// About `tokens` estimated tokens of prose, the same for the same seed.
fn prose(seed: u64, tokens: usize) -> String {
    let mut rng = Rng(seed);
    let mut text = String::new();
    while Message::estimate_tokens(&text) < tokens {
        for _ in 0..12 {
            text.push_str(WORDS[rng.between(0, WORDS.len() - 1)]);
            text.push(' ');
        }
        text.push('\n');
    }
    text
}

/// One turn of the script: its prompt, its calls' result sizes, its answer.
#[derive(Clone, Debug)]
struct Turn {
    prompt_tokens: usize,
    calls: Vec<usize>,
    answer_tokens: usize,
}

/// Turns until `requests` requests are made (one per call, one per answer).
fn script(requests: usize) -> Vec<Turn> {
    let mut rng = Rng(SEED);
    let mut turns = Vec::new();
    let mut made = 0;
    while made < requests {
        let mut calls: Vec<usize> = (0..rng.between(1, 6))
            .map(|_| rng.between(200, 6_000))
            .collect();
        calls.truncate(requests - made - 1);
        made += calls.len() + 1;
        turns.push(Turn {
            prompt_tokens: rng.between(30, 300),
            calls,
            answer_tokens: rng.between(100, 600),
        });
    }
    turns
}

/// One request as the provider saw it.
#[derive(Debug, Clone)]
struct Observed {
    /// The previous request is a byte prefix of this one.
    extends: bool,
    /// The head (instructions, tools, the first input item) is the first
    /// request's.
    head_kept: bool,
    /// The stub in this request, if any: a new one means a cut came first.
    stub: Option<uuid::Uuid>,
}

/// The provider: plays the script and serializes every request.
#[derive(Debug)]
struct SimProvider {
    script: Vec<Turn>,
    cursor: Mutex<(usize, usize)>,
    previous: Mutex<String>,
    head: Mutex<Option<String>>,
    observed: Mutex<Vec<Observed>>,
}

impl SimProvider {
    fn new(script: Vec<Turn>) -> Self {
        Self {
            script,
            cursor: Mutex::new((0, 0)),
            previous: Mutex::new(String::new()),
            head: Mutex::new(None),
            observed: Mutex::new(Vec::new()),
        }
    }

    fn observe(&self, request: &ChatRequest<'_>) {
        let body = CodexProvider::build_request_body_public_api_key(request);
        let mut wire = format!("I:{}\nT:{}\n", body["instructions"], body["tools"]);
        let input = body["input"].as_array().cloned().unwrap_or_default();
        let head_len = wire.len() + input.first().map_or(0, |item| item.to_string().len() + 1);
        for item in &input {
            wire.push_str(&item.to_string());
            wire.push('\n');
        }
        let mut head = self.head.lock().unwrap();
        let head = head.get_or_insert_with(|| wire[..head_len].to_string());
        let mut previous = self.previous.lock().unwrap();
        self.observed.lock().unwrap().push(Observed {
            extends: wire.starts_with(previous.as_str()),
            head_kept: wire.starts_with(head.as_str()),
            stub: request
                .messages
                .iter()
                .find(|m| m.user_kind == UserKind::ArchiveStub)
                .map(Message::id),
        });
        *previous = wire;
    }
}

impl LlmProvider for SimProvider {
    fn name(&self) -> &str {
        "watermark-sim"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        self.observe(&request);
        let mut cursor = self.cursor.lock().unwrap();
        let (turn, call) = *cursor;
        let script = &self.script[turn];
        let (content, tool_calls) = if call < script.calls.len() {
            *cursor = (turn, call + 1);
            let call = ToolCall {
                id: format!("call-{turn}-{call}"),
                name: "read".to_string(),
                arguments: format!(
                    r#"{{"path":"src/m{turn}_{call}.rs","tokens":{}}}"#,
                    script.calls[call]
                ),
            };
            (None, vec![call])
        } else {
            *cursor = (turn + 1, 0);
            (
                Some(prose(SEED ^ turn as u64, script.answer_tokens)),
                vec![],
            )
        };
        let response = LlmResponse {
            content,
            tool_calls,
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        };
        Box::pin(async move { Ok(response) })
    }
}

/// `read`: returns as many tokens as its call asks for.
struct SimRead;

impl Tool for SimRead {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".into(),
            description: "Read a file.".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args: serde_json::Value = serde_json::from_str(arguments).unwrap();
        let tokens = args["tokens"].as_u64().unwrap() as usize;
        let seed = arguments
            .bytes()
            .fold(SEED, |h, b| h.rotate_left(5) ^ u64::from(b));
        let content = prose(seed, tokens);
        Box::pin(async move {
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

async fn simulate(high: usize, low: usize, requests: usize) -> Vec<Observed> {
    let script = script(requests);
    let provider = Arc::new(SimProvider::new(script.clone()));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(SimRead));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(registry),
        model: "gpt-sim".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        session_key: "watermark-sim".to_string(),
        // Dials that edit history on nearly every request in the default
        // mode: none may run in watermark mode.
        context_collapse_after_tool_calls: 3,
        max_context_tokens: 1_000_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 1,
        context_collapse_after_messages: 4,
        large_result_collapse: LargeResultCollapse {
            over_tokens: 1_000,
            after_turns: 1,
        },
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    agent.set_context_mode(ContextMode::Watermark(Watermark::new(high, low).unwrap()));
    let mut messages = vec![Message::system(prose(1, 2_000))];
    for (n, turn) in script.iter().enumerate() {
        messages.push(prompt(prose(SEED ^ (n as u64) << 8, turn.prompt_tokens)));
        agent.run_loop(&mut messages).await.unwrap();
    }
    let observed = provider.observed.lock().unwrap().clone();
    assert_eq!(observed.len(), requests, "the script ran to its end");
    observed
}

#[tokio::test]
async fn every_request_extends_the_previous_one_except_right_after_a_cut() {
    let observed = simulate(30_000, 12_000, 150).await;
    let mut cuts = 0;
    for (n, pair) in observed.windows(2).enumerate() {
        let (previous, request) = (&pair[0], &pair[1]);
        let cut = request.stub.is_some() && request.stub != previous.stub;
        cuts += usize::from(cut);
        assert_eq!(
            request.extends,
            !cut,
            "request {}: a prefix extension unless a cut came first",
            n + 1
        );
    }
    assert!(cuts >= 2, "several cuts: {cuts}");
    assert!(
        observed.iter().all(|request| request.head_kept),
        "the head never changes"
    );
}
