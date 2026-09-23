//! Agent Commander spike: attach the dry-run observer to a UDS agent when
//! `QUECTO_AGENT_COMMANDER=dry-run` (off otherwise).
use std::path::Path;
use std::sync::Arc;

use super::agent::AgentFlags;
use crate::application::agent_commander::ports::CommanderSink;
use crate::application::agent_loop::AgentLoopImpl;
use crate::infrastructure::agent_commander::{AgentRole, DryRunCommander};

pub(super) fn attach(
    agent: &mut AgentLoopImpl,
    base_dir: &Path,
    flags: &AgentFlags,
    stderr: &mut String,
) {
    let role = if flags.parent_id.is_some() || flags.parent_control.is_some() {
        AgentRole::Child {
            parent_id: flags.parent_id.clone(),
        }
    } else {
        AgentRole::Root
    };
    if let Some(commander) = DryRunCommander::from_env(base_dir, role) {
        agent.set_commander(Some(commander as Arc<dyn CommanderSink>));
        stderr.push_str("agent commander: dry run on (decisions logged, none acted on)\n");
    }
}
