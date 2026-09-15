//! The one-shot `agent -m` session run: open (claim + load) the named
//! session through the composed store, run the prompt, persist the result.
//! The store comes from composition's sessions builder (#1970); this module
//! never constructs one.
use super::{AgentFlags, AgentOutput, DeadlineResult, run_with_deadline};
use crate::application::agent_loop::AgentLoopImpl;
use crate::application::agent_turn::ports::AgentLoop;
use crate::application::sessions::dto::SaveTrigger;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;

use crate::interface::cli::uds_session_handles::SessionLoopInputs;
use crate::interface::shared::scrub_ephemeral_spill;

pub(crate) fn run_agent_session(
    base_dir: &std::path::Path,
    sessions: crate::interface::cli::SessionHandlesBuilder,
    mut agent: AgentLoopImpl,
    flags: &AgentFlags,
    out: &mut AgentOutput<'_>,
) -> i32 {
    let ephemeral = flags.no_session || flags.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        SessionIdentity::ephemeral()
    } else {
        // The key grammar is the domain's: `--session <name>` was admitted
        // by the same allowlist at flag parse, so a refusal here is defensive.
        let name = flags.session_name.as_deref().unwrap_or("default");
        match SessionIdentity::named_cli(name) {
            Ok(identity) => identity,
            Err(e) => {
                out.stderr.push_str(&format!("{e}\n"));
                return 1;
            }
        }
    };
    let sessions = sessions(SessionLoopInputs {
        base_dir: base_dir.to_path_buf(),
        store: None,
        session_key: session_key.runtime_key().to_string(),
        ephemeral,
        // The one-shot run appends its prompt by id (below) rather than at
        // the head, so the save transaction has no injected head to strip.
        system_prompt: String::new(),
        spill_store: agent.spill_store().cloned(),
        durable_prefix: agent.durable_prefix_latch(),
        workflow_state: None,
        subagent_registry: None,
    });
    let rt = match crate::interface::cli::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("failed to create runtime: {}\n", e));
            return 1;
        }
    };

    // Open the session (#1863, D8 #1977): the transaction claims it (a key
    // owned by another live process is refused at open, #1460) and loads
    // it; an ephemeral run touches the store not at all.
    let mut messages: Vec<Message> = match rt.block_on(sessions.switch.resume.open_at_startup()) {
        Ok(opened) => opened.messages,
        Err(e) => {
            out.stderr.push_str(&format!("{e}\n"));
            return 1;
        }
    };

    if !ephemeral && !messages.is_empty() {
        rt.block_on(agent.prune_resumed_context(&mut messages));
    }

    // System prompt is injected at call time, never persisted. Track its
    // message ID, not its position: mid-run pruning can shift indices left,
    // making a positional `remove(idx)` delete the wrong message (#1073).
    let system_prompt_id = flags.system_prompt.as_deref().map(|sp| {
        let msg = Message::system(sp.to_string());
        let id = msg.id();
        messages.push(msg);
        id
    });

    let message = flags.message.as_deref().unwrap_or("");
    messages.push(Message::user(message.to_string()));

    let agent_result = if let Some(secs) = flags.max_time {
        match run_with_deadline(&rt, &mut agent, &mut messages, secs) {
            DeadlineResult::Completed(inner) => inner,
            DeadlineResult::TimedOut => {
                out.stderr.push_str("max-time exceeded\n");
                scrub_ephemeral_spill(base_dir, ephemeral);
                return 2;
            }
        }
    } else {
        rt.block_on(agent.process(&mut messages))
    };
    // Nothing an ephemeral run spilled for in-run recall may outlive the run.
    scrub_ephemeral_spill(base_dir, ephemeral);

    match agent_result {
        Ok(result) => {
            if !ephemeral {
                // Identity-based removal: immune to index shifts from
                // mid-run pruning (a no-op if pruning dropped it).
                if let Some(id) = system_prompt_id
                    && let Some(idx) = messages.iter().position(|m| m.id() == id)
                {
                    messages.remove(idx);
                }
                if let Err(e) = rt.block_on(
                    sessions
                        .save_session
                        .save(&mut messages, SaveTrigger::OrdinaryExit),
                ) {
                    out.stderr
                        .push_str(&format!("warning: failed to save session: {}\n", e));
                }
            }
            out.stdout.push_str(&result.response);
            out.stdout.push('\n');
            0
        }
        Err(e) => {
            out.stderr.push_str(&format!("Error: {}\n", e));
            1
        }
    }
}
