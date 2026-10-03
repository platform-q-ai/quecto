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
//!   1. reserves dispatch capacity for queueable work; rejected admission does
//!      not interrupt the active turn;
//!   2. emits an IMMEDIATE, id-correlated acceptance ack to THAT client, so the
//!      parent returns on ACCEPTANCE rather than on the child's turn completion
//!      (preserving #835 id-correlation); and
//!   3. forwards the work to the dispatch loop transformed so a busy child
//!      QUEUES it for the next turn (`prompt`/`follow_up` → `follow_up`) while
//!      `steer`/`abort` keep interrupting via the cancel side-channel that the
//!      reader fires after admission (abort needs no queue slot).
//!
//! Completion still surfaces later via the passive completion-note path (#816). The marker gates this
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
    /// Whether the forwarded work is a steer, which interrupts once admitted.
    pub(super) is_steer: bool,
    /// The images admitted here for the forwarded message (#2422), so
    /// dispatch runs them without decoding them again.
    pub(super) admitted: Option<super::uds::AdmittedImages>,
    /// The control this reader refused (#2422), for its `Rejected` receipt.
    pub(super) refused: Option<RefusedControl>,
}

/// A forwarded control refused before dispatch: its id and command name.
pub(crate) struct RefusedControl {
    pub(super) id: String,
    pub(super) command: String,
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
    if !line.contains("accept") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;
    if obj.get("ack").and_then(|v| v.as_str()) != Some("accept") {
        return None;
    }
    let raw_cmd_type = obj.get("type").and_then(|v| v.as_str())?;
    // Do not normalize malformed message/id fields into an accepted command.
    // The dispatch parser and eager-cancel classifier must agree on validity.
    let carries_message = matches!(raw_cmd_type, "prompt" | "steer" | "follow_up");
    let is_prompt_steer = raw_cmd_type == "prompt"
        && obj.get("streamingBehavior").and_then(|v| v.as_str()) == Some("steer");
    let cmd_type = if is_prompt_steer {
        "steer"
    } else {
        raw_cmd_type
    };
    let admitted = match carries_message {
        true => {
            let command = serde_json::from_str::<super::protocol::AgentCommand>(line).ok()?;
            match admit_forwarded_images(command, cmd_type) {
                Ok(images) => Some(images),
                Err(refused) => return Some(*refused),
            }
        }
        false => None,
    };
    // Echo the parent's stamped correlation id on the ack so its reader matches
    // this reply and never rides the timeout (#835). A forward with no id falls
    // back to a None id (first-response correlation on the parent).
    let id = obj.get("id").and_then(|v| v.as_str());
    let message = obj.get("message").and_then(|v| v.as_str());

    let forward_line = match raw_cmd_type {
        // A fresh prompt or an explicit follow_up both become a queued follow-up:
        // an idle child runs it immediately, a busy child enqueues it for the
        // next turn — never the "agent is running; provide streamingBehavior"
        // rejection a raw `prompt` would hit on a busy child.
        "prompt" if is_prompt_steer => Some(
            serde_json::json!({ "type": "prompt", "message": message?, "streamingBehavior": "steer" })
                .to_string(),
        ),
        "prompt" | "follow_up" => {
            Some(serde_json::json!({ "type": "follow_up", "message": message? }).to_string())
        }
        // Steering interrupts only after the reader reserves queue capacity.
        // Explicit abort uses its independent cancellation side-channel.
        "steer" => Some(serde_json::json!({ "type": "prompt", "message": message?, "streamingBehavior": "steer" }).to_string()),
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

    let forward_line = forward_line.map(|line| {
        if carries_message {
            let mut forwarded: serde_json::Value =
                serde_json::from_str(&line).expect("built control JSON");
            if let Some(id) = id {
                forwarded["id"] = serde_json::json!(id);
            }
            // The images travel with their message (#2422), admitted above.
            if let Some(images) = obj.get("images") {
                forwarded["images"] = images.clone();
            }
            forwarded.to_string()
        } else {
            line
        }
    });
    let ack_line = {
        let mut l = AgentEvent::ok(id, cmd_type, Some(serde_json::json!({"status":"accepted"})))
            .to_json_line();
        l.push('\n');
        l
    };

    Some(AcceptedControl {
        ack_line,
        forward_line,
        is_steer: cmd_type == "steer",
        admitted,
        refused: None,
    })
}

/// Admit a forwarded message's images (#2422), or refuse it at once with
/// the exact message dispatch would give: it is never forwarded, so a
/// refused steer never cancels the running turn, and dispatch records its
/// `Rejected` receipt.
fn admit_forwarded_images(
    command: super::protocol::AgentCommand,
    cmd_type: &str,
) -> Result<super::uds::AdmittedImages, Box<AcceptedControl>> {
    use super::protocol::AgentCommand;
    let id = command.id().map(str::to_owned);
    let images = match command {
        AgentCommand::Prompt { images, .. }
        | AgentCommand::Steer { images, .. }
        | AgentCommand::FollowUp { images, .. } => images,
        // A command that carries no images admits none.
        _ => Vec::new(),
    };
    super::uds::admit_images(images).map_err(|refusal| {
        let mut ack_line =
            AgentEvent::err(id.as_deref(), cmd_type, refusal.to_string()).to_json_line();
        ack_line.push('\n');
        Box::new(AcceptedControl {
            ack_line,
            forward_line: None,
            is_steer: false,
            admitted: None,
            refused: id.filter(|_| false).map(|id| RefusedControl {
                id,
                command: cmd_type.to_owned(),
            }), // red (#2422 review round 1): no receipt
        })
    })
}

#[cfg(test)]
#[path = "uds_control_forward_tests.rs"]
mod tests;
