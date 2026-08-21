/// Decide whether an id-less connect-time SNAPSHOT response can legitimately
/// answer the command we sent (#828/#835/#842).
///
/// A busy/mid-turn child cannot answer promptly, so it pushes an unsolicited
/// snapshot (`id: None`) on connect. We accept that snapshot for `get_messages`
/// (including a COUNTED tail — the parent applies `count` locally afterwards via
/// [`finalize_snapshot_answer`], #842) and `get_state`, but NEVER for a command
/// whose snapshot would be the wrong answer:
/// - an `id`-carrying response is a genuine correlated reply, not the snapshot;
/// - an `agent_id` targets a DIFFERENT (nested) agent, so this child's snapshot
///   is not a valid answer (preserves the #835 guarantee);
/// - the response `command` must match the request `type`, with the expected
///   shape, so a `get_messages` snapshot never answers a `get_state`/stats call.
pub(super) fn response_is_valid_answer(json: &serde_json::Value, command: &str) -> bool {
    if json.get("id").is_some() {
        return false;
    }
    let Ok(cmd) = serde_json::from_str::<serde_json::Value>(command) else {
        return false;
    };
    let cmd_type = cmd.get("type").and_then(|v| v.as_str());
    // A targeted (nested) request is never answered by this child's own snapshot.
    if cmd.get("agent_id").is_some() {
        return false;
    }
    match cmd_type {
        Some("get_messages") => {
            // `count` is permitted (#842): the snapshot carries the full tail and
            // the parent slices the last-N locally in `finalize_snapshot_answer`.
            // A `before` cursor is NOT (#1061): the snapshot is always the newest
            // page, so accepting it would echo the caller's own cursor back as
            // `data.before` and the paging loop would spin without advancing —
            // the silent restart paging forbids. Rejecting it lets the request
            // wait for the real (correlated) reply or time out loudly.
            cmd.get("before").is_none()
                && json.get("command").and_then(|v| v.as_str()) == Some("get_messages")
                && json
                    .pointer("/data/messages")
                    .and_then(|v| v.as_array())
                    .is_some()
        }
        Some("get_state") => {
            // A `count` on get_state is meaningless; keep it strict so an unusual
            // command shape can never be silently answered by the snapshot. #1512
            // slim busy snapshots are intentionally id-less and unmarked: accept
            // only the slim projection (or unchanged cursor response), never the
            // legacy bulky snapshot marker/message-count shape. A request carrying
            // `since` can use an id-less snapshot only at exactly that generation,
            // where finalization turns it into the bounded unchanged marker; older
            // cursors wait for the correlated live reply so a connect-time
            // projection cannot hide a fresher generation.
            cmd.get("count").is_none()
                && json.get("command").and_then(|v| v.as_str()) == Some("get_state")
                && get_state_data_is_slim_snapshot(json.pointer("/data"))
                && get_state_snapshot_honors_since(json.pointer("/data"), cmd.get("since"))
        }
        Some("get_subagents") => {
            json.get("command").and_then(|v| v.as_str()) == Some("get_subagents")
                && json
                    .pointer("/data/subagents")
                    .and_then(|v| v.as_array())
                    .is_some()
        }
        Some("get_session_stats") => {
            json.get("command").and_then(|v| v.as_str()) == Some("get_session_stats")
                && json.pointer("/data/snapshot").and_then(|v| v.as_bool()) == Some(true)
                && json
                    .pointer("/data/userMessages")
                    .and_then(|v| v.as_u64())
                    .is_some()
        }
        Some("get_tool_catalogue") => {
            json.get("command").and_then(|v| v.as_str()) == Some("get_tool_catalogue")
                && json.pointer("/data/snapshot").and_then(|v| v.as_bool()) == Some(true)
                && json
                    .pointer("/data/tools")
                    .and_then(|v| v.as_array())
                    .is_some()
        }
        _ => false,
    }
}

fn get_state_snapshot_honors_since(
    data: Option<&serde_json::Value>,
    since: Option<&serde_json::Value>,
) -> bool {
    let Some(since) = since.and_then(|v| v.as_u64()) else {
        return true;
    };
    let Some(data) = data else {
        return false;
    };
    let generation = data.get("generation").and_then(|v| v.as_u64());
    if data.get("unchanged").and_then(|v| v.as_bool()) == Some(true) {
        return generation == Some(since);
    }
    // `get_state` is best-effort supervision: even a cursor request must not
    // wait behind a busy child's live correlated reply. Any slim changed
    // snapshot with a generation is therefore an acceptable immediate answer.
    generation.is_some()
}

fn get_state_data_is_slim_snapshot(data: Option<&serde_json::Value>) -> bool {
    let Some(data) = data else {
        return false;
    };
    let Some(object) = data.as_object() else {
        return false;
    };
    if data.get("unchanged").and_then(|v| v.as_bool()) == Some(true) {
        return data.get("generation").and_then(|v| v.as_u64()).is_some() && object.len() == 2;
    }
    let allowed_top_level = [
        "state",
        "effort",
        "model",
        "sessionKey",
        "progress",
        "generation",
        "workflow",
    ];
    if object
        .keys()
        .any(|key| !allowed_top_level.contains(&key.as_str()))
    {
        return false;
    }
    data.get("state").and_then(|v| v.as_str()).is_some()
        && object.contains_key("effort")
        && data.get("model").and_then(|v| v.as_str()).is_some()
        && data.get("generation").and_then(|v| v.as_u64()).is_some()
        && progress_is_slim(data.get("progress"))
        && data.get("workflow").is_none_or(workflow_is_slim)
}

fn progress_is_slim(progress: Option<&serde_json::Value>) -> bool {
    let Some(progress) = progress.and_then(|v| v.as_object()) else {
        return false;
    };
    progress.len() == 2
        && progress.get("state").and_then(|v| v.as_str()).is_some()
        && progress.get("reason").and_then(|v| v.as_str()).is_some()
}

fn workflow_is_slim(workflow: &serde_json::Value) -> bool {
    let Some(workflow) = workflow.as_object() else {
        return false;
    };
    let allowed_workflow = ["activeTemplate", "currentStep"];
    if workflow
        .keys()
        .any(|key| !allowed_workflow.contains(&key.as_str()))
    {
        return false;
    }
    let Some(active_template) = workflow.get("activeTemplate").and_then(|v| v.as_object()) else {
        return false;
    };
    if active_template.len() != 1 || active_template.get("id").and_then(|v| v.as_str()).is_none() {
        return false;
    }
    workflow.get("currentStep").is_none_or(current_step_is_slim)
}

fn current_step_is_slim(step: &serde_json::Value) -> bool {
    let Some(step) = step.as_object() else {
        return false;
    };
    let allowed_step = ["index", "key", "label", "phase", "done"];
    step.keys().all(|key| allowed_step.contains(&key.as_str()))
        && step.get("index").and_then(|v| v.as_u64()).is_some()
        && step.get("key").and_then(|v| v.as_str()).is_some()
        && step.get("label").and_then(|v| v.as_str()).is_some()
        && step.get("phase").and_then(|v| v.as_str()).is_some()
        && step.get("done").and_then(|v| v.as_bool()).is_some()
}

/// Apply the request's `count` (if any) to an accepted `get_messages` snapshot by
/// keeping the last-N messages, then return the single-line response (#842). When
/// the request carries no `count` (every `get_state` and uncounted `get_messages`
/// snapshot) the already-read `line` is returned VERBATIM — avoiding a needless
/// re-encode and preserving the child's exact bytes. The snapshot/trimmed markers
/// in `data` are preserved untouched so the caller can still tell the data may lag
/// the in-flight turn. Used only after [`response_is_valid_answer`] approved the
/// snapshot, so `json` is the already-parsed form of `line`.
#[cfg(test)]
#[path = "subagent_snapshot_tests.rs"]
mod tests;

pub(super) fn finalize_snapshot_answer(
    line: String,
    mut json: serde_json::Value,
    command: &str,
) -> String {
    let cmd = serde_json::from_str::<serde_json::Value>(command).ok();
    if cmd
        .as_ref()
        .and_then(|cmd| cmd.get("type"))
        .and_then(|v| v.as_str())
        == Some("get_state")
    {
        if let (Some(since), Some(generation)) = (
            cmd.as_ref()
                .and_then(|cmd| cmd.get("since"))
                .and_then(|v| v.as_u64()),
            json.pointer("/data/generation").and_then(|v| v.as_u64()),
        ) {
            if generation == since
                && json.pointer("/data/unchanged").and_then(|v| v.as_bool()) != Some(true)
            {
                if let Some(data) = json.pointer_mut("/data") {
                    *data = serde_json::json!({"unchanged": true, "generation": generation});
                }
                return json.to_string();
            }
        }
        return line;
    }
    let Some(count) = cmd
        .as_ref()
        .and_then(|cmd| cmd.get("count").and_then(|v| v.as_u64()))
    else {
        return line;
    };
    let mut dropped_prefix = false;
    if let Some(msgs) = json
        .pointer_mut("/data/messages")
        .and_then(|v| v.as_array_mut())
    {
        let count = count as usize;
        if msgs.len() > count {
            let skip = msgs.len() - count;
            msgs.drain(0..skip);
            dropped_prefix = true;
        }
    }
    // Slicing dropped older messages, so the snapshot's page metadata no longer
    // holds: `before` must name the oldest message still INCLUDED (the #1061
    // cursor contract) and older history now definitely exists. Without this a
    // caller following the stale cursor would skip the dropped span (#1061
    // review follow-up).
    if dropped_prefix {
        let first_id = json
            .pointer("/data/messages/0/id")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        if let Some(data) = json.pointer_mut("/data").and_then(|v| v.as_object_mut()) {
            match first_id {
                Some(id) => {
                    data.insert("before".into(), serde_json::Value::String(id));
                    data.insert("hasMoreBefore".into(), serde_json::Value::Bool(true));
                }
                // A slice to zero messages retains no cursor anchor; flag the
                // loss instead of advertising unreachable paging.
                None => {
                    data.insert("before".into(), serde_json::Value::Null);
                    data.insert("hasMoreBefore".into(), serde_json::Value::Bool(false));
                    data.insert("trimmed".into(), serde_json::Value::Bool(true));
                }
            }
        }
    }
    json.to_string()
}
