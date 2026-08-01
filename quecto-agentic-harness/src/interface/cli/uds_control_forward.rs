//! Non-blocking forward of parent→child control commands (#876).
//!
//! When a parent agent's `agent_cmd` forwards a queueable command to a BUSY
//! child, the child's single dispatch loop is held by the
//! in-flight turn, so the command's `response` (the turn-completion ack) only
//! arrives once the turn ends — freezing the parent's own turn for the whole
//! child turn (even an idle child that accepted instantly, because `prompt`'s
//! response IS the completed turn result).
//!
//! The parent marks these forwards with `"ack":"accept"`. The child's
//! per-connection reader task — which runs independently of the (possibly
//! blocked) dispatch loop — recognises the marker and:
//!   1. emits an IMMEDIATE, id-correlated acceptance ack to THAT client, so the
//!      parent returns on ACCEPTANCE rather than on the child's turn completion
//!      (preserving #835 id-correlation); and
//!   2. forwards the work to the dispatch loop transformed so a busy child
//!      QUEUES it for the next turn (`prompt`/`follow_up` → `follow_up`) while
//!      `steer`/`abort` keep interrupting via the cancel side-channel that the
//!      reader already fires.
//!
//! Completion still surfaces later via the passive auto-await note (#816) or an
//! explicit `agent_cmd await` — those paths are untouched. The marker gates this
//! behaviour to the `agent_cmd` forward path only, so interactive TUI/CLI
//! clients (which never set it) see no protocol change.

use super::protocol::AgentEvent;

/// A flagged control command the reader accepted on the child's behalf.
pub(super) struct AcceptedControl {
    /// Immediate id-correlated acceptance `response` line (newline-terminated)
    /// written directly to the accepting client.
    pub(super) ack_line: String,
    /// The work to hand to the dispatch loop (newline NOT required; sent as an
    /// mpsc command line). `None` for `abort`, which only needs the cancel that
    /// the reader already fired — there is nothing to enqueue.
    pub(super) forward_line: Option<String>,
}

/// Inspect a raw client line: if it is an `agent_cmd` control forward (carries
/// `"ack":"accept"` and a supported `type`), return the acceptance ack to write
/// back plus the transformed work line to dispatch. Returns `None` for every
/// other line (normal prompts, queries, tool_results, …) so the caller forwards
/// it unchanged.
///
/// A cheap substring gate short-circuits the JSON parse for the overwhelmingly
/// common unflagged line before doing any allocation.
pub(super) fn intercept_control_forward(line: &str) -> Option<AcceptedControl> {
    if !line.contains(r#""accept""#) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;
    if obj.get("ack").and_then(|v| v.as_str()) != Some("accept") {
        return None;
    }
    let cmd_type = obj.get("type").and_then(|v| v.as_str())?;
    // Echo the parent's stamped correlation id on the ack so its reader matches
    // this reply and never rides the timeout (#835). A forward with no id falls
    // back to a None id (first-response correlation on the parent).
    let id = obj.get("id").and_then(|v| v.as_str());
    let message = obj.get("message").and_then(|v| v.as_str());

    let forward_line = match cmd_type {
        // A fresh prompt or an explicit follow_up both become a queued follow-up:
        // an idle child runs it immediately, a busy child enqueues it for the
        // next turn — never the "agent is running; provide streamingBehavior"
        // rejection a raw `prompt` would hit on a busy child.
        "prompt" | "follow_up" => {
            Some(serde_json::json!({ "type": "follow_up", "message": message? }).to_string())
        }
        // steer/abort interrupt via the cancel side-channel the reader already
        // fired; steer also re-queues its message ahead of the line.
        "steer" => Some(serde_json::json!({ "type": "steer", "message": message? }).to_string()),
        "abort" => None,
        "set_model" | "clear_history" => {
            let mut forwarded = obj.clone();
            forwarded.remove("ack");
            forwarded.remove("id");
            Some(serde_json::Value::Object(forwarded).to_string())
        }
        // Not a command we fast-ack — let it dispatch normally.
        _ => return None,
    };

    let ack_line = {
        let mut l = AgentEvent::ok(id, cmd_type, None).to_json_line();
        l.push('\n');
        l
    };

    Some(AcceptedControl {
        ack_line,
        forward_line,
    })
}

#[cfg(test)]
#[path = "uds_control_forward_tests.rs"]
mod tests;
