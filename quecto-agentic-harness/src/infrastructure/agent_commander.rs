//! Agent Commander dry run (SPIKE — never merged).
//!
//! Every observed event is judged by TypeSafe (Jev) and logged; nothing is
//! sent to an agent or the owner. Enabled with `QUECTO_AGENT_COMMANDER=dry-run`.
//! Key: `TYPESAFE_API_KEY`, else `~/.config/typesafe/api_key`.
//! Log: `<base_dir>/agent-commander/<session>.jsonl`, one line per decision.
use std::collections::VecDeque;
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
        let key = std::env::var("TYPESAFE_API_KEY").ok().or_else(|| {
            let path = dirs::home_dir()?.join(".config/typesafe/api_key");
            std::fs::read_to_string(path).ok()
        })?;
        let key = key.trim().to_string();
        if key.is_empty() {
            return None;
        }
        let dir = base_dir.join("agent-commander");
        std::fs::create_dir_all(&dir).ok()?;
        let (tx, rx) = mpsc::unbounded_channel();
        let worker = Worker {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .ok()?,
            key,
            dir,
            role,
            recent: VecDeque::new(),
        };
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

/// #22: does a human need to see this now, what kind, how urgent.
fn escalation_questions(questions: &mut serde_json::Map<String, Value>) {
    questions.insert(
        "owner_needed".into(),
        json!({
            "type": "noul",
            "instructions": "The owner is a human who supervises these coding agents but is not watching every event. Does the owner need to see this event now, rather than later or never?",
            "criteria": {
                "true": "Something only the owner can resolve or should know promptly: a decision or approval only the owner can give, a blocker agents cannot clear, a failure that stops the work, or a risky or surprising outcome",
                "false": "Routine progress, a finished step, or something the agents can handle themselves"
            }
        }),
    );
    questions.insert(
        "owner_kind".into(),
        json!({
            "type": "choice",
            "instructions": "If this event were raised with the owner, what kind of matter would it be?",
            "criteria": {
                "owner_decision": "A choice only the owner can make (scope, priorities, trade-offs, preferences)",
                "approval": "The agent needs permission before doing something (risky, destructive, outward-facing, or costly)",
                "blocker": "Work cannot continue until something outside the agents is fixed (credentials, configuration, access, environment)",
                "agent_answerable": "A question or problem another agent or the agent itself can resolve without the owner",
                "information": "Useful to know, but no action is needed from anyone"
            }
        }),
    );
    questions.insert(
        "urgency".into(),
        json!({
            "type": "score",
            "instructions": "How urgently does the owner need to act on this event?",
            "criteria": [
                "No action needed from the owner at all",
                "Can wait for the owner's next routine check-in or a daily digest",
                "The owner should look within the hour; work is slowed or waiting",
                "The owner should look now; work is stopped, at risk, or something harmful may happen"
            ]
        }),
    );
}

impl Worker {
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
                                "question_for_parent": "The sub-agent asks its parent a question or needs a decision to continue",
                                "blocked": "The sub-agent cannot proceed because of something outside its control (access, missing input, broken environment)",
                                "failed": "The sub-agent tried and could not do the task",
                                "partial": "Some of the task is done and more remains, without a question or blocker"
                            }
                        }),
                    );
                }
                decisions.push("22");
                escalation_questions(&mut questions);
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
                            "configuration": "A setting the harness sends on the model's behalf is wrong, so only the owner can fix it: authentication, billing or quota, model name, endpoint, permissions, or request parameters such as reasoning effort, temperature or token limits that the model or endpoint does not support",
                            "context_overflow": "The conversation is too long for the model's context window",
                            "policy_refusal": "The provider refused on content or safety policy",
                            "transient": "A temporary service problem (overload, rate limit, timeout, network, 5xx) that retrying later should fix"
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
                json!({
                    "agent": self.role_text(),
                    "event": "tool_error",
                    "turn": turn,
                    "tool": tool,
                    "arguments": head(arguments, 2000),
                    "result": head(result, 4000),
                    "recent_events": recent,
                })
            }
            CommanderEvent::SubagentNotice {
                child,
                child_uuid,
                notice,
                detail,
            } => {
                decisions.push("22");
                escalation_questions(&mut questions);
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

    /// What code would do with the answers (dry run: recorded only).
    fn would_do(answers: &Value) -> Value {
        let choice = |id: &str| -> Option<(String, f64)> {
            let a = answers.get(id)?;
            Some((
                a.get("choice")?.as_str()?.to_string(),
                a.get("confidence")?.as_f64()?,
            ))
        };
        let mut actions = serde_json::Map::new();
        if let Some((c, conf)) = choice("turn_end") {
            let act = if conf < ACT_CONFIDENCE {
                "none_uncertain"
            } else {
                match c.as_str() {
                    "complete" => "none",
                    "needs_input" => "notify_asker_needs_input",
                    "cut_off" => "auto_continue",
                    "stopped_early" => "nudge_continue",
                    _ => "none",
                }
            };
            actions.insert("5".into(), json!(act));
        }
        if let Some((c, conf)) = choice("child_state") {
            let act = if conf < ACT_CONFIDENCE {
                "plain_idle_note_uncertain"
            } else {
                match c.as_str() {
                    "done" => "tell_parent_done",
                    "question_for_parent" => "tell_parent_question",
                    "blocked" => "tell_parent_blocked_badge",
                    "failed" => "tell_parent_failed_badge",
                    "partial" => "nudge_child_continue",
                    _ => "plain_idle_note",
                }
            };
            actions.insert("6".into(), json!(act));
        }
        if let Some((c, conf)) = choice("provider_error") {
            let act = if conf < ACT_CONFIDENCE {
                "keep_rule_based_handling"
            } else {
                match c.as_str() {
                    "fixable_by_model" => "reprompt_with_error",
                    "configuration" => "stop_and_tell_owner",
                    "context_overflow" => "compact_and_retry",
                    "policy_refusal" => "stop_no_retry",
                    "transient" => "retry_with_backoff",
                    _ => "keep_rule_based_handling",
                }
            };
            actions.insert("12".into(), json!(act));
        }
        if let Some(noul) = answers
            .pointer("/owner_needed/noul")
            .and_then(Value::as_f64)
        {
            let urgency = answers
                .pointer("/urgency/score")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let act = if noul >= 0.8 && urgency >= 2.0 {
                "interrupt_owner"
            } else if noul >= 0.8 {
                "promote_in_tui"
            } else if noul >= 0.5 {
                "add_to_digest"
            } else {
                "none"
            };
            actions.insert("22".into(), json!(act));
            // Composition (policy in code): a configuration failure only the
            // owner can fix, or a child that is blocked/failed, reaches the
            // owner whatever the standalone escalation judgment said.
            let composed = match (
                actions.get("12").and_then(Value::as_str),
                actions.get("6").and_then(Value::as_str),
            ) {
                (Some("stop_and_tell_owner"), _) => Some("interrupt_owner"),
                (_, Some("tell_parent_blocked_badge" | "tell_parent_failed_badge")) => {
                    Some("promote_in_tui")
                }
                _ => None,
            };
            if let Some(composed) = composed {
                actions.insert("22_composed".into(), json!(composed));
            }
        }
        Value::Object(actions)
    }

    async fn ask(&self, body: &Value) -> Result<Value, String> {
        let mut delay = std::time::Duration::from_millis(500);
        for attempt in 1..=3 {
            let response = self
                .client
                .post(ENDPOINT)
                .bearer_auth(&self.key)
                .json(body)
                .send()
                .await
                .map_err(|e| format!("request: {e}"))?;
            let status = response.status();
            if (status.as_u16() == 429 || status.as_u16() == 529) && attempt < 3 {
                tokio::time::sleep(delay).await;
                delay *= 4;
                continue;
            }
            let text = response.text().await.map_err(|e| format!("body: {e}"))?;
            if !status.is_success() {
                return Err(format!("HTTP {status}: {}", head(&text, 500)));
            }
            return serde_json::from_str(&text).map_err(|e| format!("json: {e}"));
        }
        Err("rate limited".into())
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
            CommanderEvent::SubagentNotice { child, notice, .. } => {
                format!("sub-agent {child}: {notice}")
            }
        }
    }

    async fn judge(&mut self, job: Job) {
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
            "would_do": Self::would_do(&answers),
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
        let name = job
            .session_key
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
