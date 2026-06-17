use super::*;
use crate::infrastructure::tools::subagent_registry::SubagentStatus;

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

        let start = std::time::Instant::now();

        // Check if agent exists in registry.
        let (socket_path, exit_signal_rx) = {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            match entries.get(&agent_id) {
                Some(entry) => {
                    let rx = entry.exit_signal_tx.as_ref().map(|tx| tx.subscribe());
                    (entry.socket_path.clone(), rx)
                }
                None => {
                    let result = AwaitResult {
                        status: "error".into(),
                        reason: Some("agent_not_found".into()),
                        agent_id: agent_id.clone(),
                        elapsed_ms: 0,
                        workflow: None,
                    };
                    return Ok(ToolResult {
                        content: serde_json::to_string(&result).unwrap(),
                        is_error: false,
                        image_blocks: vec![],
                    });
                }
            }
        };

        // Check for duplicate awaiters.
        {
            let mut active = self.active_awaits.lock().unwrap_or_else(|e| e.into_inner());
            if active.contains(&agent_id) {
                let result = AwaitResult {
                    status: "error".into(),
                    reason: Some("another_await_active".into()),
                    agent_id: agent_id.clone(),
                    elapsed_ms: 0,
                    workflow: None,
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&result).unwrap(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }
            active.insert(agent_id.clone());
        }

        // Ensure we remove from active_awaits when done (RAII guard).
        let _guard = AwaitGuard {
            active_awaits: self.active_awaits.clone(),
            agent_id: agent_id.clone(),
        };

        // Check if socket is connectable (detect stale sockets early).
        // Use a synchronous non-blocking connect to avoid issues with tokio
        // single-threaded runtimes where async connect may not yield properly.
        let connectable = if socket_path.exists() {
            std::os::unix::net::UnixStream::connect(&socket_path).is_ok()
        } else {
            false
        };
        if !connectable {
            // Check if the entry is still in the registry (might have been
            // removed by the reaper between our lookup and here).
            let still_registered = {
                let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                entries.contains_key(&agent_id)
            };
            if still_registered {
                let result = AwaitResult {
                    status: "error".into(),
                    reason: Some("connection_failed".into()),
                    agent_id: agent_id.clone(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    workflow: None,
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&result).unwrap(),
                    is_error: false,
                    image_blocks: vec![],
                });
            } else {
                let result = AwaitResult {
                    status: "error".into(),
                    reason: Some("agent_not_found".into()),
                    agent_id: agent_id.clone(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    workflow: None,
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&result).unwrap(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }
        }

        // Main await loop: poll status + listen for exit signals.
        // Uses `tokio::select!` to wake instantly on exit signals while
        // polling registry status at a slower interval.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let mut idle_since: Option<tokio::time::Instant> = None;
        let mut poll_interval =
            tokio::time::interval(Duration::from_millis(AWAIT_POLL_INTERVAL_MS));
        poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let mut exit_rx = exit_signal_rx;

        loop {
            // Check if we've exceeded the overall timeout.
            if tokio::time::Instant::now() >= deadline {
                let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                let result = AwaitResult {
                    status: "timeout".into(),
                    reason: None,
                    agent_id: agent_id.clone(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    workflow,
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&result).unwrap(),
                    is_error: false,
                    image_blocks: vec![],
                });
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
                        let reason = if let Some(code) = exit_signal.exit_code {
                            Some(format!("exit_code_{code}"))
                        } else if let Some(sig) = exit_signal.signal {
                            Some(format!("signal_{sig}"))
                        } else {
                            Some("exit_code_0".to_string())
                        };
                        let result = AwaitResult {
                            status: "exited".into(),
                            reason,
                            agent_id: agent_id.clone(),
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            workflow: None,
                        };
                        return Ok(ToolResult {
                            content: serde_json::to_string(&result).unwrap(),
                            is_error: false,
                            image_blocks: vec![],
                        });
                    }
                }
            }

            // Poll the registry for current status.
            let current_status = {
                let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                entries.get(&agent_id).map(|e| e.status.clone())
            };

            match current_status {
                None | Some(SubagentStatus::Exited) => {
                    // Agent removed from registry or marked Exited. Read the
                    // exit signal from the registry entry for the actual exit
                    // code/signal; fall back to exit_code_0 if unavailable.
                    let reason = {
                        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                        entries
                            .get(&agent_id)
                            .and_then(|e| e.exit_signal_tx.as_ref())
                            .and_then(|tx| {
                                let rx = tx.subscribe();
                                rx.borrow().clone()
                            })
                            .map(|es| {
                                if let Some(code) = es.exit_code {
                                    format!("exit_code_{code}")
                                } else if let Some(sig) = es.signal {
                                    format!("signal_{sig}")
                                } else {
                                    "exit_code_0".into()
                                }
                            })
                            .or(Some("exit_code_0".into()))
                    };
                    let result = AwaitResult {
                        status: "exited".into(),
                        reason,
                        agent_id: agent_id.clone(),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        workflow: None,
                    };
                    return Ok(ToolResult {
                        content: serde_json::to_string(&result).unwrap(),
                        is_error: false,
                        image_blocks: vec![],
                    });
                }
                Some(SubagentStatus::Idle) => {
                    // Agent is idle — start or continue the idle_timeout countdown.
                    let now = tokio::time::Instant::now();
                    match idle_since {
                        None => {
                            idle_since = Some(now);
                            if idle_timeout_secs == 0 {
                                let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                                let result = AwaitResult {
                                    status: "idle".into(),
                                    reason: Some("completed".into()),
                                    agent_id: agent_id.clone(),
                                    elapsed_ms: start.elapsed().as_millis() as u64,
                                    workflow,
                                };
                                return Ok(ToolResult {
                                    content: serde_json::to_string(&result).unwrap(),
                                    is_error: false,
                                    image_blocks: vec![],
                                });
                            }
                        }
                        Some(since) => {
                            if now.duration_since(since) >= Duration::from_secs(idle_timeout_secs) {
                                let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                                let result = AwaitResult {
                                    status: "idle".into(),
                                    reason: Some("completed".into()),
                                    agent_id: agent_id.clone(),
                                    elapsed_ms: start.elapsed().as_millis() as u64,
                                    workflow,
                                };
                                return Ok(ToolResult {
                                    content: serde_json::to_string(&result).unwrap(),
                                    is_error: false,
                                    image_blocks: vec![],
                                });
                            }
                        }
                    }
                }
                Some(SubagentStatus::Running) | Some(SubagentStatus::Starting) => {
                    // Agent is actively working — reset idle countdown.
                    idle_since = None;
                }
                Some(SubagentStatus::Error) => {
                    // Agent's last tool returned an error. Treat like idle —
                    // if it stays in Error for the full idle_timeout, return
                    // so the caller can inspect and decide what to do.
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
                            let result = AwaitResult {
                                status: "idle".into(),
                                reason: Some("completed".into()),
                                agent_id: agent_id.clone(),
                                elapsed_ms: start.elapsed().as_millis() as u64,
                                workflow,
                            };
                            return Ok(ToolResult {
                                content: serde_json::to_string(&result).unwrap(),
                                is_error: false,
                                image_blocks: vec![],
                            });
                        }
                    }
                }
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
        let steps_completed = workflow
            .get("steps_completed")
            .or_else(|| workflow.get("stepsCompleted"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let steps_total = workflow
            .get("steps_total")
            .or_else(|| workflow.get("stepsTotal"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        Some(WorkflowSnapshot {
            mode,
            steps_completed,
            steps_total,
        })
    }
}
