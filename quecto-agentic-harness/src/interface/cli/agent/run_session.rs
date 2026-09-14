//! The one-shot `agent -m` session run: open (claim + load) the named
//! session through the composed store, run the prompt, persist the result.
//! The store comes from composition's sessions builder (#1970); this module
//! never constructs one.
use super::{AgentFlags, AgentOutput, DeadlineResult, run_with_deadline};
use crate::application::agent_loop::AgentLoopImpl;
use crate::application::agent_turn::ports::AgentLoop;
use crate::domain::message::Message;
use crate::domain::session::Session;
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
        let name = flags.session_name.as_deref().unwrap_or("default");
        SessionIdentity::from_persisted_key(Session::build_key("cli", name))
    };
    let sessions = sessions(SessionLoopInputs {
        base_dir: base_dir.to_path_buf(),
        store: None,
    });
    let session_store = sessions.store;
    let rt = match crate::interface::cli::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("failed to create runtime: {}\n", e));
            return 1;
        }
    };

    let mut messages: Vec<Message> = if !ephemeral {
        // Refuse at open, not at first save (#1460): a key owned by another
        // live process must fail before any turn runs against it.
        if let Err(e) = session_store.claim(&session_key) {
            out.stderr.push_str(&format!("{}\n", e));
            return 1;
        }
        match rt.block_on(session_store.load(&session_key)) {
            Ok(Some(session)) => session.messages,
            Ok(None) => Vec::new(),
            Err(e) => {
                out.stderr
                    .push_str(&format!("failed to load session: {}\n", e));
                return 1;
            }
        }
    } else {
        Vec::new()
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
                let session = Session {
                    key: session_key,
                    messages: std::mem::take(&mut messages),
                    workflow_run: None,
                    subagent_roster: Vec::new(),
                };
                if let Err(e) = rt.block_on(session_store.save(&session)) {
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
