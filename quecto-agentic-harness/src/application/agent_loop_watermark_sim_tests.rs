//! SPIKE (spike/watermark-context): an offline prompt-cache simulator.
//!
//! It drives the real agent loop and context code with a seeded synthetic
//! session: each turn a user prompt, 1-6 tool calls (one per request) with
//! results of 200-8,000 estimated tokens (log-uniform), then an answer.
//! Every request is serialized as the Codex provider sends it (its request
//! body builder: instructions, tools, then each input item in order), and
//! the cached tokens are the longest common prefix with the previous
//! request, at 4 bytes a token, rounded down to 128-token blocks with a
//! 1,024-token minimum, as OpenAI's prompt cache counts them.
//!
//! The table: `cargo test --lib watermark_sim_table -- --ignored --nocapture`.

use super::ctx_mgmt_tests::{CapturingAuditSink, MemSpillStore};
use crate::application::agent_loop::tests::MockRegistry;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::audit::ports::AuditSink;
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::tools::ports::Tool;
use crate::domain::audit::AuditEvent;
use crate::domain::conversation::watermark::ContextWatermark;
use crate::domain::error::DomainError;
use crate::domain::large_result_collapse::LargeResultCollapse;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::providers::codex::CodexProvider;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

const SEED: u64 = 0x2398_5eed;
const BYTES_PER_TOKEN: usize = 4;
const BLOCK: usize = 128;
const MIN_CACHED: usize = 1_024;

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

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, low: usize, high: usize) -> usize {
        low + (self.next() % (high - low + 1) as u64) as usize
    }

    fn log_uniform(&mut self, low: usize, high: usize) -> usize {
        let (l, h) = ((low as f64).ln(), (high as f64).ln());
        (l + (h - l) * self.unit()).exp().round() as usize
    }
}

const WORDS: &[&str] = &[
    "the", "context", "result", "module", "function", "returns", "value", "error", "handler",
    "request", "session", "cache", "prefix", "message", "tool", "call", "file", "line", "struct",
    "impl", "trait", "async", "await", "match", "option", "string", "vector", "index", "token",
    "budget",
];

/// About `tokens` estimated tokens of prose, the same for the same seed.
fn prose(seed: u64, tokens: usize) -> String {
    let mut rng = Rng(seed);
    let mut text = String::with_capacity(tokens * BYTES_PER_TOKEN + 128);
    let mut estimated = 0;
    while estimated < tokens {
        let mut line = String::new();
        for _ in 0..12 {
            line.push_str(WORDS[rng.between(0, WORDS.len() - 1)]);
            line.push(' ');
        }
        line.push('\n');
        estimated += Message::estimate_tokens(&line);
        text.push_str(&line);
    }
    text
}

/// What results a session's tool calls return.
#[derive(Clone, Copy, Debug)]
enum Results {
    /// 200-8,000 tokens, log-uniform (median about 1,260).
    LogUniform,
    /// 200-8,000 tokens, uniform (mean 4,100): the heavy case.
    Uniform,
}

/// One turn of the script: its prompt, its calls' result sizes, its answer.
#[derive(Clone, Debug)]
struct Turn {
    prompt_tokens: usize,
    calls: Vec<usize>,
    answer_tokens: usize,
}

/// Turns until `requests` requests are made (one per call, one per answer).
fn script(requests: usize, results: Results) -> Vec<Turn> {
    let mut rng = Rng(SEED);
    let mut turns = Vec::new();
    let mut made = 0;
    while made < requests {
        let mut calls: Vec<usize> = (0..rng.between(1, 6))
            .map(|_| match results {
                Results::LogUniform => rng.log_uniform(200, 8_000),
                Results::Uniform => rng.between(200, 8_000),
            })
            .collect();
        calls.truncate(requests - made - 1);
        made += calls.len() + 1;
        turns.push(Turn {
            prompt_tokens: rng.between(30, 300),
            calls,
            answer_tokens: rng.between(100, 600),
        });
    }
    debug_assert_eq!(made, requests);
    turns
}

/// One request as the cache sees it.
#[derive(Debug, Clone, Copy, Default)]
struct Observed {
    total: usize,
    cached: usize,
    /// The previous request was not a prefix of this one.
    broke: bool,
}

/// The provider: plays the script and serializes every request.
#[derive(Debug)]
struct SimProvider {
    script: Vec<Turn>,
    cursor: Mutex<(usize, usize)>,
    previous: Mutex<String>,
    observed: Mutex<Vec<Observed>>,
    /// The head's serialized prefix (instructions, tools, first message).
    head: Mutex<Option<String>>,
    heads_kept: Mutex<Vec<bool>>,
}

impl SimProvider {
    fn new(script: Vec<Turn>) -> Self {
        Self {
            script,
            cursor: Mutex::new((0, 0)),
            previous: Mutex::new(String::new()),
            observed: Mutex::new(Vec::new()),
            head: Mutex::new(None),
            heads_kept: Mutex::new(Vec::new()),
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
        self.heads_kept
            .lock()
            .unwrap()
            .push(wire.starts_with(head.as_str()));
        let mut previous = self.previous.lock().unwrap();
        let common = previous
            .bytes()
            .zip(wire.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let total = wire.len() / BYTES_PER_TOKEN;
        let blocks = common / BYTES_PER_TOKEN / BLOCK * BLOCK;
        let cached = if blocks >= MIN_CACHED {
            blocks.min(total)
        } else {
            0
        };
        self.observed.lock().unwrap().push(Observed {
            total,
            cached,
            broke: !previous.is_empty() && common < previous.len(),
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
        let response = if call < script.calls.len() {
            *cursor = (turn, call + 1);
            LlmResponse {
                content: None,
                tool_calls: vec![ToolCall {
                    id: format!("call-{turn}-{call}"),
                    name: "read".to_string(),
                    arguments: format!(
                        r#"{{"path":"src/m{turn}_{call}.rs","tokens":{}}}"#,
                        script.calls[call]
                    ),
                }],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            }
        } else {
            *cursor = (turn + 1, 0);
            LlmResponse {
                content: Some(prose(SEED ^ turn as u64, script.answer_tokens)),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            }
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
            .fold(SEED, |h, b| h.rotate_left(5) ^ b as u64);
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

/// A never-called tool whose definition stands in for the rest of the
/// tool set (about 8k tokens, as quecto's are).
struct SimDocs;

impl Tool for SimDocs {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "docs".into(),
            description: prose(7, 8_000).into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        unreachable!("never called")
    }
}

/// A context mode under measurement.
#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Today's default for a parent: tool dial 50, message dial 50, 200k.
    Default,
    /// Today's swarm member: also the 48k ceiling and 2k/3-turn collapse.
    SwarmMember,
    /// The watermark context at (high, low), `max_context_tokens` raised
    /// to 1M so the high mark is reached.
    Watermark(usize, usize),
    /// The watermark context at (high, low) under the 200k default
    /// `max_context_tokens`: the ceiling wins.
    WatermarkUnderDefaultCeiling(usize, usize),
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Default => write!(f, "(a) default pruning"),
            Mode::SwarmMember => write!(f, "(a) swarm member pruning"),
            Mode::Watermark(h, l) => write!(f, "(b) watermark {}k/{}k", h / 1000, l / 1000),
            Mode::WatermarkUnderDefaultCeiling(h, l) => write!(
                f,
                "(b) watermark {}k/{}k, 200k default ceiling",
                h / 1000,
                l / 1000
            ),
        }
    }
}

struct Outcome {
    observed: Vec<Observed>,
    cuts: usize,
    prune_passes: usize,
    heads_kept: Vec<bool>,
}

async fn simulate(mode: Mode, requests: usize, results: Results) -> Outcome {
    let script = script(requests, results);
    let provider = Arc::new(SimProvider::new(script.clone()));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(SimRead));
    registry.register(Arc::new(SimDocs));
    let sink = Arc::new(CapturingAuditSink::default());
    let large = match mode {
        Mode::SwarmMember => LargeResultCollapse {
            over_tokens: 2_000,
            after_turns: 3,
        },
        Mode::Default | Mode::Watermark(..) | Mode::WatermarkUnderDefaultCeiling(..) => {
            LargeResultCollapse::DISABLED
        }
    };
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
        context_collapse_after_tool_calls: 50,
        max_context_tokens: match mode {
            Mode::Watermark(..) => 1_000_000,
            Mode::Default | Mode::SwarmMember | Mode::WatermarkUnderDefaultCeiling(..) => 200_000,
        },
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: Some(sink.clone() as Arc<dyn AuditSink>),
        pin_recent_turns: 2,
        context_collapse_after_messages: 50,
        large_result_collapse: large,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    match mode {
        Mode::SwarmMember => agent.context_ceiling_cap().lower_to(48_000),
        Mode::Watermark(high, low) | Mode::WatermarkUnderDefaultCeiling(high, low) => {
            agent.set_context_watermark(Some(ContextWatermark::new(high, low).unwrap()));
        }
        Mode::Default => {}
    }
    let mut messages = vec![Message::system(prose(1, 4_000))];
    for (n, turn) in script.iter().enumerate() {
        messages.push(Message::user(prose(
            SEED ^ (n as u64) << 8,
            turn.prompt_tokens,
        )));
        agent.run_loop(&mut messages).await.unwrap();
    }
    let events = sink.events.lock().unwrap();
    let pruned: Vec<usize> = events
        .iter()
        .filter_map(|event| match event {
            AuditEvent::ContextPruned {
                messages_dropped, ..
            } => Some(*messages_dropped),
            _ => None,
        })
        .collect();
    let observed = provider.observed.lock().unwrap().clone();
    assert_eq!(observed.len(), requests, "the script ran to its end");
    Outcome {
        observed,
        cuts: match mode {
            Mode::Watermark(..) | Mode::WatermarkUnderDefaultCeiling(..) => {
                pruned.iter().filter(|&&dropped| dropped > 0).count()
            }
            Mode::Default | Mode::SwarmMember => 0,
        },
        prune_passes: pruned.len(),
        heads_kept: provider.heads_kept.lock().unwrap().clone(),
    }
}

fn hit(obs: &[Observed]) -> String {
    let (total, cached) = obs
        .iter()
        .fold((0, 0), |(t, c), o| (t + o.total, c + o.cached));
    if total == 0 {
        return "-".to_string();
    }
    format!("{:.2}%", 100.0 * cached as f64 / total as f64)
}

/// One table row. The first fill runs up to the first request whose
/// prefix broke (the first cut, or the first prune pass that rewrote
/// history); steady state is everything from there on.
fn row(mode: Mode, outcome: &Outcome) -> String {
    let all = &outcome.observed;
    let first_break = all.iter().position(|o| o.broke).unwrap_or(all.len());
    let (fill, steady) = all.split_at(first_break);
    let total: usize = all.iter().map(|o| o.total).sum();
    let cached: usize = all.iter().map(|o| o.cached).sum();
    let steady_uncached: usize = steady.iter().map(|o| o.total - o.cached).sum();
    let fresh = steady_uncached / steady.len().max(1);
    let max = all.iter().map(|o| o.total).max().unwrap_or(0);
    let mean = total / all.len().max(1);
    let breaks = all.iter().filter(|o| o.broke).count();
    format!(
        "| {mode} | {} | {} ({} req) | {} | {:.2}M | {:.2}M | {:.1}k | {} | {} | {} | {}k | {}k |",
        hit(steady),
        hit(fill),
        fill.len(),
        hit(all),
        total as f64 / 1e6,
        (total - cached) as f64 / 1e6,
        fresh as f64 / 1e3,
        outcome.cuts,
        outcome.prune_passes,
        breaks,
        max / 1000,
        mean / 1000,
    )
}

/// The mechanism: in watermark mode every request is the previous one
/// with messages appended, except a request right after a cut, and the
/// head (instructions, tools, the brief) is the same on every request.
#[tokio::test]
async fn watermark_requests_append_only_between_cuts_and_keep_the_head() {
    let outcome = simulate(Mode::Watermark(40_000, 20_000), 150, Results::LogUniform).await;
    let breaks: Vec<usize> = (0..outcome.observed.len())
        .filter(|&i| outcome.observed[i].broke)
        .collect();
    assert!(outcome.cuts >= 2, "several cuts: {}", outcome.cuts);
    assert_eq!(breaks.len(), outcome.cuts, "a break is a cut: {breaks:?}");
    assert!(
        outcome.heads_kept.iter().all(|&kept| kept),
        "the head never changes"
    );
    let max = outcome.observed.iter().map(|o| o.total).max().unwrap();
    assert!(
        max < 40_000 + 8_000 + 1_000,
        "under high plus one exchange: {max}"
    );
}

#[tokio::test]
#[ignore = "the spike's measurement table: run with --ignored --nocapture"]
async fn watermark_sim_table() {
    let requests: usize = std::env::var("WATERMARK_SIM_REQUESTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(800);
    let modes = [
        Mode::Watermark(256_000, 70_000),
        Mode::Watermark(120_000, 40_000),
        Mode::Watermark(100_000, 30_000),
        Mode::Watermark(60_000, 25_000),
        Mode::WatermarkUnderDefaultCeiling(256_000, 70_000),
        Mode::Default,
        Mode::SwarmMember,
    ];
    for results in [Results::LogUniform, Results::Uniform] {
        eprintln!("\n{requests} requests, results {results:?}, seed {SEED:#x}");
        eprintln!(
            "| mode | steady hit | first-fill hit | whole-run hit | input | uncached | uncached/req (steady) | cuts | prune passes | cache breaks | max ctx | mean ctx |"
        );
        eprintln!("|---|---|---|---|---|---|---|---|---|---|---|---|");
        for mode in modes {
            let outcome = simulate(mode, requests, results).await;
            eprintln!("{}", row(mode, &outcome));
        }
    }
}
