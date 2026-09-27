//! The one-shot `agent -m` session run: open (claim + load) the named
//! session through the composed store, run the prompt, persist the result.
//! The store comes from composition's sessions builder (#1970); this module
//! never constructs one.
use super::run_session_save::TranscriptSave;
use super::{AgentFlags, AgentOutput, DeadlineResult, run_with_deadline, settle_stopped_run};
use crate::application::agent_loop::AgentLoopImpl;
use crate::application::agent_turn::ports::AgentLoop;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;

use crate::interface::cli::uds_session_handles::SessionLoopInputs;

pub(crate) fn run_agent_session(
    base_dir: &std::path::Path,
    sessions: crate::interface::cli::SessionHandlesBuilder,
    mut agent: AgentLoopImpl,
    handles: crate::interface::cli::run_end_fleet::RunHandles<'_>,
    flags: &AgentFlags,
    out: &mut AgentOutput<'_>,
) -> i32 {
    let ephemeral = flags.no_session || flags.session_name.as_deref() == Some("-");
    let identity = if ephemeral {
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
        identity,
        ephemeral,
        // The one-shot run appends its prompt by id (below) rather than at
        // the head, so the save transaction has no injected head to strip.
        system_prompt: String::new(),
        spill_store: Some(handles.retention.store.clone()),
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
    // Dropped before the runtime on every return: rollbacks finish (#2173).
    let _exit = crate::interface::cli::launch_rollback_wait::WaitForLaunchRollbacks(&rt);

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
    let prompt = Message::user(message.to_string());
    let run_start = prompt.id();
    messages.push(prompt);

    let agent_result = if let Some(secs) = flags.max_time {
        match run_with_deadline(&rt, &mut agent, &mut messages, secs) {
            DeadlineResult::Completed(inner) => inner,
            DeadlineResult::TimedOut => {
                // Saved first: the settling below can take a minute, and a
                // process killed meanwhile must not lose it (#2173).
                let saving = TranscriptSave::new(&rt, &sessions, system_prompt_id, ephemeral);
                saving.stopped(&mut messages, run_start, secs, out);
                settle_stopped_run(&rt, &agent, secs);
                out.stderr.push_str("max-time exceeded\n");
                handles.retention.recall.scrub_ephemeral(ephemeral);
                return handles.run_end.after(&rt, out, 2);
            }
        }
    } else {
        rt.block_on(agent.process(&mut messages))
    };
    // Nothing an ephemeral run spilled for in-run recall may outlive the run.
    handles.retention.recall.scrub_ephemeral(ephemeral);

    match agent_result {
        Ok(result) => {
            TranscriptSave::new(&rt, &sessions, system_prompt_id, ephemeral)
                .save(&mut messages, out);
            out.stdout.push_str(&result.response);
            out.stdout.push('\n');
            handles.run_end.after(&rt, out, 0)
        }
        Err(e) => {
            out.stderr.push_str(&format!("Error: {}\n", e));
            1
        }
    }
}
