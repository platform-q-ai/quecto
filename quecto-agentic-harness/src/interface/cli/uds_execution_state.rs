use crate::domain::agent::AgentProgressEvent;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub(crate) type ExecutionStateHandle = Arc<Mutex<ExecutionState>>;
const WINDOW: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSnapshot {
    pub phase: String,
    pub activity_generation: u64,
    pub last_activity_at: String,
    pub last_activity_seconds_ago: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<CurrentToolSnapshot>,
    pub tools: ToolSummary,
    pub progress: ProgressSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentToolSnapshot {
    pub name: String,
    pub call_id: String,
    pub started_at: String,
    pub elapsed_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolSummary {
    pub used: Vec<String>,
    pub started: u64,
    pub completed: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSummary {
    pub state: String,
    pub reason: String,
    pub window_seconds: u64,
    pub last_progress_seconds_ago: u64,
    pub tool_calls_completed: u64,
    pub tool_calls_failed: u64,
}

#[derive(Debug)]
pub(crate) struct ExecutionState {
    phase: &'static str,
    generation: u64,
    last_activity: Instant,
    last_activity_at: String,
    active_tools: BTreeMap<String, CurrentTool>,
    used: BTreeSet<String>,
    started: u64,
    completed: u64,
    failed: u64,
    recent: VecDeque<(Instant, bool)>,
    message_count: usize,
}

#[derive(Debug)]
struct CurrentTool {
    name: String,
    call_id: String,
    started: Instant,
    started_at: String,
}

fn timestamp_now() -> String {
    humantime::format_rfc3339_seconds(SystemTime::now()).to_string()
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self {
            phase: "idle",
            generation: 0,
            last_activity: Instant::now(),
            last_activity_at: timestamp_now(),
            active_tools: BTreeMap::new(),
            used: BTreeSet::new(),
            started: 0,
            completed: 0,
            failed: 0,
            recent: VecDeque::new(),
            message_count: 0,
        }
    }
}

impl ExecutionState {
    fn touch(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.last_activity = Instant::now();
        self.last_activity_at = timestamp_now();
    }
    pub(crate) fn set_message_count(&mut self, count: usize) {
        self.message_count = count;
    }

    pub(crate) fn add_messages(&mut self, count: usize) {
        self.message_count = self.message_count.saturating_add(count);
        self.touch();
    }

    pub(crate) fn message_count(&self) -> usize {
        self.message_count
    }

    pub(crate) fn start_run(&mut self) {
        self.phase = "thinking";
        self.active_tools.clear();
        self.used.clear();
        self.started = 0;
        self.completed = 0;
        self.failed = 0;
        self.recent.clear();
        self.touch();
    }
    pub(crate) fn finish_run(&mut self) {
        self.phase = "idle";
        self.active_tools.clear();
        self.touch();
    }
    pub(crate) fn observe(&mut self, event: &AgentProgressEvent) {
        match event {
            AgentProgressEvent::Thinking { .. } => {
                self.phase = "thinking";
                self.active_tools.clear();
                self.touch();
            }
            AgentProgressEvent::ToolStarted {
                tool_call_id, name, ..
            } => {
                self.phase = "runningTool";
                self.started = self.started.saturating_add(1);
                self.used.insert(name.clone());
                self.active_tools.insert(
                    tool_call_id.clone(),
                    CurrentTool {
                        name: name.clone(),
                        call_id: tool_call_id.clone(),
                        started: Instant::now(),
                        started_at: timestamp_now(),
                    },
                );
                self.touch();
            }
            AgentProgressEvent::ToolFinished {
                tool_call_id,
                is_error,
                ..
            } => {
                self.completed = self.completed.saturating_add(1);
                if *is_error {
                    self.failed = self.failed.saturating_add(1);
                }
                self.recent.push_back((Instant::now(), *is_error));
                self.active_tools.remove(tool_call_id);
                self.phase = if self.active_tools.is_empty() {
                    "thinking"
                } else {
                    "runningTool"
                };
                self.touch();
            }
            AgentProgressEvent::TurnCompleted { messages } => {
                self.add_messages(messages.len());
            }
            AgentProgressEvent::Done => {
                self.phase = "finalizing";
                self.active_tools.clear();
                self.touch();
            }
            _ => {}
        }
    }
    pub(crate) fn snapshot(&mut self) -> ExecutionSnapshot {
        let now = Instant::now();
        while self
            .recent
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > WINDOW)
        {
            self.recent.pop_front();
        }
        let recent_completed = self.recent.len() as u64;
        let recent_failed = self.recent.iter().filter(|(_, failed)| *failed).count() as u64;
        let activity_ago = now.duration_since(self.last_activity).as_secs();
        let progress_ago = self
            .recent
            .back()
            .map_or(activity_ago, |(at, _)| now.duration_since(*at).as_secs());
        let (state, reason) = if recent_completed > 0 {
            (
                "advancing",
                format!("{recent_completed} tools completed in the last 120 seconds"),
            )
        } else if self.phase != "idle" {
            (
                "active",
                format!(
                    "{} with no completed tools in the last 120 seconds",
                    self.phase
                ),
            )
        } else {
            (
                "quiet",
                "no tool activity in the last 120 seconds".to_string(),
            )
        };
        ExecutionSnapshot {
            phase: self.phase.into(),
            activity_generation: self.generation,
            last_activity_at: self.last_activity_at.clone(),
            last_activity_seconds_ago: activity_ago,
            current_tool: self
                .active_tools
                .values()
                .next_back()
                .map(|t| CurrentToolSnapshot {
                    name: t.name.clone(),
                    call_id: t.call_id.clone(),
                    started_at: t.started_at.clone(),
                    elapsed_seconds: now.duration_since(t.started).as_secs(),
                }),
            tools: ToolSummary {
                used: self.used.iter().cloned().collect(),
                started: self.started,
                completed: self.completed,
                failed: self.failed,
            },
            progress: ProgressSummary {
                state: state.into(),
                reason,
                window_seconds: 120,
                last_progress_seconds_ago: progress_ago,
                tool_calls_completed: recent_completed,
                tool_calls_failed: recent_failed,
            },
        }
    }
}
