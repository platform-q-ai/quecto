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
use crate::domain::conversation::UserKind;
use crate::domain::conversation::watermark::Watermark;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, StopReason, ToolCall};
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
    /// Every this many turns, the turn's first reply is cut off before
    /// anything visible (#2124): the loop asks again with feedback.
    cut_off_every: Option<usize>,
    cut_off: Mutex<std::collections::BTreeSet<usize>>,
}

impl SimProvider {
    fn new(script: Vec<Turn>, cut_off_every: Option<usize>) -> Self {
        Self {
            script,
            cursor: Mutex::new((0, 0)),
            previous: Mutex::new(String::new()),
            head: Mutex::new(None),
            observed: Mutex::new(Vec::new()),
            cut_off_every,
            cut_off: Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    /// Whether this request, the first of turn `turn`, is cut off.
    fn cuts_off(&self, turn: usize, call: usize) -> bool {
        let due = self
            .cut_off_every
            .is_some_and(|every| call == 0 && turn % every == every - 1);
        due && self.cut_off.lock().unwrap().insert(turn)
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
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
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
        if self.cuts_off(turn, call) {
            let response = LlmResponse {
                content: None,
                tool_calls: vec![],
                usage: None,
                stop_reason: Some(StopReason::MaxTokens),
                thinking_blocks: vec![],
            };
            return Box::pin(async move { Ok(response) });
        }
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

/// The loop the simulation runs, at `marks` (`None`: the owner's) under
/// `max_context_tokens`.
fn sim_agent(
    provider: Arc<SimProvider>,
    marks: Option<(usize, usize)>,
    max_context_tokens: usize,
) -> AgentLoopImpl {
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(SimRead));
    let marks = marks.map_or_else(Watermark::default, |(high, low)| {
        Watermark::new(high, low).unwrap()
    });
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "gpt-sim".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        session_key: "watermark-sim".to_string(),
        max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 1,
        context_marks: marks,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
}

async fn simulate(
    high: usize,
    low: usize,
    requests: usize,
    cut_off_every: Option<usize>,
) -> Vec<Observed> {
    simulate_on(Some((high, low)), 1_000_000, requests, cut_off_every).await
}

async fn simulate_on(
    marks: Option<(usize, usize)>,
    max_context_tokens: usize,
    requests: usize,
    cut_off_every: Option<usize>,
) -> Vec<Observed> {
    let script = script(requests);
    let provider = Arc::new(SimProvider::new(script.clone(), cut_off_every));
    let mut agent = sim_agent(provider.clone(), marks, max_context_tokens);
    let mut messages = vec![Message::system(prose(1, 2_000))];
    for (n, turn) in script.iter().enumerate() {
        messages.push(prompt(prose(SEED ^ (n as u64) << 8, turn.prompt_tokens)));
        agent.run_loop(&mut messages).await.unwrap();
    }
    let observed = provider.observed.lock().unwrap().clone();
    let cut_offs = provider.cut_off.lock().unwrap().len();
    assert_eq!(
        observed.len(),
        requests + cut_offs,
        "the script ran to its end"
    );
    observed
}

/// Every request extends the previous one, byte for byte, except a
/// request right after a cut.
fn assert_append_only(observed: &[Observed]) {
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

#[tokio::test]
async fn every_request_extends_the_previous_one_except_right_after_a_cut() {
    assert_append_only(&simulate(30_000, 12_000, 150, None).await);
}

/// Review L1: a reply cut off before anything visible is asked again with
/// feedback; the feedback is appended, never merged into the prompt already
/// sent, so no request but one after a cut breaks the prefix.
#[tokio::test]
async fn feedback_after_a_cut_off_reply_keeps_every_request_an_extension() {
    assert_append_only(&simulate(30_000, 12_000, 150, Some(5)).await);
}

/// #2414: watermark is the only context mode. Over a long session, a loop
/// built by default (the owner's marks scaled to a 40k ceiling) never edits
/// an earlier message except at a cut: every other request extends the one
/// before it.
#[tokio::test]
async fn a_loop_built_with_no_mode_set_edits_no_earlier_message_except_at_a_cut() {
    assert_append_only(&simulate_on(None, 40_000, 200, Some(7)).await);
}
