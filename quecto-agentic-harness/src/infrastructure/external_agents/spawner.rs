//! The member session's spawner (#2304): detached work runs as a task of
//! the tokio runtime the session is driven on.

use crate::application::external_agent::ports::{DetachedWork, ExternalAgentSpawner};

/// Spawns onto the current tokio runtime; the session only ever calls it
/// from a task of that runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokioExternalAgentSpawner;

impl ExternalAgentSpawner for TokioExternalAgentSpawner {
    fn spawn(&self, work: DetachedWork) {
        // Detached: the handle is dropped and the task runs to its end.
        drop(tokio::spawn(work));
    }
}
