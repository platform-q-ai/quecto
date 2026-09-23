//! Agent Commander dry run (SPIKE — never merged).
//!
//! Every observed event is judged by TypeSafe (Jev) and logged; nothing is
//! sent to an agent or the owner. Enabled with `QUECTO_AGENT_COMMANDER=dry-run`.
//! Key: `TYPESAFE_API_KEY`, else `~/.config/typesafe/api_key`.
//! Log: `<base_dir>/agent-commander/<session>.jsonl`, one line per decision.
use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::application::agent_commander::ports::{CommanderEvent, CommanderSink};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const MODEL: &str = "jev-latest";
/// Actions fire only above this confidence (dry run: recorded, not taken).
const ACT_CONFIDENCE: f64 = 0.6;
const RECENT: usize = 8;
/// Configuration answers stop the run on a non-4xx failure only above this.
const STOP_CONFIDENCE: f64 = 0.8;
/// A tool error is judged only once the same failure happens this many times
/// in a row; a single failure (a red test in TDD, a mistyped edit) is routine.
const TOOL_ERROR_REPEATS: u32 = 3;
/// A matter stays with the parent agent only on a clear yes.
const PARENT_HANDLES: f64 = 0.7;

#[derive(Debug)]
struct Job {
    ts: String,
    seq: u64,
    session_key: String,
    model: String,
    event: CommanderEvent,
}

#[derive(Debug)]
pub struct DryRunCommander {
    tx: mpsc::UnboundedSender<Job>,
    seq: std::sync::atomic::AtomicU64,
}

impl DryRunCommander {
    /// `Some` when the switch is on and a key is available.
    pub fn from_env(base_dir: &Path, role: AgentRole) -> Option<Arc<Self>> {
        let switch = std::env::var("QUECTO_AGENT_COMMANDER").unwrap_or_default();
        if !matches!(switch.as_str(), "dry-run" | "1") {
            return None;
        }
        let key = load_key()?;
        let dir = base_dir.join("agent-commander");
        std::fs::create_dir_all(&dir).ok()?;
        let (tx, rx) = mpsc::unbounded_channel();
        let worker = Worker::new(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .ok()?,
            key,
            dir,
            role,
            ENDPOINT.to_string(),
            std::time::Duration::from_millis(500),
        );
        // Its own thread and runtime: independent of the agent's runtime
        // (which may not exist yet) and never competing with the loop.
        std::thread::Builder::new()
            .name("agent-commander".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(worker.run(rx)),
                    Err(e) => tracing::warn!(target: "agent_commander", error = %e, "no runtime"),
                }
            })
            .ok()?;
        Some(Arc::new(Self {
            tx,
            seq: std::sync::atomic::AtomicU64::new(0),
        }))
    }
}

fn load_key() -> Option<String> {
    let key = std::env::var("TYPESAFE_API_KEY").ok().or_else(|| {
        let path = dirs::home_dir()?.join(".config/typesafe/api_key");
        std::fs::read_to_string(path).ok()
    })?;
    let key = key.trim().to_string();
    (!key.is_empty()).then_some(key)
}

#[path = "agent_commander_questions.rs"]
mod questions;
#[path = "agent_commander_stall.rs"]
mod stall;
use questions::{escalation_questions, parent_question, receiving_parent_question};
use stall::{StallWatch, progress_question};

#[path = "agent_commander_policy.rs"]
mod policy;
use policy::escalation_rank;

#[path = "agent_commander_replay.rs"]
mod replay;
pub use replay::replay_session;

impl CommanderSink for DryRunCommander {
    fn observe(&self, session_key: &str, model: &str, event: CommanderEvent) {
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = self.tx.send(Job {
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| format!("{:.3}", d.as_secs_f64()))
                .unwrap_or_default(),
            seq,
            session_key: session_key.to_string(),
            model: model.to_string(),
            event,
        });
    }
}

/// Who this agent answers to.
#[derive(Debug, Clone)]
pub enum AgentRole {
    /// Talks to the owner in the TUI.
    Root,
    /// A delegated agent reporting to a parent agent.
    Child { parent_id: Option<String> },
}

struct Worker {
    client: reqwest::Client,
    key: String,
    dir: PathBuf,
    role: AgentRole,
    recent: VecDeque<String>,
    /// The TypeSafe endpoint (a local mock in tests).
    endpoint: String,
    /// First retry delay; each retry waits four times longer.
    backoff: std::time::Duration,
    /// (session, event kind, owner kind) already raised with the owner.
    escalated: HashSet<(String, String, String)>,
    /// The current run of identical tool failures: (signature, count).
    tool_streak: Option<(String, u32)>,
    /// Recent tool calls, for the stall check.
    stall: StallWatch,
    /// Set while the event being built should carry the stall question.
    stall_check: bool,
    /// Sub-agent crashes seen per session: a second one is not a one-off.
    crashes: std::collections::HashMap<String, u32>,
}

/// Rate limits and any server-side failure (5xx, incl. Cloudflare 520-529)
/// are worth retrying; other client errors are final.
fn retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// A 403 whose body is an HTML page came from the CDN in front of the API
/// (a Cloudflare challenge), not from TypeSafe refusing the key.
fn retryable_response(status: u16, body: &str) -> bool {
    retryable_status(status) || (status == 403 && body.trim_start().starts_with('<'))
}

fn event_kind(event: &CommanderEvent) -> &'static str {
    match event {
        CommanderEvent::TurnEnd { .. } => "turn_end",
        CommanderEvent::ProviderFailure { .. } => "provider_failure",
        CommanderEvent::ToolError { .. } => "tool_error",
        CommanderEvent::ToolOk { .. } => "tool_ok",
        CommanderEvent::SubagentNotice { .. } => "subagent_notice",
    }
}

fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let skipped: String = text.chars().skip(count - max).collect();
    format!("…{skipped}")
}

fn head(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

impl Worker {
    fn new(
        client: reqwest::Client,
        key: String,
        dir: PathBuf,
        role: AgentRole,
        endpoint: String,
        backoff: std::time::Duration,
    ) -> Self {
        Self {
            client,
            key,
            dir,
            role,
            recent: VecDeque::new(),
            endpoint,
            backoff,
            escalated: HashSet::new(),
            tool_streak: None,
            stall: StallWatch::default(),
            stall_check: false,
            crashes: std::collections::HashMap::new(),
        }
    }

    /// `would_do`, then each kind of matter interrupts the owner once per
    /// session; repeats of it are recorded as `already_escalated`.
    fn decide(&mut self, session: &str, event: &CommanderEvent, answers: &Value) -> Value {
        let mut actions = Self::would_do(event, &self.role, answers);
        if let CommanderEvent::SubagentNotice { notice, .. } = event
            && notice.contains("exited unexpectedly")
        {
            let crashes = self.crashes.entry(session.to_string()).or_default();
            *crashes += 1;
            let current = actions
                .get("22_composed")
                .or_else(|| actions.get("22"))
                .and_then(Value::as_str)
                .unwrap_or("none");
            if *crashes >= 2 && escalation_rank(current) < escalation_rank("promote_in_tui") {
                actions["22_composed"] = json!("promote_in_tui");
            }
        }
        let owner_kind = answers
            .pointer("/owner_kind/choice")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let key = (
            session.to_string(),
            event_kind(event).to_string(),
            owner_kind.to_string(),
        );
        let interrupts = ["22", "22_composed"]
            .into_iter()
            .filter(|id| actions.get(*id).and_then(Value::as_str) == Some("interrupt_owner"))
            .collect::<Vec<_>>();
        if !interrupts.is_empty() && !self.escalated.insert(key) {
            for id in interrupts {
                actions[id] = json!("already_escalated");
            }
        }
        actions
    }

    /// Counts identical consecutive tool failures; true once one repeats
    /// often enough to be worth judging.
    fn tool_error_repeated(&mut self, event: &CommanderEvent) -> bool {
        let CommanderEvent::ToolError { tool, result, .. } = event else {
            return true;
        };
        let signature = format!("{tool}\u{0}{}", head(result, 300));
        let count = match &self.tool_streak {
            Some((last, count)) if *last == signature => count + 1,
            _ => 1,
        };
        self.tool_streak = Some((signature, count));
        count >= TOOL_ERROR_REPEATS
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Job>) {
        while let Some(job) = rx.recv().await {
            self.judge(job).await;
        }
    }

    fn role_text(&self) -> Value {
        match &self.role {
            AgentRole::Root => {
                json!("root agent: talks directly to the human owner in a terminal UI")
            }
            AgentRole::Child { parent_id } => json!({
                "role": "sub-agent: was given a task by a parent agent and reports back to it, not to the human",
                "parent": parent_id,
            }),
        }
    }

    fn is_child(&self) -> bool {
        matches!(self.role, AgentRole::Child { .. })
    }

    /// State, questions and the decisions they feed for one event.
    fn build(
        &self,
        event: &CommanderEvent,
    ) -> (Value, serde_json::Map<String, Value>, Vec<&'static str>) {
        let mut questions = serde_json::Map::new();
        let mut decisions = Vec::new();
        let recent: Vec<&String> = self.recent.iter().collect();
        let state = match event {
            CommanderEvent::TurnEnd {
                turn,
                prompt,
                final_text,
                stop_reason,
                tool_rounds,
                ended_by,
                output_tokens,
                max_tokens,
            } => {
                decisions.push("5");
                questions.insert(
                    "turn_end".into(),
                    json!({
                        "type": "choice",
                        "instructions": "Why did this agent's turn end? Judge from `final_text` (the agent's last message), `stop_reason` from the model provider, and `ended_by`.",
                        "criteria": {
                                                        "complete": "The work asked for in `prompt` is finished and nothing is asked of anyone",
                            "waiting_on_others": "The agent handed work to sub-agents, reviewers, workflows or other processes (including just launching or starting one) and correctly paused until their results arrive; it will continue when they report, and nothing is asked of anyone now",
                            "needs_input": "The agent asks a question or needs a decision or approval before it can continue",
                            "cut_off": "The message was truncated mid-thought (output token limit, interruption) and the agent meant to keep going",
                            "stopped_early": "The agent stopped before finishing without asking anything: it gave up, summarised a plan it did not carry out, or hit a limit"
                        }
                    }),
                );
                if self.is_child() {
                    decisions.push("6");
                    questions.insert(
                        "child_state".into(),
                        json!({
                            "type": "choice",
                            "instructions": "This is a sub-agent's last message to its parent agent. What state is the sub-agent's task in?",
                            "criteria": {
                                                                "done": "The task is finished and the result is reported",
                                "waiting_on_subagents": "The sub-agent delegated parts of its task to its own sub-agents and is correctly waiting for their reports before it continues",
                                "question_for_parent": "The sub-agent asks its parent a question or needs a decision to continue",
                                "blocked": "The sub-agent cannot proceed because of something outside its control: access, missing input, a broken environment, or the task can no longer be done as instructed because its target changed or a precondition failed (for example the branch or PR head moved), even if it stopped cleanly and reported this",
                                "failed": "The sub-agent tried and could not do the task",
                                "partial": "Some of the task is done and more remains, without a question or blocker"
                            }
                        }),
                    );
                }
                decisions.push("22");
                escalation_questions(&mut questions);
                if self.is_child() {
                    parent_question(&mut questions);
                }
                json!({
                    "agent": self.role_text(),
                    "event": "turn_end",
                    "turn": turn,
                    "prompt": tail(prompt, 3000),
                    "final_text": if final_text.chars().count() > 6000 {
                        format!("{}\n[…]\n{}", head(final_text, 1500), tail(final_text, 4500))
                    } else { final_text.clone() },
                    "stop_reason": stop_reason,
                    "ended_by": ended_by,
                    "output_tokens": output_tokens,
                    "max_output_tokens": max_tokens,
                    "tool_rounds_this_turn": tool_rounds,
                    "recent_events": recent,
                })
            }
            CommanderEvent::ProviderFailure {
                turn,
                provider,
                class,
                http_status,
                error,
                outcome,
                attempt,
            } => {
                decisions.push("12");
                questions.insert(
                    "provider_error".into(),
                    json!({
                        "type": "choice",
                        "instructions": "A request from this agent to its language-model provider failed with `error`. The model only writes messages and tool calls; the harness sets the model name, endpoint, credentials and request parameters from configuration. What kind of failure is it, and so who can fix it?",
                        "criteria": {
                            "fixable_by_model": "Something the model itself wrote was invalid and it can correct it if told: its tool-call arguments, a tool name it invented, or the content of its own messages",
                            "configuration": "A setting the harness sends on the model's behalf is wrong, so only the owner can fix it: authentication, billing or quota, model name, endpoint, permissions, or request parameters such as reasoning effort, temperature or token limits that the model or endpoint does not support. Usually a 4xx `http_status` (400, 401, 402, 403, 404) that says what is wrong. A connection refused by a local model server (localhost, 127.0.0.1) means the server is not running, which is configuration",
                            "context_overflow": "The conversation is too long for the model's context window",
                            "policy_refusal": "The provider refused on content or safety policy",
                            "transient": "A temporary service problem (overload, rate limit, timeout, network, 5xx) that retrying later should fix. A 5xx `http_status` whose message says to try again is transient even when it mentions access, verification or accounts"
                        }
                    }),
                );
                decisions.push("22");
                escalation_questions(&mut questions);
                json!({
                    "agent": self.role_text(),
                    "event": "provider_failure",
                    "turn": turn,
                    "provider": provider,
                    "http_status": http_status,
                    "error": head(error, 4000),
                    "harness_outcome": outcome,
                    "attempt": attempt,
                    "rule_based_class": class,
                    "recent_events": recent,
                })
            }
            CommanderEvent::ToolError {
                turn,
                tool,
                arguments,
                result,
            } => {
                decisions.push("22");
                escalation_questions(&mut questions);
                let recent_tool_calls = if self.stall_check {
                    decisions.push("stall");
                    progress_question(&mut questions);
                    Some(self.stall.window())
                } else {
                    None
                };
                json!({
                    "agent": self.role_text(),
                    "event": "tool_error",
                    "recent_tool_calls": recent_tool_calls,
                    "turn": turn,
                    "tool": tool,
                    "arguments": head(arguments, 2000),
                    "result": head(result, 4000),
                    "recent_events": recent,
                })
            }
            CommanderEvent::ToolOk { tool, .. } => json!({"event": "tool_ok", "tool": tool}),
            CommanderEvent::SubagentNotice {
                child,
                child_uuid,
                notice,
                detail,
            } => {
                decisions.push("22");
                escalation_questions(&mut questions);
                receiving_parent_question(&mut questions);
                json!({
                    "agent": self.role_text(),
                    "event": "subagent_notice",
                    "child": child,
                    "child_uuid": child_uuid,
                    "notice": notice,
                    "detail": detail.as_deref().map(|d| head(d, 3000)),
                    "recent_events": recent,
                })
            }
        };
        (state, questions, decisions)
    }

    async fn ask(&self, body: &Value) -> Result<Value, String> {
        let mut delay = self.backoff;
        for attempt in 1..=3 {
            let sent = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.key)
                .json(body)
                .send()
                .await;
            let response = match sent {
                Ok(response) => response,
                Err(e) if e.is_timeout() && attempt < 3 => {
                    tokio::time::sleep(delay).await;
                    delay *= 4;
                    continue;
                }
                Err(e) => return Err(format!("request: {e}")),
            };
            let status = response.status();
            let text = response.text().await.map_err(|e| format!("body: {e}"))?;
            if retryable_response(status.as_u16(), &text) && attempt < 3 {
                tokio::time::sleep(delay).await;
                delay *= 4;
                continue;
            }
            if !status.is_success() {
                return Err(format!("HTTP {status}: {}", head(&text, 500)));
            }
            return serde_json::from_str(&text).map_err(|e| format!("json: {e}"));
        }
        Err("retries exhausted".into())
    }

    fn summary(event: &CommanderEvent, answers: &Value) -> String {
        let pick = |id: &str| {
            answers
                .pointer(&format!("/{id}/choice"))
                .and_then(Value::as_str)
                .unwrap_or("-")
                .to_string()
        };
        match event {
            CommanderEvent::TurnEnd {
                turn,
                stop_reason,
                ended_by,
                ..
            } => format!(
                "turn {turn} ended ({ended_by}, stop={}) judged {}",
                stop_reason.as_deref().unwrap_or("?"),
                pick("turn_end")
            ),
            CommanderEvent::ProviderFailure { outcome, class, .. } => {
                format!(
                    "provider failure {class} ({outcome}) judged {}",
                    pick("provider_error")
                )
            }
            CommanderEvent::ToolError { tool, .. } => format!("tool {tool} returned an error"),
            CommanderEvent::ToolOk { tool, .. } => format!("tool {tool} succeeded"),
            CommanderEvent::SubagentNotice { child, notice, .. } => {
                format!("sub-agent {child}: {notice}")
            }
        }
    }

    async fn judge(&mut self, job: Job) {
        let ts = job.ts.parse::<f64>().unwrap_or_default();
        self.stall_check = match &job.event {
            CommanderEvent::ToolOk { tool, .. } => {
                self.stall.record(ts, tool, true, "");
                let record = json!({
                    "ts": job.ts,
                    "seq": job.seq,
                    "session": job.session_key,
                    "agent_role": format!("{:?}", self.role),
                    "decisions": [],
                    "event": job.event,
                    "skipped": "tool_ok",
                    "mode": "dry_run",
                });
                self.write(&job.session_key, &record);
                return;
            }
            CommanderEvent::ToolError { tool, result, .. } => {
                self.stall.record(ts, tool, false, result)
            }
            // A new instruction starts a new attempt: judge it on its own calls.
            CommanderEvent::TurnEnd { .. } => {
                self.stall = StallWatch::default();
                false
            }
            _ => false,
        };
        let repeated = self.tool_error_repeated(&job.event);
        if repeated && matches!(job.event, CommanderEvent::ToolError { .. }) {
            // The same failure again and again is the plainest stall.
            self.stall_check = true;
        }
        let (state, questions, decisions) = self.build(&job.event);
        let body = json!({ "model": MODEL, "state": state, "questions": questions });
        let started = Instant::now();
        let result = self.ask(&body).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let (answers, jev_model, usage, error) = match &result {
            Ok(response) => (
                response.get("answers").cloned().unwrap_or(Value::Null),
                response.get("model").cloned().unwrap_or(Value::Null),
                response.get("usage").cloned().unwrap_or(Value::Null),
                None,
            ),
            Err(e) => (Value::Null, Value::Null, Value::Null, Some(e.clone())),
        };
        let record = json!({
            "ts": job.ts,
            "seq": job.seq,
            "session": job.session_key,
            "pid": std::process::id(),
            "agent_role": format!("{:?}", self.role),
            "agent_model": job.model,
            "decisions": decisions,
            "event": job.event,
            "state_sent": state,
            "questions": questions,
            "answers": answers,
            "would_do": self.decide(&job.session_key, &job.event, &answers),
            "jev_model": jev_model,
            "usage": usage,
            "latency_ms": latency_ms,
            "error": error,
            "mode": "dry_run",
        });
        let summary = Self::summary(&job.event, &answers);
        self.recent.push_back(summary);
        if self.recent.len() > RECENT {
            self.recent.pop_front();
        }
        self.write(&job.session_key, &record);
    }

    fn write(&self, session_key: &str, record: &Value) {
        let name = session_key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let name = if name.is_empty() {
            "no-session".to_string()
        } else {
            name
        };
        let path = self.dir.join(format!("{name}.jsonl"));
        let line = format!("{record}\n");
        let write = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = write {
            tracing::warn!(target: "agent_commander", error = %e, "dry-run log write failed");
        }
    }
}

#[cfg(test)]
#[path = "agent_commander_tests.rs"]
mod tests;
