use super::*;
use crate::infrastructure::tools::subagent_lifecycle::{
    SubagentLifecycleEvent, apply_lifecycle_event_to_entry,
};
use crate::infrastructure::tools::subagent_registry::{
    ExitSignalRx, SubagentStatus, mark_completion_consumed_by_await, resolve_registry_key,
};

fn await_tool_result(
    status: &str,
    reason: Option<&str>,
    agent_id: String,
    elapsed_ms: u64,
    workflow: Option<WorkflowSnapshot>,
) -> ToolResult {
    await_tool_result_with_error(status, reason, agent_id, elapsed_ms, workflow, None)
}

/// Serialize an [`AwaitResult`] as a (non-error) `ToolResult`, optionally
/// carrying the actual run-level error cause so it is visible in the `await`
/// response (#752). [`await_tool_result`] delegates here with `error: None`.
fn await_tool_result_with_error(
    status: &str,
    reason: Option<&str>,
    agent_id: String,
    elapsed_ms: u64,
    workflow: Option<WorkflowSnapshot>,
    error: Option<&str>,
) -> ToolResult {
    let result = AwaitResult::with_error(status, reason, agent_id, elapsed_ms, workflow, error);
    ToolResult {
        content: serde_json::to_string(&result).unwrap(),
        is_error: false,
        image_blocks: vec![],
    }
}

fn elapsed_ms(start: tokio::time::Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

/// Render an exit signal as the await `reason` string. Single definition
/// shared by the main await loop and the dead-socket classifier.
fn exit_signal_reason(
    signal: &crate::infrastructure::tools::subagent_registry::ExitSignal,
) -> String {
    use crate::infrastructure::tools::subagent_registry::ExitSignalKind;
    if let Some(code) = signal.exit_code {
        format!("exit_code_{code}")
    } else if let Some(sig) = signal.signal {
        format!("signal_{sig}")
    } else {
        // No status exists: report HOW the death was observed instead of
        // fabricating a clean exit for a child that may have been killed
        // behind quecto's back (#1369 slice 3).
        match signal.kind {
            ExitSignalKind::ConnectionClosed => "connection_closed".to_string(),
            ExitSignalKind::NeverReachable => "never_reachable".to_string(),
            ExitSignalKind::ProcessExit => "exit_status_unknown".to_string(),
        }
    }
}

/// Output of [`AgentCmdTool::prepare_await_session`]: the resolved durable
/// key, exit-signal subscription, active-await guard, and whether the child's
/// socket accepted a probe connection (a dead socket is classified by
/// `classify_dead_socket` rather than failing outright, #1369 slice 3).
struct PreparedAwait {
    registry_key: String,
    exit_signal_rx: Option<ExitSignalRx>,
    guard: AwaitGuard,
    connectable: bool,
}

impl AgentCmdTool {
    pub(super) async fn execute_await(&self, arguments: &str) -> Result<ToolResult, DomainError> {
        // LLM-addressable: malformed JSON and missing fields → Ok(is_error=true)
        // so the LLM can see the message and retry. Tool contract.
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!(
                        "invalid JSON arguments: {e}. Example: {{\"agent_id\": \"my-agent\"}}"
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }
        };

        let agent_id = match args.get("agent_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Ok(ToolResult {
                    content: "missing required field: agent_id".to_string(),
                    is_error: true,
                    image_blocks: vec![],
                });
            }
        };

        if let Err(e) = validate_agent_id_format(&agent_id) {
            return Ok(ToolResult {
                content: format!("agent_cmd error: {e}"),
                is_error: true,
                image_blocks: vec![],
            });
        }

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(AWAIT_DEFAULT_TIMEOUT)
            .min(AWAIT_MAX_TIMEOUT);

        let idle_timeout_secs = args
            .get("idle_timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(AWAIT_DEFAULT_IDLE_TIMEOUT);

        let start = tokio::time::Instant::now();

        // Resolve display→UUID, arm active-await, and validate the socket (#1378).
        let PreparedAwait {
            registry_key,
            exit_signal_rx,
            guard: _guard,
            connectable,
        } = match self.prepare_await_session(&agent_id) {
            Ok(prepared) => prepared,
            Err(result) => return Ok(result),
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        if !connectable {
            if let Err(result) = self
                .classify_dead_socket(
                    &registry_key,
                    &agent_id,
                    start,
                    exit_signal_rx.as_ref(),
                    deadline,
                )
                .await
            {
                return Ok(result);
            }
        }

        // Main await loop: poll status + listen for exit signals.
        // Uses `tokio::select!` to wake instantly on exit signals while
        // polling registry status at a slower interval.
        let mut idle_since: Option<tokio::time::Instant> = None;
        let mut poll_interval =
            tokio::time::interval(Duration::from_millis(AWAIT_POLL_INTERVAL_MS));
        poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let mut exit_rx = exit_signal_rx;

        loop {
            // Check if we've exceeded the overall timeout.
            if tokio::time::Instant::now() >= deadline {
                {
                    let mut entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                    // Registry is UUID-keyed; never get_mut the raw display label (#1378).
                    if let Some(entry) = entries.get_mut(&registry_key) {
                        entry.status = apply_lifecycle_event_to_entry(
                            entry,
                            SubagentLifecycleEvent::AwaitTimedOut,
                        );
                    }
                }
                let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                return Ok(await_tool_result(
                    "timeout",
                    None,
                    agent_id.clone(),
                    elapsed_ms(start),
                    workflow,
                ));
            }

            // Wait for either an exit signal (instant wakeup) or the next
            // poll tick (for status/idle checks). This avoids burning CPU
            // and reduces exit-detection latency from 500ms to near-zero.
            let got_exit_signal = if let Some(ref mut rx) = exit_rx {
                tokio::select! {
                    result = rx.changed() => result.is_ok(),
                    _ = poll_interval.tick() => false,
                }
            } else {
                poll_interval.tick().await;
                false
            };

            // Handle exit signal (instant path).
            if got_exit_signal {
                if let Some(ref mut rx) = exit_rx {
                    let signal = rx.borrow_and_update().clone();
                    if let Some(exit_signal) = signal {
                        let reason = Some(exit_signal_reason(&exit_signal));
                        mark_completion_consumed_by_await(&self.registry, &registry_key);
                        return Ok(await_tool_result(
                            "exited",
                            reason.as_deref(),
                            agent_id.clone(),
                            elapsed_ms(start),
                            None,
                        ));
                    }
                }
            }

            // Poll the registry for current status (UUID key).
            let current_status = {
                let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                entries
                    .get(&registry_key)
                    .map(|e| (e.status.clone(), e.run_error.clone()))
            };

            match current_status {
                None | Some((SubagentStatus::Exited, _)) => {
                    // Agent removed from registry or marked Exited. Read the
                    // exit signal from the registry entry for the actual exit
                    // code/signal/observation kind; only an entry that never
                    // published a signal at all falls back to exit_code_0
                    // (the pre-signal legacy contract for locally reaped
                    // children).
                    let reason = {
                        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                        entries
                            .get(&registry_key)
                            .and_then(|e| e.exit_signal_tx.as_ref())
                            .and_then(|tx| {
                                let rx = tx.subscribe();
                                rx.borrow().clone()
                            })
                            .map(|es| exit_signal_reason(&es))
                            .or(Some("exit_code_0".into()))
                    };
                    mark_completion_consumed_by_await(&self.registry, &registry_key);
                    return Ok(await_tool_result(
                        "exited",
                        reason.as_deref(),
                        agent_id.clone(),
                        elapsed_ms(start),
                        None,
                    ));
                }
                Some((SubagentStatus::Idle, _)) => {
                    // Agent is idle — start or continue the idle_timeout countdown.
                    let now = tokio::time::Instant::now();
                    match idle_since {
                        None => {
                            idle_since = Some(now);
                            if idle_timeout_secs == 0 {
                                let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                                mark_completion_consumed_by_await(&self.registry, &registry_key);
                                return Ok(await_tool_result(
                                    "idle",
                                    Some("idle"),
                                    agent_id.clone(),
                                    elapsed_ms(start),
                                    workflow,
                                ));
                            }
                        }
                        Some(since) => {
                            if now.duration_since(since) >= Duration::from_secs(idle_timeout_secs) {
                                let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                                mark_completion_consumed_by_await(&self.registry, &registry_key);
                                return Ok(await_tool_result(
                                    "idle",
                                    Some("idle"),
                                    agent_id.clone(),
                                    elapsed_ms(start),
                                    workflow,
                                ));
                            }
                        }
                    }
                }
                Some((SubagentStatus::Running, _)) | Some((SubagentStatus::Starting, _)) => {
                    // Agent is actively working — reset idle countdown.
                    idle_since = None;
                }
                Some((SubagentStatus::Error, Some(run_error))) => {
                    // The prompt run failed (for example a provider/model error).
                    // Return a structured error after the idle window so parents
                    // can triage instead of waiting for the process to exit. The
                    // actual cause is surfaced so parents can triage without
                    // reading the child's logs (#752).
                    let now = tokio::time::Instant::now();
                    if idle_since.is_none() {
                        idle_since = Some(now);
                    }
                    if let Some(since) = idle_since {
                        let elapsed_idle = now.duration_since(since);
                        if idle_timeout_secs == 0
                            || elapsed_idle >= Duration::from_secs(idle_timeout_secs)
                        {
                            let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                            mark_completion_consumed_by_await(&self.registry, &registry_key);
                            return Ok(await_tool_result_with_error(
                                "error",
                                Some("agent_error"),
                                agent_id.clone(),
                                elapsed_ms(start),
                                workflow,
                                Some(&run_error),
                            ));
                        }
                    }
                }
                Some((SubagentStatus::Error, None)) => {
                    // Recoverable tool-call error. Preserve existing behavior:
                    // wait for the child to either continue running, become idle,
                    // exit, or hit the await timeout.
                    let now = tokio::time::Instant::now();
                    if idle_since.is_none() {
                        idle_since = Some(now);
                    }
                    if let Some(since) = idle_since {
                        let elapsed_idle = now.duration_since(since);
                        if idle_timeout_secs == 0
                            || elapsed_idle >= Duration::from_secs(idle_timeout_secs)
                        {
                            let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                            mark_completion_consumed_by_await(&self.registry, &registry_key);
                            return Ok(await_tool_result(
                                "idle",
                                Some("idle"),
                                agent_id.clone(),
                                elapsed_ms(start),
                                workflow,
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Resolve display→UUID, arm the active-await set, and validate the child's
    /// socket is connectable. On failure returns a ready-made tool result so
    /// `execute_await` stays under the line budget (#1378).
    fn prepare_await_session(&self, agent_id: &str) -> Result<PreparedAwait, ToolResult> {
        // Resolve live display label → UUID registry key before any await /
        // dedupe bookkeeping. User-facing `agent_id` stays the display label.
        let registry_key = {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            match resolve_registry_key(&entries, agent_id) {
                Ok(key) => key,
                Err(_) => {
                    return Err(await_tool_result(
                        "error",
                        Some("agent_not_found"),
                        agent_id.to_string(),
                        0,
                        None,
                    ));
                }
            }
        };

        let (socket_path, exit_signal_rx) = {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            match entries.get(&registry_key) {
                Some(entry) => {
                    let rx = entry.exit_signal_tx.as_ref().map(|tx| tx.subscribe());
                    (entry.socket_path.clone(), rx)
                }
                None => {
                    return Err(await_tool_result(
                        "error",
                        Some("agent_not_found"),
                        agent_id.to_string(),
                        0,
                        None,
                    ));
                }
            }
        };

        // Duplicate awaiters are keyed by durable UUID identity.
        {
            let mut active = self.active_awaits.lock().unwrap_or_else(|e| e.into_inner());
            if active.contains(&registry_key) {
                return Err(await_tool_result(
                    "error",
                    Some("another_await_active"),
                    agent_id.to_string(),
                    0,
                    None,
                ));
            }
            active.insert(registry_key.clone());
        }

        let guard = AwaitGuard {
            active_awaits: self.active_awaits.clone(),
            agent_id: registry_key.clone(),
        };

        // Synchronous non-blocking connect to detect stale sockets early.
        let connectable = if socket_path.exists() {
            std::os::unix::net::UnixStream::connect(&socket_path).is_ok()
        } else {
            false
        };
        Ok(PreparedAwait {
            registry_key,
            exit_signal_rx,
            guard,
            connectable,
        })
    }

    /// Classify a dead-socket await (#1369 slice 3): a registered child whose
    /// socket is gone is a PUSHED death in flight, not a hard error — its
    /// monitor observed (or is about to observe) EOF and will mark the entry
    /// exited. An already-delivered exit signal (read from the in-hand watch
    /// channel's current value — `subscribe` marks it seen, so `changed()`
    /// alone would miss it) short-circuits to `exited` immediately. The
    /// signal itself fires only after the post-mortem inspect completes, and
    /// the inspect subprocess is bounded (`INSPECT_TIMEOUT`), so the grace
    /// window here is sized to outlast it: an awaiter arriving mid-inspect
    /// still classifies as `exited` rather than `connection_failed` —
    /// unless the caller's own await deadline (which always wins) elapses
    /// first. A child that stays non-exited on a dead socket keeps the
    /// fail-fast `connection_failed` contract.
    async fn classify_dead_socket(
        &self,
        registry_key: &str,
        agent_id: &str,
        start: tokio::time::Instant,
        exit_rx: Option<&ExitSignalRx>,
        deadline: tokio::time::Instant,
    ) -> Result<(), ToolResult> {
        // Must exceed subagent_cleanup::INSPECT_TIMEOUT so a hung-then-killed
        // inspect cannot make a dead-socket await misreport connection_failed.
        // Without an exit-signal channel no pushed death can ever arrive, so
        // waiting would only delay the honest connection_failed answer.
        let grace = if exit_rx.is_some() {
            (tokio::time::Instant::now() + Duration::from_secs(6)).min(deadline)
        } else {
            tokio::time::Instant::now()
        };
        loop {
            // A pushed death that already fired IS the answer — never
            // misclassify it as connection_failed behind a grace timer.
            if let Some(rx) = exit_rx {
                let signal = rx.borrow().clone();
                if let Some(exit_signal) = signal {
                    mark_completion_consumed_by_await(&self.registry, registry_key);
                    return Err(await_tool_result(
                        "exited",
                        Some(&exit_signal_reason(&exit_signal)),
                        agent_id.to_string(),
                        elapsed_ms(start),
                        None,
                    ));
                }
            }
            let status = {
                let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                entries.get(registry_key).map(|entry| entry.status.clone())
            };
            match status {
                None => {
                    return Err(await_tool_result(
                        "error",
                        Some("agent_not_found"),
                        agent_id.to_string(),
                        elapsed_ms(start),
                        None,
                    ));
                }
                Some(SubagentStatus::Exited) => return Ok(()),
                Some(_) if tokio::time::Instant::now() >= grace => {
                    return Err(await_tool_result(
                        "error",
                        Some("connection_failed"),
                        agent_id.to_string(),
                        elapsed_ms(start),
                        None,
                    ));
                }
                Some(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }

    /// Fetch workflow state from a subagent via UDS `get_state` command.
    /// Returns `None` if the fetch fails or workflow is not enabled.
    /// Uses a short timeout (2s) to avoid blocking if the agent is unresponsive.
    async fn fetch_workflow_snapshot(&self, agent_id: &str) -> Option<WorkflowSnapshot> {
        let socket_path = self.lookup_socket(agent_id).ok()?;
        let cmd = serde_json::json!({"type": "get_state"}).to_string();
        let response =
            tokio::time::timeout(Duration::from_secs(2), send_uds_command(&socket_path, &cmd))
                .await
                .ok()?
                .ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&response).ok()?;
        let data = parsed.get("data")?;

        // Look for workflow state in the response.
        let workflow = data.get("workflow").or_else(|| data.get("workflowState"))?;
        let mode = workflow
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        // `get_state` serializes the workflow snapshot with a nested
        // `progress: { done, total }` (see domain `WorkflowSnapshot`).
        let progress = workflow.get("progress");
        let steps_completed = progress
            .and_then(|p| p.get("done"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let steps_total = progress
            .and_then(|p| p.get("total"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        Some(WorkflowSnapshot {
            mode,
            steps_completed,
            steps_total,
        })
    }
}

#[cfg(test)]
#[path = "agent_cmd_await_cov_tests.rs"]
mod cov_tests;
